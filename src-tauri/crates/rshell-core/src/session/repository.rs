//! 会话存储仓库
//!
//! 提供会配置的持久化操作接口。

use rshell_api::types::SessionConfig;
use rshell_infra::storage::session_store::SessionStore;
use std::path::PathBuf;
use uuid::Uuid;

/// 会话仓库
pub struct SessionRepository {
    store: SessionStore,
}

impl SessionRepository {
    /// 创建新的仓库
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: SessionStore::new(path),
        }
    }

    /// 使用默认路径创建仓库
    pub fn with_default_path() -> Self {
        Self {
            store: SessionStore::new(SessionStore::default_path()),
        }
    }

    /// 保存会话
    pub fn save(&self, session: &SessionConfig) -> anyhow::Result<()> {
        self.store.save(session)
    }

    /// 加载会话
    pub fn load(&self, id: Uuid) -> anyhow::Result<Option<SessionConfig>> {
        self.store.load(id)
    }

    /// 删除会话
    pub fn delete(&self, id: Uuid) -> anyhow::Result<()> {
        self.store.delete(id)
    }

    /// 列出所有会话
    pub fn list_all(&self) -> anyhow::Result<Vec<SessionConfig>> {
        self.store.list()
    }
}
