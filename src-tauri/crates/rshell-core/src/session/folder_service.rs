//! 连接分组（文件夹）服务
//!
//! 管理分组的 CRUD 与持久化。
//!
//! 分组是「连接信息」这个可复用配方的一部分（见 docs/08 的术语定义）：
//! 一条连接通过 `SessionConfig::folder_id` 指向所属分组，`None` 表示「默认」。
//! 删除分组**不会**删除组内的连接——它们回到「默认」。
//!
//! 持久化用同目录临时文件 + rename 的原子提交；文件损坏时保留原文件不动，
//! 只记 warn，不让启动失败。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tracing::{info, warn};
use uuid::Uuid;

use rshell_api::events::AppEvent;
use rshell_api::types::Folder;

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 分组服务
pub struct FolderService {
    folders: Arc<RwLock<HashMap<Uuid, Folder>>>,
    event_bus: Arc<EventBus>,
    path: Option<PathBuf>,
    /// 文件读不出来时为 true：此时拒绝写入，避免覆盖用户原有的分组数据
    read_only: bool,
}

impl FolderService {
    /// 创建不持久化的分组服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            folders: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            path: None,
            read_only: false,
        }
    }

    /// 创建带持久化的分组服务
    pub fn with_path(event_bus: Arc<EventBus>, path: PathBuf) -> Self {
        let (folders, writable) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Vec<Folder>>(&bytes) {
                Ok(items) => (
                    items.into_iter().map(|item| (item.id, item)).collect(),
                    true,
                ),
                Err(error) => {
                    warn!(path = %path.display(), %error, "Folder file is invalid; leaving it untouched");
                    (HashMap::new(), false)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (HashMap::new(), true),
            Err(error) => {
                warn!(path = %path.display(), %error, "Could not read folders; leaving file untouched");
                (HashMap::new(), false)
            }
        };
        Self {
            folders: Arc::new(RwLock::new(folders)),
            event_bus,
            path: Some(path),
            read_only: !writable,
        }
    }

    /// 列出全部分组（按名称排序，界面直接用这个顺序）
    pub fn list_folders(&self) -> Result<Vec<Folder>, CoreError> {
        let folders = self
            .folders
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let mut items: Vec<Folder> = folders.values().cloned().collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(items)
    }

    /// 新建分组
    pub fn create_folder(&self, name: &str) -> Result<Uuid, CoreError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidState(
                "Folder name cannot be empty".into(),
            ));
        }
        if trimmed.chars().count() > 64 {
            return Err(CoreError::InvalidState(
                "Folder name is longer than 64 characters".into(),
            ));
        }

        let mut folders = self
            .folders
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        if folders
            .values()
            .any(|folder| folder.name.eq_ignore_ascii_case(trimmed))
        {
            return Err(CoreError::InvalidState(
                "A folder with the same name already exists".into(),
            ));
        }

        let id = Uuid::new_v4();
        let mut updated = folders.clone();
        updated.insert(
            id,
            Folder {
                id,
                name: trimmed.to_string(),
            },
        );
        self.persist(&updated)?;
        *folders = updated;
        drop(folders);

        info!(folder_id = %id, name = %trimmed, "Folder created");
        self.event_bus.publish(AppEvent::FolderListChanged);
        Ok(id)
    }

    /// 重命名分组
    pub fn rename_folder(&self, folder_id: Uuid, name: &str) -> Result<(), CoreError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidState(
                "Folder name cannot be empty".into(),
            ));
        }

        let mut folders = self
            .folders
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        if !folders.contains_key(&folder_id) {
            return Err(CoreError::NotFound(format!(
                "Folder {} not found",
                folder_id
            )));
        }
        if folders
            .values()
            .any(|folder| folder.id != folder_id && folder.name.eq_ignore_ascii_case(trimmed))
        {
            return Err(CoreError::InvalidState(
                "A folder with the same name already exists".into(),
            ));
        }

        let mut updated = folders.clone();
        if let Some(folder) = updated.get_mut(&folder_id) {
            folder.name = trimmed.to_string();
        }
        self.persist(&updated)?;
        *folders = updated;
        drop(folders);

        info!(folder_id = %folder_id, name = %trimmed, "Folder renamed");
        self.event_bus.publish(AppEvent::FolderListChanged);
        Ok(())
    }

    /// 删除分组（组内的连接由调用方把 `folder_id` 置回 `None`）
    pub fn delete_folder(&self, folder_id: Uuid) -> Result<(), CoreError> {
        let mut folders = self
            .folders
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        if !folders.contains_key(&folder_id) {
            return Err(CoreError::NotFound(format!(
                "Folder {} not found",
                folder_id
            )));
        }
        let mut updated = folders.clone();
        updated.remove(&folder_id);
        self.persist(&updated)?;
        *folders = updated;
        drop(folders);

        info!(folder_id = %folder_id, "Folder deleted");
        self.event_bus.publish(AppEvent::FolderListChanged);
        Ok(())
    }

    /// 分组是否存在（新建/修改连接时校验 `folder_id`）
    pub fn exists(&self, folder_id: Uuid) -> Result<bool, CoreError> {
        let folders = self
            .folders
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(folders.contains_key(&folder_id))
    }

    fn persist(&self, folders: &HashMap<Uuid, Folder>) -> Result<(), CoreError> {
        if self.read_only {
            return Err(CoreError::StorageError(
                "Folder file could not be read; preserving it without changes".into(),
            ));
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut items: Vec<Folder> = folders.values().cloned().collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let bytes = serde_json::to_vec_pretty(&items)
            .map_err(|e| CoreError::StorageError(e.to_string()))?;
        crate::storage::persist_json(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc() -> FolderService {
        FolderService::new(Arc::new(EventBus::new()))
    }

    #[test]
    fn creates_and_lists_folders() {
        let svc = make_svc();
        let id = svc.create_folder("生产").unwrap();
        let folders = svc.list_folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id, id);
        assert_eq!(folders[0].name, "生产");
    }

    #[test]
    fn rejects_empty_name() {
        let svc = make_svc();
        assert!(svc.create_folder("   ").is_err());
    }

    #[test]
    fn rejects_duplicate_name() {
        let svc = make_svc();
        svc.create_folder("生产").unwrap();
        assert!(svc.create_folder("生产").is_err());
        // 大小写不同也算重复
        svc.create_folder("prod").unwrap();
        assert!(svc.create_folder("PROD").is_err());
    }

    #[test]
    fn renames_folder() {
        let svc = make_svc();
        let id = svc.create_folder("旧名").unwrap();
        svc.rename_folder(id, "新名").unwrap();
        assert_eq!(svc.list_folders().unwrap()[0].name, "新名");
    }

    #[test]
    fn rename_unknown_folder_fails() {
        let svc = make_svc();
        assert!(svc.rename_folder(Uuid::new_v4(), "x").is_err());
    }

    #[test]
    fn deletes_folder() {
        let svc = make_svc();
        let id = svc.create_folder("临时").unwrap();
        svc.delete_folder(id).unwrap();
        assert!(svc.list_folders().unwrap().is_empty());
        assert!(!svc.exists(id).unwrap());
    }

    #[test]
    fn delete_unknown_folder_fails() {
        let svc = make_svc();
        assert!(svc.delete_folder(Uuid::new_v4()).is_err());
    }

    #[test]
    fn persists_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("folders.json");

        let first = FolderService::with_path(Arc::new(EventBus::new()), path.clone());
        let id = first.create_folder("生产").unwrap();
        assert!(path.exists());

        let second = FolderService::with_path(Arc::new(EventBus::new()), path);
        let folders = second.list_folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id, id);
    }

    #[test]
    fn malformed_file_is_ignored_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("folders.json");
        std::fs::write(&path, b"{not json").unwrap();

        let svc = FolderService::with_path(Arc::new(EventBus::new()), path.clone());
        assert!(svc.list_folders().unwrap().is_empty());
        // 损坏的文件必须原样保留，不能被后续写入覆盖
        assert!(svc.create_folder("新").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }
}
