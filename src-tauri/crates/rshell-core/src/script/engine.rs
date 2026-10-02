//! Rhai 脚本引擎
//!
//! 嵌入式脚本执行，提供 rshell API 给脚本环境。
//! 脚本可以连接/断开会话、发送命令、等待输出等。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use rhai::{Engine, EvalAltResult, Scope, AST};
use rshell_api::types::ScriptResult;
use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Host actions are injected by core; the Rhai engine never depends on Tauri.
pub trait ScriptHost: Send + Sync {
    fn send_text(&self, session_id: Uuid, text: &str) -> Result<(), CoreError>;
    fn list_sessions(&self) -> Result<Vec<Uuid>, CoreError>;
    fn execute_quick_command(&self, command_id: Uuid, session_id: Uuid) -> Result<(), CoreError>;
}

fn rhai_error(message: impl ToString) -> Box<EvalAltResult> {
    message.to_string().into()
}

/// 脚本超时统一文案:on_progress 终止标记、rshell_sleep 中止错误与
/// execute_string 的错误改写共用,保证上层 (dispatcher/IPC) 能识别超时。
const SCRIPT_TIMEOUT_MESSAGE: &str = "script execution timed out";

/// 默认单次脚本执行的挂钟时间上限
const DEFAULT_MAX_EXECUTION_TIME: Duration = Duration::from_secs(30);

/// rshell_sleep 的睡眠分片粒度:deadline 到点后最多再多睡一个片
const SLEEP_SLICE: Duration = Duration::from_millis(100);

thread_local! {
    /// 当前线程正在执行的脚本截止时刻;None 表示该线程当前没有受限执行。
    static EXEC_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
    /// 标记本次执行因超时被中止;execute_* 据此把内部中止错误改写为超时文案。
    static EXEC_TIMED_OUT: Cell<bool> = const { Cell::new(false) };
}

fn exec_deadline_exceeded() -> bool {
    EXEC_DEADLINE.with(|d| d.get().is_some_and(|deadline| Instant::now() >= deadline))
}

fn exec_timed_out() -> bool {
    EXEC_TIMED_OUT.with(Cell::get)
}

fn mark_exec_timed_out() {
    EXEC_TIMED_OUT.with(|t| t.set(true));
}

fn script_timeout_error() -> Box<EvalAltResult> {
    rhai_error(SCRIPT_TIMEOUT_MESSAGE)
}

