//! 后端层错误定义

use thiserror::Error;

/// 后端层通用错误
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Service error: {0}")]
    ServiceError(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
}
