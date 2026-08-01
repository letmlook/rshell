//! Tauri 壳 ↔ 后端 CoreError 之间的 IPC 错误映射（设计 §3.5）。
//!
//! 规则：`CoreError` 不直接过 IPC（含 `anyhow::Error` 等不可序列化内容）。
//! 通过 `IpcError { kind, message, session_id }` 三段式传至前端：
//! - `kind`：稳定的机器可读判别串，前端据此分支（toast / 重试 / 重弹 host key 框等）
//! - `message`：仅用于展示，**不要**做解析依据
//! - `session_id`：可选，便于前端把错误挂到正确的会话行

use serde::Serialize;
use uuid::Uuid;

use rshell_core::CoreError;

/// 固定 kind 集合（设计 §3.5）。新增变体前请先在设计文档追加。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorKind {
    NotFound,
    AuthFailed,
    HostKeyMismatch,
    Connection,
    Io,
    Permission,
    OutcomeMismatch,
    Internal,
    Storage,
}

impl IpcErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AuthFailed => "auth_failed",
            Self::HostKeyMismatch => "host_key_mismatch",
            Self::Connection => "connection",
            Self::Io => "io",
            Self::Permission => "permission",
            Self::OutcomeMismatch => "outcome_mismatch",
            Self::Internal => "internal",
            Self::Storage => "storage",
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct IpcError {
    pub kind: String,
    pub message: String,
    pub session_id: Option<Uuid>,
}

impl IpcError {
    pub fn new(kind: IpcErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind: kind.as_str().to_string(),
            message: message.into(),
            session_id: None,
        }
    }

    pub fn with_session(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// 仅在 dispatcher 返回的 `CommandOutcome` 与预期变体不匹配时调用。
    /// 理论上不可达 —— 它只在 dispatcher 分支写错时触发，等于运行时断言。
    /// 用 `IpcError` 表达而不是 `unreachable!()`，符合设计 §3.4 与 §6.3 不变量。
    pub fn outcome_mismatch(expected: &str, actual: &str) -> Self {
        Self::new(
            IpcErrorKind::OutcomeMismatch,
            format!("dispatcher returned {:?} where {:?} expected", actual, expected),
        )
    }
}

impl From<CoreError> for IpcError {
    fn from(err: CoreError) -> Self {
        let (kind, message) = match &err {
            CoreError::NotFound(_) => (IpcErrorKind::NotFound, err.to_string()),
            CoreError::AuthError(_) | CoreError::AuthenticationFailed(_) => {
                (IpcErrorKind::AuthFailed, err.to_string())
            }
            CoreError::ConnectionError(_) => (IpcErrorKind::Connection, err.to_string()),
            CoreError::StorageError(_) => (IpcErrorKind::Storage, err.to_string()),
            CoreError::InvalidState(_) => (IpcErrorKind::Internal, err.to_string()),
            CoreError::ServiceError(_) | CoreError::Internal(_) => {
                (IpcErrorKind::Internal, err.to_string())
            }
        };
        Self {
            kind: kind.as_str().to_string(),
            message,
            session_id: None,
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for IpcError {}

/// Tauri `#[tauri::command]` 的返回值要求 `Result<T, E>` 中 `E: Serialize`。
/// `String` 是兜底透传通道 —— 真正的结构化错误由薄壳优先返回 `IpcError` 序列化。
/// 切片 1.2 的宏统一走 `IpcError`；少数 fallback 仍可用 `String`（如 channel 推失败）。
impl From<IpcError> for String {
    fn from(err: IpcError) -> Self {
        serde_json::to_string(&err).unwrap_or_else(|e| {
            format!(r#"{{"kind":"internal","message":"IpcError serialize failed: {}"}}"#, e)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_strings_are_stable() {
        assert_eq!(IpcErrorKind::NotFound.as_str(), "not_found");
        assert_eq!(IpcErrorKind::AuthFailed.as_str(), "auth_failed");
        assert_eq!(IpcErrorKind::HostKeyMismatch.as_str(), "host_key_mismatch");
        assert_eq!(IpcErrorKind::Connection.as_str(), "connection");
        assert_eq!(IpcErrorKind::Io.as_str(), "io");
        assert_eq!(IpcErrorKind::Permission.as_str(), "permission");
        assert_eq!(IpcErrorKind::OutcomeMismatch.as_str(), "outcome_mismatch");
        assert_eq!(IpcErrorKind::Internal.as_str(), "internal");
        assert_eq!(IpcErrorKind::Storage.as_str(), "storage");
    }

    #[test]
    fn core_error_mapping_is_exhaustive() {
        let cases = [
            CoreError::NotFound("x".into()),
            CoreError::AuthError("x".into()),
            CoreError::AuthenticationFailed("x".into()),
            CoreError::ConnectionError("x".into()),
            CoreError::StorageError("x".into()),
            CoreError::InvalidState("x".into()),
            CoreError::ServiceError("x".into()),
            CoreError::Internal("x".into()),
        ];
        let kinds: Vec<String> = cases
            .into_iter()
            .map(|c| {
                let ipc: IpcError = c.into();
                ipc.kind
            })
            .collect();
        // 必须全部命中 5 个稳定 kind 之一(不含 HostKeyMismatch —— 它由
        // host_key_decision 单独 publish,不走 CoreError 路径)
        for k in &kinds {
            assert!(
                ["not_found", "auth_failed", "connection", "storage", "internal"]
                    .iter()
                    .any(|allowed| allowed == &k.as_str()),
                "unexpected kind: {}",
                k
            );
        }
    }

    #[test]
    fn serializes_to_expected_json_shape() {
        let e = IpcError::new(IpcErrorKind::Connection, "boom");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "connection");
        assert_eq!(json["message"], "boom");
        assert!(json["session_id"].is_null());
    }
}