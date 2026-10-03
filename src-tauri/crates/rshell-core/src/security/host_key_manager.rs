//! 主机密钥管理器
//!
//! 管理 known_hosts 文件，验证服务器身份。
//!
//! 文件格式遵循 OpenSSH 标准 known_hosts 规范：
//! ```text
//! <host_pattern> <keytype> <base64-key> [comment]
//! ```
//! 这样可以与系统 `ssh-keygen` 等工具互操作。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use rshell_api::types::{HostKeyEntry, TrustLevel};
use tracing::{info, warn};

use crate::error::CoreError;

/// 主机密钥管理器
pub struct HostKeyManager {
    /// host:port -> HostKeyEntry
    entries: Arc<RwLock<HashMap<String, HostKeyEntry>>>,
    known_hosts_path: PathBuf,
}

impl HostKeyManager {
    /// 创建新的主机密钥管理器
    pub fn new(known_hosts_path: PathBuf) -> Self {
        let manager = Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            known_hosts_path,
        };

        // 加载 known_hosts 文件
        let _ = manager.load_known_hosts();

        manager
    }

    /// 加载 known_hosts 文件（OpenSSH 标准格式）
    fn load_known_hosts(&self) -> Result<(), CoreError> {
        if !self.known_hosts_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.known_hosts_path)
            .map_err(|e| CoreError::Internal(format!("Failed to read known_hosts: {}", e)))?;

        let mut entries = HashMap::new();
        let now = Utc::now().to_rfc3339();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // OpenSSH known_hosts 行格式：
            //   <host_pattern>[,<host_pattern>...] <keytype> <base64-key> [comment]
            // hashed 条目：|1|base64(salt)|base64(hash) <keytype> <base64-key> ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let host_field = parts[0];
            // 跳过 hashed 条目（无法精确匹配 host 模式）
            if host_field.starts_with("|1|") {
                continue;
            }

            let key_type = parts[1].to_string();
            // key_blob 直接作为 fingerprint 存储（用于快速等价比较与显示）。
            // 真实的安全校验依赖基于 base64 内容的精确匹配，而非 SHA256 哈希。
            let fingerprint = parts[2].to_string();

            // 对 host_field 中的每个 host 模式，提取 host/port 并加入 entries
            for pattern in host_field.split(',') {
                let (host, port) = parse_host_port(pattern, 22);
                let map_key = format!("{}:{}", host, port);
                entries.insert(
                    map_key,
                    HostKeyEntry {
                        host,
                        port,
                        key_type: key_type.clone(),
                        fingerprint: fingerprint.clone(),
                        trust_level: TrustLevel::Trusted,
                        first_seen: now.clone(),
                        last_seen: now.clone(),
                    },
                );
            }
        }

        *self.entries.write().expect("host key lock poisoned") = entries;

        info!(
            "Loaded {} entries from known_hosts",
            self.entries.read().expect("host key lock poisoned").len()
        );
        Ok(())
    }

    /// 保存 known_hosts 文件（OpenSSH 标准格式）
    fn save_known_hosts(&self) -> Result<(), CoreError> {
        let entries = self.entries.read().expect("host key lock poisoned");
        let mut content = String::new();

        for entry in entries.values() {
            if entry.trust_level != TrustLevel::Trusted {
                continue;
            }
            // 如果 fingerprint 字段实际是 openssh 格式（ssh-xxx base64...）则原样写回；
            // 否则（旧的 SHA256:xxx 字符串）跳过——已不兼容新格式
            if entry.fingerprint.starts_with("SHA256:") {
                warn!(
                    host = %entry.host,
                    port = entry.port,
                    "Skipping legacy SHA256-fingerprint entry during rewrite"
                );
                continue;
            }
            let host_pattern = if entry.port == 22 {
                entry.host.clone()
            } else {
                format!("[{}]:{}", entry.host, entry.port)
            };
            content.push_str(&format!(
                "{} {} {}\n",
                host_pattern, entry.key_type, entry.fingerprint
            ));
        }

        if let Some(parent) = self.known_hosts_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&self.known_hosts_path, content)
            .map_err(|e| CoreError::Internal(format!("Failed to write known_hosts: {}", e)))?;
        Ok(())
    }

    // 已知条目但指纹不一致（可能中间人）由协议层 rshell-protocol::ssh::client
    // 的 verify_known_hosts → check_server_key 决策链处理，不再在此重复判定。

    /// 信任主机密钥
    ///
    /// `key_type` 应是 OpenSSH 算法名（如 `ssh-ed25519`、`rsa-sha2-256`）。
    /// `key_blob` 应是 base64 编码的公钥（无 keytype 前缀）。
    pub async fn trust_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        key_blob: &str,
    ) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        let now = Utc::now().to_rfc3339();

        let entry = HostKeyEntry {
            host: host.to_string(),
            port,
            key_type: key_type.to_string(),
            fingerprint: key_blob.to_string(),
            trust_level: TrustLevel::Trusted,
            first_seen: now.clone(),
            last_seen: now,
        };

        self.entries
            .write()
            .expect("host key lock poisoned")
            .insert(key, entry);
        self.save_known_hosts()?;

        info!("Host key trusted: {}:{}", host, port);
        Ok(())
    }

    /// 删除主机密钥
    pub async fn delete_host_key(&self, host: &str, port: u16) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        self.entries
            .write()
            .expect("host key lock poisoned")
            .remove(&key);
        self.save_known_hosts()?;
        info!("Host key deleted: {}:{}", host, port);
        Ok(())
    }

    /// 列出所有已知主机
    pub async fn list_hosts(&self) -> Vec<HostKeyEntry> {
        self.entries
            .read()
            .expect("host key lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 更新最后访问时间
    pub async fn update_last_seen(&self, host: &str, port: u16) {
        let key = format!("{}:{}", host, port);
        let mut entries = self.entries.write().expect("host key lock poisoned");
        if let Some(entry) = entries.get_mut(&key) {
            entry.last_seen = Utc::now().to_rfc3339();
        }
    }
}

/// 解析 OpenSSH host pattern 为 (host, port)
/// 支持 `[host]:port` / `host:port` / `host` 及裸 IPv6 字面量（如 `::1`）
fn parse_host_port(pattern: &str, default_port: u16) -> (String, u16) {
    let pattern = pattern.trim_end_matches(',');
    if let Some(idx) = pattern.find("]:") {
        let host = &pattern[..idx + 1];
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = pattern[idx + 2..].parse::<u16>().unwrap_or(default_port);
        return (host.to_string(), port);
    }
    // 裸 IPv6 字面量：盲切 rfind(':') 会得到错误 host/port。OpenSSH 对
    // 端口 22 的 IPv6 主机即写裸地址，按 default_port 处理（R2-06）
    if pattern.parse::<std::net::IpAddr>().is_ok() {
        return (pattern.to_string(), default_port);
    }
    if let Some(idx) = pattern.rfind(':') {
        let host = &pattern[..idx];
        let port = pattern[idx + 1..].parse::<u16>().unwrap_or(default_port);
        return (host.to_string(), port);
    }
    (pattern.to_string(), default_port)
}

/// 从 OpenSSH base64 公钥字符串计算 SHA256 指纹（用于显示）
///
/// 当前实现直接返回原 blob — `key_blob` 本身就是用于等价比较的稳定标识。
/// 显示用的人类可读指纹可由 UI 层根据 base64 blob 自行计算后展示。
fn _fingerprint_helper(key_blob: &str) -> String {
    let _ = key_blob;
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn permanent_trust_persists_across_manager_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let manager = HostKeyManager::new(path.clone());
        manager
            .trust_host_key("example.test", 2222, "ssh-ed25519", "AAAAkey")
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("[example.test]:2222 ssh-ed25519 AAAAkey"));

        // 重启后应从磁盘恢复条目（握手期的比对由协议层 verify_known_hosts 负责）
        let restored = HostKeyManager::new(path);
        let entries = restored.entries.read().unwrap();
        let entry = entries
            .get("example.test:2222")
            .expect("重启后条目应被加载");
        assert_eq!(entry.fingerprint, "AAAAkey");
        assert_eq!(entry.trust_level, TrustLevel::Trusted);
    }

    /// R2-06 验收：端口 22 的 IPv6 主机写裸地址（OpenSSH 格式），重启加载
    /// 时不得被 rfind(':') 误切成 host=":" port=1
    #[tokio::test]
    async fn ipv6_host_persists_as_bare_address_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let manager = HostKeyManager::new(path.clone());
        manager
            .trust_host_key("::1", 22, "ssh-ed25519", "AAAAkey6")
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains("::1 ssh-ed25519 AAAAkey6"),
            "端口 22 的 IPv6 主机必须写裸地址，实际：{}",
            on_disk
        );

        let restored = HostKeyManager::new(path);
        let entries = restored.entries.read().unwrap();
        let entry = entries
            .get("::1:22")
            .expect("裸 IPv6 条目应以 host=\"::1\" port=22 恢复");
        assert_eq!(entry.host, "::1");
        assert_eq!(entry.port, 22);
    }

    #[test]
    fn parse_host_port_handles_ipv6_literals() {
        // 裸 IPv6 按默认端口处理，不得切分冒号（回归：切出 host=":" port=1）
        assert_eq!(parse_host_port("::1", 22), ("::1".to_string(), 22));
        assert_eq!(parse_host_port("fe80::1", 22), ("fe80::1".to_string(), 22));
        // 方括号与 host:port 写法行为不变
        assert_eq!(parse_host_port("[::1]:2222", 22), ("::1".to_string(), 2222));
        assert_eq!(
            parse_host_port("example.test:2222", 22),
            ("example.test".to_string(), 2222)
        );
        assert_eq!(
            parse_host_port("example.test", 22),
            ("example.test".to_string(), 22)
        );
    }
}
