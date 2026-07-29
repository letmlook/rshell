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

        // 注册 rshell API 函数
        engine.register_fn("rshell_log", |msg: String| {
            info!(target: "rshell_script", "{}", msg);
        });

        engine.register_fn("rshell_sleep", |ms: i64| {
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
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
