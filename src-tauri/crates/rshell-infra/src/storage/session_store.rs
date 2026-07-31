//! 会话存储
//!
//! 使用 TOML 文件格式持久化会话配置。
//! 每个会话存储为独立的 `.toml` 文件，位于配置目录下。

use rshell_api::types::SessionConfig;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};
use uuid::Uuid;

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
        let content = toml::to_string_pretty(session)?;
        fs::write(&path, content)?;

        debug!(id = %session.id, path = %path.display(), "Session saved");
        Ok(())
    }

    /// 加载会话
    pub fn load(&self, id: Uuid) -> anyhow::Result<Option<SessionConfig>> {
        let path = self.session_path(id);

        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)?;
        let config: SessionConfig = toml::from_str(&content)?;

        debug!(id = %id, path = %path.display(), "Session loaded");
        Ok(Some(config))
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
        if !self.dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();

        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                match fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<SessionConfig>(&content) {
                        Ok(config) => sessions.push(config),
                        Err(e) => {
                            warn!(path = %path.display(), error = %e, "Failed to parse session file");
                        }
                    },
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "Failed to read session file");
                    }
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
                password: "password123".to_string(),
            },
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
}
