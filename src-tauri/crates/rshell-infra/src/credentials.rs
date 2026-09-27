//! Native credential adapter. Backend diagnostics are never included in public errors.

use rshell_api::credentials::{CredentialError, CredentialKey, CredentialStore};
use std::sync::Arc;

enum BackendError {
    Missing,
    Unavailable,
    Failed(String),
}

trait Backend: Send + Sync {
    fn get(&self, service: &str, account: &str) -> Result<String, BackendError>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), BackendError>;
}

struct KeyringBackend;

fn map_keyring_error(error: keyring::v1::Error) -> BackendError {
    match error {
        keyring::v1::Error::NoEntry => BackendError::Missing,
        keyring::v1::Error::NoDefaultStore | keyring::v1::Error::NoStorageAccess(_) => {
            BackendError::Unavailable
        }
        _ => BackendError::Failed(String::new()),
    }
}

impl Backend for KeyringBackend {
    fn get(&self, service: &str, account: &str) -> Result<String, BackendError> {
        let entry = keyring::v1::Entry::new(service, account).map_err(map_keyring_error)?;
        entry.get_password().map_err(map_keyring_error)
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError> {
        let entry = keyring::v1::Entry::new(service, account).map_err(map_keyring_error)?;
        entry.set_password(secret).map_err(map_keyring_error)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), BackendError> {
        let entry = keyring::v1::Entry::new(service, account).map_err(map_keyring_error)?;
        entry.delete_credential().map_err(map_keyring_error)
    }
}

pub struct SystemCredentialStore {
    service: &'static str,
    backend: Arc<dyn Backend>,
}

impl SystemCredentialStore {
    pub fn new(service: &'static str) -> Self {
        Self {
            service,
            backend: Arc::new(KeyringBackend),
        }
    }

    #[cfg(test)]
    fn with_backend(service: &'static str, backend: Arc<dyn Backend>) -> Self {
        Self { service, backend }
    }
}

fn sanitize(error: BackendError) -> CredentialError {
    match error {
        BackendError::Unavailable => CredentialError::Unavailable,
        BackendError::Missing => CredentialError::BackendFailure,
        BackendError::Failed(diagnostic) => {
            drop(diagnostic);
            CredentialError::BackendFailure
        }
    }
}

impl CredentialStore for SystemCredentialStore {
    fn get(&self, key: &CredentialKey) -> Result<Option<String>, CredentialError> {
        match self.backend.get(self.service, &key.account()) {
            Ok(secret) => Ok(Some(secret)),
            Err(BackendError::Missing) => Ok(None),
            Err(error) => Err(sanitize(error)),
        }
    }

    fn set(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialError> {
        self.backend
            .set(self.service, &key.account(), secret)
            .map_err(sanitize)
    }

    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        match self.backend.delete(self.service, &key.account()) {
            Ok(()) | Err(BackendError::Missing) => Ok(()),
            Err(error) => Err(sanitize(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendError, SystemCredentialStore};
    use rshell_api::credentials::{
        CredentialError, CredentialKey, CredentialKind, CredentialStore,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    #[derive(Default)]
    struct MemoryBackend {
        entries: Mutex<HashMap<(String, String), String>>,
        failure: Mutex<Option<String>>,
    }

    impl Backend for MemoryBackend {
        fn get(&self, service: &str, account: &str) -> Result<String, BackendError> {
            if let Some(secret) = self.failure.lock().unwrap().as_ref() {
                return Err(BackendError::Failed(secret.clone()));
            }
            self.entries
                .lock()
                .unwrap()
                .get(&(service.into(), account.into()))
                .cloned()
                .ok_or(BackendError::Missing)
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError> {
            if let Some(secret) = self.failure.lock().unwrap().as_ref() {
                return Err(BackendError::Failed(secret.clone()));
            }
            self.entries
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), secret.into());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), BackendError> {
            if let Some(secret) = self.failure.lock().unwrap().as_ref() {
                return Err(BackendError::Failed(secret.clone()));
            }
            self.entries
                .lock()
                .unwrap()
                .remove(&(service.into(), account.into()))
                .map(|_| ())
                .ok_or(BackendError::Missing)
        }
    }

    fn key(kind: CredentialKind) -> CredentialKey {
        CredentialKey {
            session_id: Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            kind,
        }
    }

    #[test]
    fn set_get_delete_use_distinct_accounts() {
        let backend = Arc::new(MemoryBackend::default());
        let store: Box<dyn CredentialStore> = Box::new(SystemCredentialStore::with_backend(
            "com.letmlook.rshell.credentials",
            backend.clone(),
        ));
        let password = key(CredentialKind::Password);
        let passphrase = key(CredentialKind::Passphrase);
        store.set(&password, "password-value").unwrap();
        store.set(&passphrase, "passphrase-value").unwrap();
        assert_eq!(
            store.get(&password).unwrap().as_deref(),
            Some("password-value")
        );
        assert_eq!(
            store.get(&passphrase).unwrap().as_deref(),
            Some("passphrase-value")
        );
        assert!(backend.entries.lock().unwrap().contains_key(&(
            "com.letmlook.rshell.credentials".into(),
            "session:123e4567-e89b-12d3-a456-426614174000:password".into()
        )));
        assert!(backend.entries.lock().unwrap().contains_key(&(
            "com.letmlook.rshell.credentials".into(),
            "session:123e4567-e89b-12d3-a456-426614174000:passphrase".into()
        )));
        store.delete(&password).unwrap();
        assert_eq!(store.get(&password).unwrap(), None);
        assert_eq!(
            store.get(&passphrase).unwrap().as_deref(),
            Some("passphrase-value")
        );
    }

    #[test]
    fn deleting_missing_entry_is_idempotent() {
        let store =
            SystemCredentialStore::with_backend("test-service", Arc::new(MemoryBackend::default()));
        store.delete(&key(CredentialKind::Password)).unwrap();
        store.delete(&key(CredentialKind::Password)).unwrap();
        assert_eq!(store.get(&key(CredentialKind::Password)).unwrap(), None);
    }

    #[test]
    fn empty_secret_round_trips() {
        let store =
            SystemCredentialStore::with_backend("test-service", Arc::new(MemoryBackend::default()));
        let key = key(CredentialKind::Password);
        store.set(&key, "").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(""));
    }

    #[test]
    fn backend_failure_does_not_expose_secret() {
        let backend = Arc::new(MemoryBackend::default());
        *backend.failure.lock().unwrap() = Some("sensitive-backend-value".into());
        let store = SystemCredentialStore::with_backend("test-service", backend);
        let key = key(CredentialKind::Password);
        for error in [
            store.get(&key).unwrap_err(),
            store.set(&key, "sensitive-input").unwrap_err(),
            store.delete(&key).unwrap_err(),
        ] {
            assert_eq!(error, CredentialError::BackendFailure);
            assert!(!format!("{error:?} {error}").contains("sensitive"));
        }
    }
}
