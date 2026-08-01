//! SSH 密钥管理器
//!
//! 管理 SSH 密钥对的生成、导入、导出和删除。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use chrono::Utc;
use tracing::{info, warn};

use rshell_api::types::{SshKeyInfo, SshKeyType};
use rshell_api::events::AppEvent;
use rshell_infra::crypto::hash::sha256_fingerprint;

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 存储的 SSH 密钥
#[derive(Debug, Clone)]
pub struct StoredSshKey {
    pub id: Uuid,
    pub name: String,
    pub key_type: SshKeyType,
    pub fingerprint: String,
    pub public_key_blob: String,
    pub private_key_data: Vec<u8>,
    pub comment: String,
    pub has_passphrase: bool,
    pub created_at: String,
}

/// SSH 密钥管理器
pub struct KeyManager {
    keys: Arc<RwLock<HashMap<Uuid, StoredSshKey>>>,
    keys_dir: PathBuf,
    event_bus: Arc<EventBus>,
}

impl KeyManager {
    /// 创建新的密钥管理器
    pub fn new(keys_dir: PathBuf, event_bus: Arc<EventBus>) -> Self {
        // 确保密钥目录存在
        let _ = std::fs::create_dir_all(&keys_dir);

        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            keys_dir,
            event_bus,
        }
    }

    /// 生成新的 SSH 密钥对
    pub async fn generate_key(
        &self,
        name: &str,
        key_type: SshKeyType,
        _passphrase: Option<&str>,
    ) -> Result<SshKeyInfo, CoreError> {
        info!("Generating SSH key: name={}, type={:?}", name, key_type);

        // 使用 ssh-key crate 生成密钥
        let private_key = match key_type {
            SshKeyType::ED25519 => {
                ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
                    .map_err(|e| CoreError::Internal(format!("Failed to generate ED25519 key: {}", e)))?
            }
            SshKeyType::RSA2048 | SshKeyType::RSA4096 => {
                ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Rsa { hash: Some(ssh_key::HashAlg::Sha256) })
                    .map_err(|e| CoreError::Internal(format!("Failed to generate RSA key: {}", e)))?
            }
            SshKeyType::ECDSA256 => {
                ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ecdsa { curve: ssh_key::EcdsaCurve::NistP256 })
                    .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {}", e)))?
            }
            SshKeyType::ECDSA384 => {
                ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ecdsa { curve: ssh_key::EcdsaCurve::NistP384 })
                    .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {}", e)))?
            }
            SshKeyType::ECDSA521 => {
                ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ecdsa { curve: ssh_key::EcdsaCurve::NistP521 })
                    .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {}", e)))?
            }
        };

        // 获取公钥
        let public_key = private_key.public_key();
        let public_key_blob = public_key.to_bytes()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        // 计算指纹
        let fingerprint = sha256_fingerprint(&public_key_blob);

        // 编码公钥为 OpenSSH 格式
        let public_key_str = public_key.to_openssh()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        // 编码私钥 - to_openssh 返回 Zeroizing<String>，需要转换为 bytes
        let private_key_string = private_key.to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| CoreError::Internal(format!("Failed to encode private key: {}", e)))?;
        let private_key_data = private_key_string.to_string().into_bytes();

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        let stored_key = StoredSshKey {
            id,
            name: name.to_string(),
            key_type,
            fingerprint: fingerprint.clone(),
            public_key_blob: public_key_str.clone(),
            private_key_data: private_key_data.clone(),
            comment: String::new(),
            has_passphrase: false,
            created_at: now.clone(),
        };

        // 保存到文件
        let key_file = self.keys_dir.join(format!("{}.key", id));
        tokio::fs::write(&key_file, &private_key_data)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to save key file: {}", e)))?;

        // 存储到内存
        self.keys.write().await.insert(id, stored_key);

        let key_info = SshKeyInfo {
            id,
            name: name.to_string(),
            key_type,
            fingerprint,
            public_key_blob: public_key_str,
            comment: String::new(),
            has_passphrase: false,
            created_at: now,
        };

        // 发布事件（同步调用）
        self.event_bus.publish(AppEvent::SshKeyGenerated { key: key_info.clone() });
        self.event_bus.publish(AppEvent::SshKeyListChanged);

        info!("SSH key generated: id={}, fingerprint={}", id, key_info.fingerprint);
        Ok(key_info)
    }

    /// 导入私钥文件
    pub async fn import_private_key(
        &self,
        path: &std::path::Path,
        _passphrase: Option<&str>,
    ) -> Result<SshKeyInfo, CoreError> {
        info!("Importing private key from: {:?}", path);

        let key_data = tokio::fs::read(path)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to read key file: {}", e)))?;

        // 尝试解码私钥
        let private_key = ssh_key::PrivateKey::from_openssh(key_data.as_slice())
            .map_err(|e| CoreError::Internal(format!("Failed to decode key: {}", e)))?;

        // 获取公钥信息
        let public_key = private_key.public_key();
        let public_key_blob = public_key.to_bytes()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;
        let fingerprint = sha256_fingerprint(&public_key_blob);

        let key_type = match public_key.algorithm() {
            ssh_key::Algorithm::Ed25519 => SshKeyType::ED25519,
            ssh_key::Algorithm::Rsa { .. } => SshKeyType::RSA4096,
            ssh_key::Algorithm::Ecdsa { curve } => match curve {
                ssh_key::EcdsaCurve::NistP256 => SshKeyType::ECDSA256,
                ssh_key::EcdsaCurve::NistP384 => SshKeyType::ECDSA384,
                ssh_key::EcdsaCurve::NistP521 => SshKeyType::ECDSA521,
            },
            _ => SshKeyType::ED25519,
        };

        let public_key_str = public_key.to_openssh()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        let id = Uuid::new_v4();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_string();
        let now = Utc::now().to_rfc3339();

        let stored_key = StoredSshKey {
            id,
            name: name.clone(),
            key_type,
            fingerprint: fingerprint.clone(),
            public_key_blob: public_key_str.clone(),
            private_key_data: key_data.clone(),
            comment: String::new(),
            has_passphrase: false,
            created_at: now.clone(),
        };

        // 保存到密钥目录
        let key_file = self.keys_dir.join(format!("{}.key", id));
        tokio::fs::write(&key_file, &key_data)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to save key file: {}", e)))?;

        self.keys.write().await.insert(id, stored_key);

        let key_info = SshKeyInfo {
            id,
            name,
            key_type,
            fingerprint,
            public_key_blob: public_key_str,
            comment: String::new(),
            has_passphrase: false,
            created_at: now,
        };

        self.event_bus.publish(AppEvent::SshKeyGenerated { key: key_info.clone() });
        self.event_bus.publish(AppEvent::SshKeyListChanged);

        info!("SSH key imported: id={}, fingerprint={}", id, key_info.fingerprint);
        Ok(key_info)
    }

    /// 删除密钥
    pub async fn delete_key(&self, key_id: Uuid) -> Result<(), CoreError> {
        info!("Deleting SSH key: {}", key_id);

        let key = self.keys.write().await.remove(&key_id);
        if let Some(_key) = key {
            // 删除文件
            let key_file = self.keys_dir.join(format!("{}.key", key_id));
            let _ = tokio::fs::remove_file(&key_file).await;
        } else {
            warn!("Key not found: {}", key_id);
        }

        self.event_bus.publish(AppEvent::SshKeyListChanged);
        Ok(())
    }

    /// 导出公钥（OpenSSH 格式）
    pub async fn export_public_key(&self, key_id: Uuid) -> Result<String, CoreError> {
        let keys = self.keys.read().await;
        let key = keys
            .get(&key_id)
            .ok_or_else(|| CoreError::NotFound(format!("Key not found: {}", key_id)))?;

        Ok(key.public_key_blob.clone())
    }

    /// 列出所有密钥
    pub async fn list_keys(&self) -> Vec<SshKeyInfo> {
        let keys = self.keys.read().await;
        keys.values()
            .map(|k| SshKeyInfo {
                id: k.id,
                name: k.name.clone(),
                key_type: k.key_type,
                fingerprint: k.fingerprint.clone(),
                public_key_blob: k.public_key_blob.clone(),
                comment: k.comment.clone(),
                has_passphrase: k.has_passphrase,
                created_at: k.created_at.clone(),
            })
            .collect()
    }

    /// 获取密钥私钥数据（用于 SSH 连接）
    pub async fn get_private_key_data(&self, key_id: Uuid) -> Result<Vec<u8>, CoreError> {
        let keys = self.keys.read().await;
        let key = keys
            .get(&key_id)
            .ok_or_else(|| CoreError::NotFound(format!("Key not found: {}", key_id)))?;

        Ok(key.private_key_data.clone())
    }
}
