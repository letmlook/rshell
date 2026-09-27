//! Portable credential storage boundary. Values never appear in public session metadata.

use std::fmt;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CredentialKind {
    Password,
    Passphrase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CredentialKey {
    pub session_id: Uuid,
    pub kind: CredentialKind,
}

impl CredentialKey {
    pub fn account(&self) -> String {
        let kind = match self.kind {
            CredentialKind::Password => "password",
            CredentialKind::Passphrase => "passphrase",
        };
        format!("session:{}:{kind}", self.session_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialError {
    Unavailable,
    BackendFailure,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("credential store unavailable"),
            Self::BackendFailure => f.write_str("credential store operation failed"),
        }
    }
}

impl std::error::Error for CredentialError {}

pub trait CredentialStore: Send + Sync {
    fn get(&self, key: &CredentialKey) -> Result<Option<String>, CredentialError>;
    fn set(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError>;
}
