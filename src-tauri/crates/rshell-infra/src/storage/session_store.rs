//! 会话存储
//!
//! 使用 TOML 文件格式持久化会话配置。
//! 每个会话存储为独立的 `.toml` 文件，位于配置目录下。

use rshell_api::types::{AuthMethod, CredentialUpdate, SessionConfig, SessionCredential};
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tracing::{debug, warn};
use uuid::Uuid;

// Private compatibility DTO: never serialized, logged, or sent through IPC.
#[derive(Deserialize)]
enum StoredAuth {
    Password {
        username: String,
        has_password: Option<bool>,
        password: Option<String>,
    },
    PublicKey {
        username: String,
        key_path: PathBuf,
        has_passphrase: Option<bool>,
        passphrase: Option<String>,
    },
    KeyboardInteractive {
        username: String,
        has_password: Option<bool>,
        password: Option<String>,
    },
}

fn decode(content: &str) -> anyhow::Result<(SessionConfig, Option<CredentialUpdate>)> {
    // TOML diagnostics contain source lines; deliberately discard them.
    let invalid = || anyhow::anyhow!("Invalid session metadata");
    let mut value: toml::Value = toml::from_str(content).map_err(|_| invalid())?;
    let auth: StoredAuth = value
        .get("auth_method")
        .ok_or_else(invalid)?
        .clone()
        .try_into()
        .map_err(|_| invalid())?;
    let (auth, legacy, secret) = match auth {
        StoredAuth::Password {
            username,
            has_password,
            password,
        } => {
            if has_password.is_none() && password.is_none() {
                return Err(invalid());
            }
            let legacy = password.is_some() || has_password.is_none();
            (
                AuthMethod::Password {
                    username,
                    has_password: password
                        .as_ref()
                        .map_or(has_password.unwrap_or(false), |s| !s.is_empty()),
                },
                legacy,
                password,
            )
        }
        StoredAuth::PublicKey {
            username,
            key_path,
            has_passphrase,
            passphrase,
        } => {
            let legacy = passphrase.is_some() || has_passphrase.is_none();
            (
                AuthMethod::PublicKey {
                    username,
                    key_path,
                    has_passphrase: passphrase
                        .as_ref()
                        .map_or(has_passphrase.unwrap_or(false), |s| !s.is_empty()),
                },
                legacy,
                passphrase,
            )
        }
        StoredAuth::KeyboardInteractive {
            username,
            has_password,
            password,
        } => {
            let legacy = password.is_some() || has_password.is_none();
            (
                AuthMethod::KeyboardInteractive {
                    username,
                    has_password: password
                        .as_ref()
                        .map_or(has_password.unwrap_or(false), |s| !s.is_empty()),
                },
                legacy,
                password,
            )
        }
    };
    value["auth_method"] = toml::Value::try_from(auth).map_err(|_| invalid())?;
    let config = value.try_into().map_err(|_| invalid())?;
    let migration = legacy.then_some(match secret {
        Some(secret) if !secret.is_empty() => CredentialUpdate::Set(SessionCredential { secret }),
        _ => CredentialUpdate::Clear,
    });
    Ok((config, migration))
}

/// 会话存储
pub struct SessionStore {
    /// 会话文件存储目录
    dir: PathBuf,
}

impl SessionStore {
    /// 创建新的存储实例
    ///
    /// `path` 可以是目录路径或文件路径：
    /// - 如果是目录，每个会话存储为该目录下的 `{id}.toml`
    /// - 如果是文件，所有会话存储在单个文件中（暂不支持）
    pub fn new(path: PathBuf) -> Self {
        Self { dir: path }
    }

