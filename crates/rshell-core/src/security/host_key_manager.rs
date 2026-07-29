//! 主机密钥管理器
//!
//! 管理 known_hosts 文件，验证服务器身份。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Utc;
use tracing::{info, warn};

use rshell_api::types::{HostKeyEntry, TrustLevel};
use rshell_api::events::AppEvent;

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 主机密钥管理器
pub struct HostKeyManager {
    /// host:port -> HostKeyEntry
    entries: Arc<RwLock<HashMap<String, HostKeyEntry>>>,
    known_hosts_path: PathBuf,
    event_bus: Arc<EventBus>,
}

impl HostKeyManager {
    /// 创建新的主机密钥管理器
    pub fn new(known_hosts_path: PathBuf, event_bus: Arc<EventBus>) -> Self {
        let manager = Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            known_hosts_path,
            event_bus,
        };

        // 加载 known_hosts 文件
        let _ = manager.load_known_hosts();

        manager
    }

    /// 加载 known_hosts 文件
    fn load_known_hosts(&self) -> Result<(), CoreError> {
        if !self.known_hosts_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.known_hosts_path)
            .map_err(|e| CoreError::Internal(format!("Failed to read known_hosts: {}", e)))?;

        let mut entries = HashMap::new();
        let now = Utc::now().to_rfc3339();

        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // 格式: host:port key_type fingerprint
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let host_port: Vec<&str> = parts[0].split(':').collect();
                let host = host_port[0].to_string();
                let port = host_port.get(1).and_then(|p| p.parse().ok()).unwrap_or(22);

                let key = format!("{}:{}", host, port);
                entries.insert(key, HostKeyEntry {
                    host,
                    port,
                    key_type: parts[1].to_string(),
                    fingerprint: parts[2].to_string(),
                    trust_level: TrustLevel::Trusted,
                    first_seen: now.clone(),
                    last_seen: now.clone(),
                });
            }
        }

        // 使用 blocking 写入（在初始化时调用）
        let entries_clone = entries.clone();
        let rt = tokio::runtime::Handle::try_current();
        if rt.is_ok() {
            // 在异步上下文中，使用 spawn_blocking
            let entries_arc = Arc::new(self.entries.clone());
            let _ = std::thread::spawn(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    *entries_arc.write().await = entries_clone;
                });
            }).join();
        } else {
            // 不在异步上下文中，直接写入
            *self.entries.blocking_write() = entries;
        }

        info!("Loaded {} entries from known_hosts", self.entries.blocking_read().len());
        Ok(())
    }

    /// 保存 known_hosts 文件
    fn save_known_hosts(&self) -> Result<(), CoreError> {
        let entries = self.entries.blocking_read();
        let mut content = String::new();

        for entry in entries.values() {
            if entry.trust_level == TrustLevel::Trusted {
                content.push_str(&format!(
                    "{}:{} {} {}\n",
                    entry.host, entry.port, entry.key_type, entry.fingerprint
                ));
            }
        }

        std::fs::write(&self.known_hosts_path, content)
            .map_err(|e| CoreError::Internal(format!("Failed to write known_hosts: {}", e)))?;

        Ok(())
    }

    /// 检查主机密钥
    ///
    /// 返回:
    /// - Ok(None): 首次见到，需要用户确认
    /// - Ok(Some(true)): 匹配已知密钥
    /// - Ok(Some(false)): 密钥不匹配（可能中间人攻击）
    pub async fn check_host_key(
        &self,
        host: &str,
        port: u16,
        _key_type: &str,
        fingerprint: &str,
    ) -> Result<Option<bool>, CoreError> {
        let key = format!("{}:{}", host, port);
        let entries = self.entries.read().await;

        if let Some(entry) = entries.get(&key) {
            if entry.fingerprint == fingerprint {
                // 匹配
                info!("Host key matches: {}:{}", host, port);
                Ok(Some(true))
            } else if entry.trust_level == TrustLevel::Trusted {
                // 密钥不匹配
                warn!("Host key mismatch for {}:{}! Expected: {}, Got: {}",
                    host, port, entry.fingerprint, fingerprint);
                self.event_bus.publish(AppEvent::HostKeyMismatch {
                    host: host.to_string(),
                    expected: entry.fingerprint.clone(),
                    received: fingerprint.to_string(),
                });
                Ok(Some(false))
            } else {
                // 未知信任级别，需要确认
                Ok(None)
            }
        } else {
            // 首次见到
            info!("New host: {}:{} with fingerprint: {}", host, port, fingerprint);
            Ok(None)
        }
    }

    /// 信任主机密钥
    pub async fn trust_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
    ) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        let now = Utc::now().to_rfc3339();

        let entry = HostKeyEntry {
            host: host.to_string(),
            port,
            key_type: key_type.to_string(),
            fingerprint: fingerprint.to_string(),
            trust_level: TrustLevel::Trusted,
            first_seen: now.clone(),
            last_seen: now,
        };

        self.entries.write().await.insert(key, entry);
        self.save_known_hosts()?;

        info!("Host key trusted: {}:{}", host, port);
        Ok(())
    }

    /// 删除主机密钥
    pub async fn delete_host_key(&self, host: &str, port: u16) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        self.entries.write().await.remove(&key);
        self.save_known_hosts()?;

        info!("Host key deleted: {}:{}", host, port);
        Ok(())
    }

    /// 列出所有已知主机
    pub async fn list_hosts(&self) -> Vec<HostKeyEntry> {
        self.entries.read().await.values().cloned().collect()
    }

    /// 更新最后访问时间
    pub async fn update_last_seen(&self, host: &str, port: u16) {
        let key = format!("{}:{}", host, port);
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(&key) {
            entry.last_seen = Utc::now().to_rfc3339();
        }
    }
}
