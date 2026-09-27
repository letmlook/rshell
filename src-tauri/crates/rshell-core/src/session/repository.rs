//! 会话存储仓库
//!
//! 提供会配置的持久化操作接口。

use rshell_api::credentials::{CredentialKey, CredentialKind, CredentialStore};
use rshell_api::types::{
    AuthMethod, CredentialUpdate, SessionConfig, SessionCredential, SessionLoadIssue,
};
use rshell_infra::storage::session_store::SessionStore;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// 会话仓库
pub struct SessionRepository {
    store: SessionStore,
    credentials: Arc<dyn CredentialStore>,
    transaction: Mutex<()>,
}

#[derive(Default)]
pub struct SessionLoadReport {
    pub sessions: Vec<SessionConfig>,
    pub issues: Vec<SessionLoadIssue>,
}

impl SessionRepository {
    /// 创建新的仓库
    pub fn new(path: PathBuf, credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            store: SessionStore::new(path),
            credentials,
            transaction: Mutex::new(()),
        }
    }

    /// 使用默认路径创建仓库
    pub fn with_default_path(credentials: Arc<dyn CredentialStore>) -> Self {
        Self::new(SessionStore::default_path(), credentials)
    }

    /// Fresh metadata has no previous credential to clean up. Never reuse this
    /// path for updates or failed legacy migrations with an existing file.
    pub fn create(
        &self,
        session: &SessionConfig,
        credential: Option<SessionCredential>,
    ) -> anyhow::Result<()> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        anyhow::ensure!(
            self.store.load_pending(session.id)?.is_none(),
            "Session already exists"
        );
        match credential {
            Some(credential) if !credential.secret.is_empty() => {
                self.save_locked(session, CredentialUpdate::Set(credential))
            }
            _ => {
                let mut metadata = session.clone();
                set_presence(&mut metadata, false);
                self.store.save(&metadata)
            }
        }
    }

    /// 保存会话
    pub fn save(&self, session: &SessionConfig, update: CredentialUpdate) -> anyhow::Result<()> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        self.save_locked(session, update)
    }

    fn save_locked(&self, session: &SessionConfig, update: CredentialUpdate) -> anyhow::Result<()> {
        let mut metadata = session.clone();
        let kind = credential_kind(&metadata);
        if matches!(update, CredentialUpdate::Keep) {
            let previous = self.store.load(session.id)?;
            if let Some(previous) = &previous {
                anyhow::ensure!(
                    credential_kind(previous) == kind,
                    "Changing credential kind requires Set or Clear"
                );
            }
            set_presence(&mut metadata, previous.as_ref().is_some_and(has_credential));
            return self.store.save(&metadata);
        }
        let keys =
            [CredentialKind::Password, CredentialKind::Passphrase].map(|kind| CredentialKey {
                session_id: session.id,
                kind,
            });
        let previous = [
            self.credentials.get(&keys[0])?,
            self.credentials.get(&keys[1])?,
        ];
        let secret = match &update {
            CredentialUpdate::Set(credential) if !credential.secret.is_empty() => {
                Some(credential.secret.as_str())
            }
            _ => None,
        };
        set_presence(&mut metadata, secret.is_some());
        let result = (|| -> anyhow::Result<()> {
            for key in &keys {
                if key.kind == kind {
                    if let Some(secret) = secret {
                        self.credentials.set(key, secret)?;
                        continue;
                    }
                }
                self.credentials.delete(key)?;
            }
            self.store.save(&metadata)
        })();
        if let Err(error) = result {
            let mut rollback_failed = false;
            for (key, secret) in keys.iter().zip(previous.iter()) {
                let restored = match secret {
                    Some(secret) => self.credentials.set(key, secret),
                    None => self.credentials.delete(key),
                };
                rollback_failed |= restored.is_err();
            }
            if rollback_failed {
                return Err(anyhow::anyhow!(
                    "Session save failed; credential rollback failed"
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    /// 加载会话
    pub fn load(&self, id: Uuid) -> anyhow::Result<Option<SessionConfig>> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        self.load_locked(id)
    }

    fn load_locked(&self, id: Uuid) -> anyhow::Result<Option<SessionConfig>> {
        let Some((config, migration)) = self.store.load_pending(id)? else {
            return Ok(None);
        };
        if let Some(update) = migration {
            self.save_locked(&config, update)?;
        }
        Ok(Some(config))
    }

    /// 删除会话
    pub fn delete(&self, id: Uuid) -> anyhow::Result<()> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        self.store.delete(id)?;
        // Attempt both entries even when one backend deletion fails.
        let password = self.credentials.delete(&CredentialKey {
            session_id: id,
            kind: CredentialKind::Password,
        });
        let passphrase = self.credentials.delete(&CredentialKey {
            session_id: id,
            kind: CredentialKind::Passphrase,
        });
        password?;
        passphrase?;
        Ok(())
    }

    /// 列出所有会话
    pub fn list_all(&self) -> anyhow::Result<SessionLoadReport> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        let mut report = SessionLoadReport::default();
        for id in self.store.list_ids()? {
            match self.load_locked(id) {
                Ok(Some(config)) => report.sessions.push(config),
                Ok(None) => {}
                Err(_) => {
                    tracing::warn!(session_id = %id, "Could not load or migrate session");
                    report.issues.push(SessionLoadIssue {
                        session_id: Some(id),
                        message: "Could not load or migrate saved session. Check configuration and Keychain access, then retry.".into(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Resolve only at the SSH connection boundary; never cache in session state.
    pub fn credential(&self, config: &SessionConfig) -> anyhow::Result<Option<String>> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| anyhow::anyhow!("Session transaction unavailable"))?;
        if !has_credential(config) {
            return Ok(None);
        }
        let secret = self.credentials.get(&CredentialKey {
            session_id: config.id,
            kind: credential_kind(config),
        })?;
        anyhow::ensure!(
            secret.as_ref().is_some_and(|s| !s.is_empty()),
            "Stored session credential is unavailable"
        );
        Ok(secret)
    }
}

fn credential_kind(config: &SessionConfig) -> CredentialKind {
    match config.auth_method {
        AuthMethod::PublicKey { .. } => CredentialKind::Passphrase,
        _ => CredentialKind::Password,
    }
}

pub(crate) fn has_credential(config: &SessionConfig) -> bool {
    match config.auth_method {
        AuthMethod::Password { has_password, .. }
        | AuthMethod::KeyboardInteractive { has_password, .. } => has_password,
        AuthMethod::PublicKey { has_passphrase, .. } => has_passphrase,
    }
}

pub(crate) fn set_presence(config: &mut SessionConfig, present: bool) {
    match &mut config.auth_method {
        AuthMethod::Password { has_password, .. }
        | AuthMethod::KeyboardInteractive { has_password, .. } => *has_password = present,
        AuthMethod::PublicKey { has_passphrase, .. } => *has_passphrase = present,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rshell_api::credentials::{
        CredentialError, CredentialKey, CredentialKind, CredentialStore,
    };
    use rshell_api::types::{AuthMethod, CredentialUpdate, Protocol, SessionCredential};
    use std::{
        collections::HashMap,
        fs,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    pub(crate) struct MemoryCredentials {
        pub entries: Mutex<HashMap<CredentialKey, String>>,
        pub fail: Mutex<bool>,
    }
    impl CredentialStore for MemoryCredentials {
        fn get(&self, key: &CredentialKey) -> Result<Option<String>, CredentialError> {
            if *self.fail.lock().unwrap() {
                return Err(CredentialError::Unavailable);
            }
            Ok(self.entries.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialError> {
            if *self.fail.lock().unwrap() {
                return Err(CredentialError::Unavailable);
            }
            self.entries.lock().unwrap().insert(*key, secret.into());
            Ok(())
        }
        fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
            if *self.fail.lock().unwrap() {
                return Err(CredentialError::Unavailable);
            }
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }
    }

    pub(crate) fn config() -> SessionConfig {
        SessionConfig {
            id: Uuid::new_v4(),
            name: "session".into(),
            folder_id: None,
            host: "localhost".into(),
            port: 22,
            protocol: Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "user".into(),
                has_password: true,
            },
            serial_config: None,
        }
    }
    fn key(id: Uuid, kind: CredentialKind) -> CredentialKey {
        CredentialKey {
            session_id: id,
            kind,
        }
    }
    pub(crate) fn set(secret: &str) -> CredentialUpdate {
        CredentialUpdate::Set(SessionCredential {
            secret: secret.into(),
        })
    }
    fn legacy(id: Uuid, auth: &str) -> String {
        format!("# original legacy bytes\nid = '{id}'\nname = 'legacy'\nhost = 'localhost'\nport = 22\nprotocol = 'SSH'\n{auth}\n")
    }
    fn assert_safe(path: &std::path::Path) {
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("sample-secret"));
        assert!(!text
            .lines()
            .any(|line| line.starts_with("password =") || line.starts_with("passphrase =")));
    }

    #[test]
    fn migrates_all_legacy_auth_variants_before_returning() {
        for (auth, kind) in [
            ("[auth_method.Password]\nusername = 'user'\npassword = 'sample-secret'", CredentialKind::Password),
            ("[auth_method.PublicKey]\nusername = 'user'\nkey_path = '/tmp/key'\npassphrase = 'sample-secret'", CredentialKind::Passphrase),
            ("[auth_method.KeyboardInteractive]\nusername = 'user'\npassword = 'sample-secret'", CredentialKind::Password),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let id = Uuid::new_v4();
            let path = dir.path().join(format!("{id}.toml"));
            fs::write(&path, legacy(id, auth)).unwrap();
            let credentials = Arc::new(MemoryCredentials::default());
            let repo = SessionRepository::new(dir.path().into(), credentials.clone());
            assert_eq!(repo.load(id).unwrap().unwrap().id, id);
            assert_eq!(credentials.get(&key(id, kind)).unwrap().as_deref(), Some("sample-secret"));
            assert_safe(&path);
            assert_eq!(repo.list_all().unwrap().sessions.len(), 1);
        }
    }

    #[test]
    fn failed_migration_preserves_original_bytes_and_omits_session() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let path = dir.path().join(format!("{id}.toml"));
        let bytes = legacy(
            id,
            "[auth_method.Password]\nusername = 'user'\npassword = 'sample-secret'",
        );
        fs::write(&path, &bytes).unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        *credentials.fail.lock().unwrap() = true;
        let repo = SessionRepository::new(dir.path().into(), credentials);
        assert!(repo.load(id).is_err());
        let report = repo.list_all().unwrap();
        assert!(report.sessions.is_empty());
        assert_eq!(report.issues[0].session_id, Some(id));
        assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
    }

    #[test]
    fn metadata_keep_preserves_secret_and_serialized_metadata_is_safe() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let mut cfg = config();
        repo.save(&cfg, set("sample-secret")).unwrap();
        cfg.name = "renamed".into();
        repo.save(&cfg, CredentialUpdate::Keep).unwrap();
        assert_eq!(
            credentials
                .get(&key(cfg.id, CredentialKind::Password))
                .unwrap()
                .as_deref(),
            Some("sample-secret")
        );
        assert_eq!(repo.load(cfg.id).unwrap().unwrap().name, "renamed");
        assert_safe(&dir.path().join(format!("{}.toml", cfg.id)));
    }

    #[test]
    fn failed_metadata_save_restores_previous_credential_and_removes_new_one() {
        for previous in [None, Some("old-secret")] {
            let dir = tempfile::tempdir().unwrap();
            let credentials = Arc::new(MemoryCredentials::default());
            let repo = SessionRepository::new(dir.path().into(), credentials.clone());
            let cfg = config();
            let key = key(cfg.id, CredentialKind::Password);
            if let Some(secret) = previous {
                credentials.set(&key, secret).unwrap();
            }
            fs::create_dir(dir.path().join(format!("{}.toml", cfg.id))).unwrap();
            assert!(repo.save(&cfg, set("sample-secret")).is_err());
            assert_eq!(credentials.get(&key).unwrap().as_deref(), previous);
        }
    }

    #[test]
    fn empty_password_creates_no_entry_and_is_marked_absent() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let cfg = config();
        repo.save(&cfg, set("")).unwrap();
        assert!(credentials.entries.lock().unwrap().is_empty());
        assert!(matches!(
            repo.load(cfg.id).unwrap().unwrap().auth_method,
            AuthMethod::Password {
                has_password: false,
                ..
            }
        ));
    }

    #[test]
    fn delete_is_idempotent_and_removes_both_credential_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let cfg = config();
        repo.save(&cfg, set("sample-secret")).unwrap();
        credentials
            .set(&key(cfg.id, CredentialKind::Passphrase), "old-key-secret")
            .unwrap();
        repo.delete(cfg.id).unwrap();
        repo.delete(cfg.id).unwrap();
        assert!(credentials.entries.lock().unwrap().is_empty());
        assert!(repo.load(cfg.id).unwrap().is_none());
    }

    #[test]
    fn metadata_delete_failure_does_not_delete_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let cfg = config();
        let key = key(cfg.id, CredentialKind::Password);
        credentials.set(&key, "sample-secret").unwrap();
        fs::create_dir(dir.path().join(format!("{}.toml", cfg.id))).unwrap();
        assert!(repo.delete(cfg.id).is_err());
        assert_eq!(
            credentials.get(&key).unwrap().as_deref(),
            Some("sample-secret")
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_metadata_failure_preserves_exact_bytes_and_rolls_back_keychain() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let path = dir.path().join(format!("{id}.toml"));
        let bytes = legacy(
            id,
            "[auth_method.Password]\nusername = 'user'\npassword = 'sample-secret'",
        );
        fs::write(&path, &bytes).unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = repo.load(id);
        let listed = repo.list_all();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert!(listed.unwrap().sessions.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
        assert!(credentials.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn keep_works_with_unavailable_keychain_and_preserves_presence() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let mut cfg = config();
        repo.save(&cfg, set("sample-secret")).unwrap();
        *credentials.fail.lock().unwrap() = true;
        cfg.name = "renamed".into();
        set_presence(&mut cfg, false);
        repo.save(&cfg, CredentialUpdate::Keep).unwrap();
        assert!(has_credential(&repo.load(cfg.id).unwrap().unwrap()));
        assert_eq!(
            credentials
                .entries
                .lock()
                .unwrap()
                .get(&key(cfg.id, CredentialKind::Password))
                .map(String::as_str),
            Some("sample-secret")
        );
    }

    #[test]
    fn clear_and_auth_kind_switch_do_not_leave_stale_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = SessionRepository::new(dir.path().into(), credentials.clone());
        let mut cfg = config();
        repo.save(&cfg, set("sample-secret")).unwrap();
        cfg.auth_method = AuthMethod::PublicKey {
            username: "user".into(),
            key_path: "/tmp/key".into(),
            has_passphrase: true,
        };
        repo.save(&cfg, set("sample-secret-passphrase")).unwrap();
        assert!(credentials
            .get(&key(cfg.id, CredentialKind::Password))
            .unwrap()
            .is_none());
        assert_eq!(
            credentials
                .get(&key(cfg.id, CredentialKind::Passphrase))
                .unwrap()
                .as_deref(),
            Some("sample-secret-passphrase")
        );
        repo.save(&cfg, CredentialUpdate::Clear).unwrap();
        assert!(credentials.entries.lock().unwrap().is_empty());
        assert!(!has_credential(&repo.load(cfg.id).unwrap().unwrap()));
    }
}