    /// 确保存储目录存在
    fn ensure_dir(&self) -> anyhow::Result<()> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
            debug!(dir = %self.dir.display(), "Created session store directory");
        }
        Ok(())
    }

    /// 获取会话文件路径
    fn session_path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{}.toml", id))
    }

    /// 保存会话
    pub fn save(&self, session: &SessionConfig) -> anyhow::Result<()> {
        self.ensure_dir()?;

        let path = self.session_path(session.id);
        let content = toml::to_string_pretty(session)
            .map_err(|_| anyhow::anyhow!("Could not serialize session metadata"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.dir)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        // Same-directory rename is the commit point. Never report a failure
        // after it: callers would otherwise roll back an already committed secret.
        temporary.persist(&path).map_err(|e| e.error)?;

        debug!(id = %session.id, path = %path.display(), "Session saved");
        Ok(())
    }

    /// 加载会话
    pub fn load(&self, id: Uuid) -> anyhow::Result<Option<SessionConfig>> {
        self.load_pending(id)?
            .map(|(config, migration)| {
                anyhow::ensure!(migration.is_none(), "Session requires credential migration");
                Ok(config)
            })
            .transpose()
    }

    /// Repository-only migration boundary; callers must commit before exposing config.
    pub fn load_pending(
        &self,
        id: Uuid,
    ) -> anyhow::Result<Option<(SessionConfig, Option<CredentialUpdate>)>> {
        let path = self.session_path(id);

        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)?;
        let (config, migration) = decode(&content)?;
        anyhow::ensure!(
            config.id == id,
            "Session metadata ID does not match filename"
        );

        debug!(id = %id, path = %path.display(), "Session loaded");
        Ok(Some((config, migration)))
    }

    /// 删除会话
    pub fn delete(&self, id: Uuid) -> anyhow::Result<()> {
        let path = self.session_path(id);

        if path.exists() {
            fs::remove_file(&path)?;
            debug!(id = %id, path = %path.display(), "Session deleted");
        } else {
            warn!(id = %id, "Session file not found, nothing to delete");
        }

        Ok(())
    }

    /// 列出所有会话
    pub fn list(&self) -> anyhow::Result<Vec<SessionConfig>> {
        Ok(self
            .list_ids()?
            .into_iter()
            .filter_map(|id| self.load(id).ok().flatten())
            .collect())
    }

    pub fn list_ids(&self) -> anyhow::Result<Vec<Uuid>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();

        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(id) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                {
                    sessions.push(id);
                }
            }
        }

        debug!(count = sessions.len(), "Listed sessions");
        Ok(sessions)
    }

    /// 获取默认存储路径
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rshell")
            .join("sessions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::{AuthMethod, Protocol};
    use tempfile::tempdir;

    fn test_session_config() -> SessionConfig {
        SessionConfig {
            id: Uuid::new_v4(),
            name: "Test Session".to_string(),
            folder_id: None,
            host: "192.168.1.1".to_string(),
            port: 22,
            protocol: Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "root".to_string(),
                has_password: true,
            },
            serial_config: None,
        }
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());

        let config = test_session_config();
        store.save(&config).unwrap();

        let loaded = store.load(config.id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, config.id);
        assert_eq!(loaded.name, config.name);
        assert_eq!(loaded.host, config.host);
        assert_eq!(loaded.port, config.port);
    }

    #[test]
    fn test_delete() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());

        let config = test_session_config();
        store.save(&config).unwrap();

        assert!(store.load(config.id).unwrap().is_some());

        store.delete(config.id).unwrap();
        assert!(store.load(config.id).unwrap().is_none());
    }

    #[test]
    fn test_list() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());

        let config1 = test_session_config();
        let config2 = test_session_config();

        store.save(&config1).unwrap();
        store.save(&config2).unwrap();

        let sessions = store.list().unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());

        let loaded = store.load(Uuid::new_v4()).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn replacement_does_not_modify_the_previous_inode() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut config = test_session_config();
        store.save(&config).unwrap();
        let path = store.session_path(config.id);
        let previous = dir.path().join("previous");
        fs::hard_link(&path, &previous).unwrap();
        let bytes = fs::read(&previous).unwrap();
        config.name = "Updated".into();
        store.save(&config).unwrap();
        assert_eq!(fs::read(previous).unwrap(), bytes);
        assert_eq!(store.load(config.id).unwrap().unwrap().name, "Updated");
    }

    #[test]
    fn invalid_metadata_errors_do_not_echo_source_values() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let id = Uuid::new_v4();
        fs::write(
            store.session_path(id),
            "password = 'sensitive-fixture' invalid = [",
        )
        .unwrap();
        let error = store.load(id).unwrap_err();
        assert!(!format!("{error:?}").contains("sensitive-fixture"));
    }

    #[test]
    fn failed_replacement_cleans_up_temporary_file() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let config = test_session_config();
        fs::create_dir(store.session_path(config.id)).unwrap();
        assert!(store.save(&config).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_password_metadata_is_not_treated_as_intentional_empty_auth() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let config = test_session_config();
        let content = toml::to_string(&config)
            .unwrap()
            .replace("has_password = true\n", "");
        fs::write(store.session_path(config.id), content).unwrap();
        assert!(store.load_pending(config.id).is_err());
    }
}