/// 按片睡眠:每个片结束后检查挂钟 deadline,到点立即以超时错误中止脚本。
/// 单次长睡眠 (如 rshell_sleep(10000)) 不会再绕过 on_progress 的语句级检查点。
fn script_sleep_with_deadline(ms: i64) -> Result<(), Box<EvalAltResult>> {
    let mut remaining = Duration::from_millis(ms.clamp(0, 10_000) as u64);
    while !remaining.is_zero() {
        if exec_deadline_exceeded() {
            mark_exec_timed_out();
            return Err(script_timeout_error());
        }
        let slice = remaining.min(SLEEP_SLICE);
        std::thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
    Ok(())
}

/// 进入执行时设置线程级 deadline,Drop 时清除 (spawn_blocking 线程会被复用)。
struct ExecDeadlineGuard;

impl ExecDeadlineGuard {
    fn arm(deadline: Instant) -> Self {
        EXEC_DEADLINE.with(|d| d.set(Some(deadline)));
        EXEC_TIMED_OUT.with(|t| t.set(false));
        Self
    }
}

impl Drop for ExecDeadlineGuard {
    fn drop(&mut self) {
        EXEC_DEADLINE.with(|d| d.set(None));
        EXEC_TIMED_OUT.with(|t| t.set(false));
    }
}

/// 脚本引擎
pub struct ScriptEngine {
    /// Rhai 引擎实例
    engine: Engine,
    /// 事件总线
    event_bus: Arc<EventBus>,
    /// 单次脚本执行的挂钟时间上限 (PROB-20)
    time_limit: Duration,
}

/// 脚本上下文
pub struct ScriptContext {
    pub session_id: Uuid,
    pub target_sessions: Vec<Uuid>,
    pub variables: std::collections::HashMap<String, String>,
}

impl ScriptEngine {
    /// 创建新的脚本引擎
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self::build(event_bus, None, DEFAULT_MAX_EXECUTION_TIME)
    }

    pub fn with_host(event_bus: Arc<EventBus>, host: Arc<dyn ScriptHost>) -> Self {
        Self::build(event_bus, Some(host), DEFAULT_MAX_EXECUTION_TIME)
    }

    /// 自定义挂钟时间上限 (配置覆盖或测试用)
    pub fn with_time_limit(
        event_bus: Arc<EventBus>,
        host: Option<Arc<dyn ScriptHost>>,
        time_limit: Duration,
    ) -> Self {
        Self::build(event_bus, host, time_limit)
    }

    fn build(
        event_bus: Arc<EventBus>,
        host: Option<Arc<dyn ScriptHost>>,
        time_limit: Duration,
    ) -> Self {
        let mut engine = Engine::new();
        engine.set_max_operations(100_000);

        // PROB-20: 在 max_operations 之外增加挂钟时间上限。
        // on_progress 在每条语句/表达式求值后触发,deadline 过点即中止脚本;
        // rshell_sleep 自身按片睡眠并检查 deadline,保证单次长睡眠也不会
        // 让执行线程占用超过 时限 + 一个睡眠片。
        engine.on_progress(|_| {
            if exec_deadline_exceeded() {
                mark_exec_timed_out();
                Some(SCRIPT_TIMEOUT_MESSAGE.into())
            } else {
                None
            }
        });

        // 注册 rshell API 函数 (host API)
        // 命名约定: rshell_<verb>, 全部无副作用或仅 log, 不引入 host state
        // 引用以避免循环依赖: SessionService / TransferService / 等通过 dispatch
        // (AppCommand) 操作,脚本侧只发 intent,不直接调 service。
        engine.register_fn("rshell_log", |msg: String| {
            info!(target: "rshell_script", "{}", msg);
        });

        // 带级别的 log
        engine.register_fn("rshell_log_level", |level: &str, msg: String| {
            match level.to_ascii_lowercase().as_str() {
                "error" => tracing::error!(target: "rshell_script", "{}", msg),
                "warn" => tracing::warn!(target: "rshell_script", "{}", msg),
                "debug" => tracing::debug!(target: "rshell_script", "{}", msg),
                _ => tracing::info!(target: "rshell_script", "{}", msg),
            }
        });

        engine.register_fn(
            "rshell_sleep",
            |ms: i64| -> Result<(), Box<EvalAltResult>> { script_sleep_with_deadline(ms) },
        );

        if let Some(host) = host {
            let send_host = host.clone();
            engine.register_fn(
                "rshell_send",
                move |session: String, text: String| -> Result<(), Box<EvalAltResult>> {
                    let id = Uuid::parse_str(&session).map_err(rhai_error)?;
                    send_host.send_text(id, &text).map_err(rhai_error)
                },
            );
            let list_host = host.clone();
            engine.register_fn(
                "rshell_list_sessions",
                move || -> Result<rhai::Array, Box<EvalAltResult>> {
                    Ok(list_host
                        .list_sessions()
                        .map_err(rhai_error)?
                        .into_iter()
                        .map(|id| id.to_string().into())
                        .collect())
                },
            );
            engine.register_fn(
                "rshell_execute_quick_command",
                move |command: String, session: String| -> Result<(), Box<EvalAltResult>> {
                    let command_id = Uuid::parse_str(&command).map_err(rhai_error)?;
                    let session_id = Uuid::parse_str(&session).map_err(rhai_error)?;
                    host.execute_quick_command(command_id, session_id)
                        .map_err(rhai_error)
                },
            );
        }

        // 当前 Unix epoch 毫秒
        engine.register_fn("rshell_now_ms", || -> i64 {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        });

        // 生成新的 UUID v4 字符串
        engine.register_fn("rshell_uuid_v4", || -> String {
            Uuid::new_v4().to_string()
        });

        // 把字符串解析为 UUID, 失败返回空串
        engine.register_fn("rshell_parse_uuid", |s: &str| -> String {
            Uuid::parse_str(s)
                .map(|u| u.to_string())
                .unwrap_or_default()
        });

        // 比较两个版本字符串 (semver-ish: "1.2.3" vs "1.2.4")
        // 返回 -1 / 0 / 1 (统一 i64 便于 rhai 直接 == 比较)
        engine.register_fn("rshell_version_compare", |a: &str, b: &str| -> i64 {
            let parse =
                |s: &str| -> Vec<u64> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
            let av = parse(a);
            let bv = parse(b);
            for i in 0..av.len().max(bv.len()) {
                let x = *av.get(i).unwrap_or(&0);
                let y = *bv.get(i).unwrap_or(&0);
                if x < y {
                    return -1;
                }
                if x > y {
                    return 1;
                }
            }
            0
        });

        Self {
            engine,
            event_bus,
            time_limit,
        }
    }

    /// 执行脚本字符串
    pub fn execute_string(
        &self,
        code: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, CoreError> {
        info!(session_id = %context.session_id, "Executing script");

        let mut scope = Scope::new();
        scope.push("session_id", context.session_id.to_string());
        scope.push(
            "target_sessions",
            context
                .target_sessions
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );

        // 注入变量
        for (key, value) in &context.variables {
            scope.push(key.as_str(), value.clone());
        }

        // PROB-20: 为本次执行设置挂钟 deadline (on_progress 与 rshell_sleep 检查)。
        let _deadline = ExecDeadlineGuard::arm(Instant::now() + self.time_limit);

        match self
            .engine
            .eval_with_scope::<rhai::Dynamic>(&mut scope, code)
        {
            Ok(result) => {
                let output = format!("{:?}", result);
                debug!(output = %output, "Script executed successfully");
                Ok(ScriptResult {
                    success: true,
                    output,
                    error: None,
                })
            }
            Err(e) => {
                let message = if exec_timed_out() {
                    format!(
                        "{} (limit: {}ms)",
                        SCRIPT_TIMEOUT_MESSAGE,
                        self.time_limit.as_millis()
                    )
                } else {
                    e.to_string()
                };
                warn!(error = %message, "Script execution failed");
                Ok(ScriptResult {
                    success: false,
                    output: String::new(),
                    error: Some(message),
                })
            }
        }
    }

    /// 编译脚本为 AST（可缓存复用）
    pub fn compile(&self, code: &str) -> Result<AST, CoreError> {
        self.engine
            .compile(code)
            .map_err(|e| CoreError::Internal(format!("Script compile error: {}", e)))
    }

    /// 执行已编译的 AST
    pub fn execute_ast(
        &self,
        ast: &AST,
        context: &ScriptContext,
    ) -> Result<ScriptResult, CoreError> {
        let mut scope = Scope::new();
        scope.push("session_id", context.session_id.to_string());

        for (key, value) in &context.variables {
            scope.push(key.as_str(), value.clone());
        }

        // PROB-20: 为本次执行设置挂钟 deadline (on_progress 与 rshell_sleep 检查)。
        let _deadline = ExecDeadlineGuard::arm(Instant::now() + self.time_limit);

        match self
            .engine
            .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, ast)
        {
            Ok(result) => {
                let output = format!("{:?}", result);
                Ok(ScriptResult {
                    success: true,
                    output,
                    error: None,
                })
            }
            Err(e) => Ok(ScriptResult {
                success: false,
                output: String::new(),
                error: Some(if exec_timed_out() {
                    format!(
                        "{} (limit: {}ms)",
                        SCRIPT_TIMEOUT_MESSAGE,
                        self.time_limit.as_millis()
                    )
                } else {
                    e.to_string()
                }),
            }),
        }
    }

    /// 获取事件总线引用
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeHost {
        sent: Mutex<Vec<(Uuid, String)>>,
        quick: Mutex<Vec<(Uuid, Uuid)>>,
    }

    impl ScriptHost for FakeHost {
        fn send_text(&self, session_id: Uuid, text: &str) -> Result<(), CoreError> {
            self.sent
                .lock()
                .unwrap()
                .push((session_id, text.to_owned()));
            Ok(())
        }
        fn list_sessions(&self) -> Result<Vec<Uuid>, CoreError> {
            Ok(vec![Uuid::nil()])
        }
        fn execute_quick_command(
            &self,
            command_id: Uuid,
            session_id: Uuid,
        ) -> Result<(), CoreError> {
            self.quick.lock().unwrap().push((command_id, session_id));
            Ok(())
        }
    }

    #[test]
    fn host_api_sends_exact_text_and_lists_sessions() {
        let host = Arc::new(FakeHost::default());
        let engine = ScriptEngine::with_host(Arc::new(EventBus::new()), host.clone());
        let result = engine
            .execute_string(
                "rshell_send(session_id, \"clear\\n\"); rshell_list_sessions().len",
                &empty_ctx(),
            )
            .unwrap();
        assert!(result.success, "{result:?}");
        assert_eq!(
            host.sent.lock().unwrap().as_slice(),
            &[(Uuid::nil(), "clear\n".into())]
        );
        assert_eq!(result.output, "1");
    }

    #[test]
    fn host_api_quick_command_and_invalid_id_error() {
        let host = Arc::new(FakeHost::default());
        let engine = ScriptEngine::with_host(Arc::new(EventBus::new()), host.clone());
        let id = Uuid::new_v4();
        let code = format!("rshell_execute_quick_command(\"{id}\", session_id)");
        assert!(engine.execute_string(&code, &empty_ctx()).unwrap().success);
        assert_eq!(host.quick.lock().unwrap().as_slice(), &[(id, Uuid::nil())]);
        let failure = engine
            .execute_string("rshell_send(\"bad-id\", \"x\")", &empty_ctx())
            .unwrap();
        assert!(!failure.success);
        assert!(failure.error.unwrap().contains("invalid"));
    }

    fn make_engine() -> ScriptEngine {
        ScriptEngine::new(Arc::new(EventBus::new()))
    }

    fn empty_ctx() -> ScriptContext {
        ScriptContext {
            session_id: Uuid::nil(),
            target_sessions: vec![],
            variables: HashMap::new(),
        }
    }

    #[test]
    fn test_rshell_log_does_not_error() {
        let eng = make_engine();
        let r = eng
            .execute_string("rshell_log(\"hi\");", &empty_ctx())
            .unwrap();
        assert!(r.success);
    }

    #[test]
    fn test_rshell_now_ms_returns_positive() {
        let eng = make_engine();
        let r = eng
            .execute_string("let t = rshell_now_ms(); t > 0", &empty_ctx())
            .unwrap();
        assert!(r.success, "{:?}", r);
    }

    #[test]
    fn test_rshell_uuid_v4_unique() {
        let eng = make_engine();
        let r = eng
            .execute_string(
                "let a = rshell_uuid_v4(); let b = rshell_uuid_v4(); a != b",
                &empty_ctx(),
            )
            .unwrap();
        assert!(r.success);
    }

    #[test]
    fn test_rshell_parse_uuid_valid() {
        let eng = make_engine();
        let r = eng
            .execute_string(
                "let u = rshell_parse_uuid(\"550e8400-e29b-41d4-a716-446655440000\"); u.len() == 36",
                &empty_ctx(),
            )
            .unwrap();
        assert!(r.success);
    }

    #[test]
    fn test_rshell_parse_uuid_invalid_returns_empty() {
        let eng = make_engine();
        let r = eng
            .execute_string(
                "let u = rshell_parse_uuid(\"not-a-uuid\"); u.len() == 0",
                &empty_ctx(),
            )
            .unwrap();
        assert!(r.success);
    }

    #[test]
    fn test_rshell_version_compare() {
        let eng = make_engine();
        let r = eng
            .execute_string("rshell_version_compare(\"1.2.3\", \"1.2.4\")", &empty_ctx())
            .unwrap();
        assert!(r.success);
        // 验证结果 -1
        let r2 = eng
            .execute_string(
                "let c = rshell_version_compare(\"1.2.3\", \"1.2.4\"); c == -1",
                &empty_ctx(),
            )
            .unwrap();
        assert!(r2.success, "should be -1, got {:?}", r2);
    }

    #[test]
    fn test_rshell_log_level_accepted() {
        let eng = make_engine();
        let r = eng
            .execute_string("rshell_log_level(\"warn\", \"x\");", &empty_ctx())
            .unwrap();
        assert!(r.success);
    }

    #[test]
    fn test_invalid_script_returns_error_in_result() {
        let eng = make_engine();
        let r = eng.execute_string("not_a_real_fn()", &empty_ctx()).unwrap();
        assert!(!r.success);
        assert!(r.error.is_some());
    }

    /// PROB-20: 含长睡眠循环的脚本必须在设定时限内以超时错误中止,
    /// 而不是每次真实睡眠 10s 地占满阻塞线程。
    #[test]
    fn sleep_loop_aborts_with_timeout_within_time_limit() {
        let engine = ScriptEngine::with_time_limit(
            Arc::new(EventBus::new()),
            None,
            Duration::from_millis(150),
        );
        let started = Instant::now();
        let result = engine
            .execute_string(
                "let n = 0; while n < 1000000 { rshell_sleep(10000); n += 1; } n",
                &empty_ctx(),
            )
            .unwrap();
        let elapsed = started.elapsed();
        assert!(!result.success, "loop should be aborted: {result:?}");
        let error = result.error.expect("timeout error message");
        assert!(error.contains("timed out"), "unexpected error: {error}");
        // 完整执行需要小时级;限时中止后应在秒级内返回。
        assert!(elapsed < Duration::from_secs(5), "elapsed {elapsed:?}");
    }

    /// 时限内的短睡眠不受影响,确认不会误伤正常脚本。
    #[test]
    fn short_sleep_within_time_limit_succeeds() {
        let engine =
            ScriptEngine::with_time_limit(Arc::new(EventBus::new()), None, Duration::from_secs(2));
        let result = engine
            .execute_string("rshell_sleep(30); 42", &empty_ctx())
            .unwrap();
        assert!(result.success, "{result:?}");
        assert_eq!(result.output, "42");
    }
}
