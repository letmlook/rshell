//! Rhai 脚本引擎
//!
//! 嵌入式脚本执行，提供 rshell API 给脚本环境。
//! 脚本可以连接/断开会话、发送命令、等待输出等。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use rshell_api::types::ScriptResult;
use rhai::{Engine, Scope, AST};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// 脚本引擎
pub struct ScriptEngine {
    /// Rhai 引擎实例
    engine: Engine,
    /// 事件总线
    event_bus: Arc<EventBus>,
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
        let mut engine = Engine::new();

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

        engine.register_fn("rshell_sleep", |ms: i64| {
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
        });

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
            Uuid::parse_str(s).map(|u| u.to_string()).unwrap_or_default()
        });

        // 比较两个版本字符串 (semver-ish: "1.2.3" vs "1.2.4")
        // 返回 -1 / 0 / 1 (统一 i64 便于 rhai 直接 == 比较)
        engine.register_fn("rshell_version_compare", |a: &str, b: &str| -> i64 {
            let parse = |s: &str| -> Vec<u64> {
                s.split('.').filter_map(|p| p.parse().ok()).collect()
            };
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
        }
    }

    /// 执行脚本字符串
    pub fn execute_string(&self, code: &str, context: &ScriptContext) -> Result<ScriptResult, CoreError> {
        info!(session_id = %context.session_id, "Executing script");

        let mut scope = Scope::new();
        scope.push("session_id", context.session_id.to_string());
        scope.push("target_sessions", context.target_sessions.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(","));

        // 注入变量
        for (key, value) in &context.variables {
            scope.push(key.as_str(), value.clone());
        }

        match self.engine.eval_with_scope::<rhai::Dynamic>(&mut scope, code) {
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
                warn!(error = %e, "Script execution failed");
                Ok(ScriptResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
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
    pub fn execute_ast(&self, ast: &AST, context: &ScriptContext) -> Result<ScriptResult, CoreError> {
        let mut scope = Scope::new();
        scope.push("session_id", context.session_id.to_string());

        for (key, value) in &context.variables {
            scope.push(key.as_str(), value.clone());
        }

        match self.engine.eval_ast_with_scope::<rhai::Dynamic>(&mut scope, ast) {
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
                error: Some(e.to_string()),
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
        let r = eng.execute_string("rshell_log(\"hi\");", &empty_ctx()).unwrap();
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
}
