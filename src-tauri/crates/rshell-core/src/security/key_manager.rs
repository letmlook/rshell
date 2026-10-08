//! SSH 密钥管理器
//!
//! 管理 SSH 密钥对的生成、导入、导出和删除。

use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use rshell_api::events::AppEvent;
use rshell_api::types::{SshKeyInfo, SshKeyType};
use rshell_infra::crypto::hash::sha256_fingerprint;

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 系统的密码学随机源。
///
/// ssh-key 0.7 依赖的 rand_core 0.10 移除了 `OsRng`，系统熵源改由
/// `getrandom::SysRng` 提供；它实现的是 `TryRng`（错误可能非 `Infallible`），
/// 用 `UnwrapErr` 包装后即为 `CryptoRng`，正好满足 `PrivateKey::random` /
/// `RsaKeypair::random` 的约束。
fn os_rng() -> getrandom::rand_core::UnwrapErr<getrandom::SysRng> {
    getrandom::rand_core::UnwrapErr(getrandom::SysRng)
}

/// 根据公钥算法推导密钥类型；RSA 按模数位数区分 2048/4096
fn ssh_key_type_of(public_key: &ssh_key::PublicKey) -> SshKeyType {
    match public_key.algorithm() {
        ssh_key::Algorithm::Ed25519 => SshKeyType::ED25519,
        ssh_key::Algorithm::Rsa { .. } => {
            // ssh-key 0.7 起 rsa::RsaPublicKey 的 `n` 字段私有，位数改用
            // 官方 `key_size()`（已按 Mpint 前导零规则算好），不再手工剥字节。
            let bits = match public_key.key_data() {
                ssh_key::public::KeyData::Rsa(rsa) => rsa.key_size() as usize,
                _ => 0,
            };
            if bits >= 4096 {
                SshKeyType::RSA4096
            } else {
                SshKeyType::RSA2048
            }
        }
        ssh_key::Algorithm::Ecdsa { curve } => match curve {
            ssh_key::EcdsaCurve::NistP256 => SshKeyType::ECDSA256,
            ssh_key::EcdsaCurve::NistP384 => SshKeyType::ECDSA384,
            ssh_key::EcdsaCurve::NistP521 => SshKeyType::ECDSA521,
        },
        _ => SshKeyType::ED25519,
    }
}

/// 归一化 passphrase 参数：None 或空串均视为未设置口令
fn effective_passphrase(passphrase: Option<&str>) -> Option<&str> {
    match passphrase {
        Some(p) if !p.is_empty() => Some(p),
        _ => None,
    }
}

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
    ///
    /// 生成/导入的私钥都会落盘到 keys_dir/{uuid}.key，构造时必须扫描目录
    /// 重建内存索引，否则重启后密钥从列表消失，磁盘上留下无法管理的孤儿文件。
    pub fn new(keys_dir: PathBuf, event_bus: Arc<EventBus>) -> Self {
        // 确保密钥目录存在
        let _ = std::fs::create_dir_all(&keys_dir);

        let keys = Self::load_keys_from_dir(&keys_dir);

        Self {
            keys: Arc::new(RwLock::new(keys)),
            keys_dir,
            event_bus,
        }
    }

    /// 扫描 keys_dir/*.key，从磁盘私钥文件重建内存索引
    ///
    /// 仅纳入文件名为 {uuid}.key 且可解析的私钥（口令加密的私钥同样可解析，
    /// has_passphrase 按实际加密状态如实标记）；无法识别的文件
    /// 保留在磁盘上并记录告警，不做静默删除。
    fn load_keys_from_dir(keys_dir: &std::path::Path) -> HashMap<Uuid, StoredSshKey> {
        let mut keys = HashMap::new();

        let entries = match std::fs::read_dir(keys_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read keys dir {:?}: {}", keys_dir, e);
                return keys;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("key") {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                warn!("Skipping key file with unreadable name: {:?}", path);
                continue;
            };
            let Ok(key_id) = Uuid::parse_str(stem) else {
                warn!("Skipping key file with non-UUID name: {:?}", path);
                continue;
            };

            let Ok(key_data) = std::fs::read(&path) else {
                warn!("Failed to read key file: {:?}", path);
                continue;
            };

            if let Some(stored) = Self::rebuild_stored_key(key_id, &key_data, &path) {
                info!(
                    "Restored SSH key from disk: id={}, fingerprint={}",
                    key_id, stored.fingerprint
                );
                keys.insert(key_id, stored);
            }
        }

        keys
    }

    /// 解析私钥数据，重建 StoredSshKey
    ///
    /// name 取自私钥注释（生成/导入时写入），无注释时退回文件名；
    /// 口令加密的私钥注释在密文内无法读取，name 一律退回文件名；
    /// created_at 取文件修改时间。
    fn rebuild_stored_key(
        key_id: Uuid,
        key_data: &[u8],
        path: &std::path::Path,
    ) -> Option<StoredSshKey> {
        let private_key = match ssh_key::PrivateKey::from_openssh(key_data) {
            Ok(k) => k,
            Err(e) => {
                warn!("Failed to parse key file {:?}: {}", path, e);
                return None;
            }
        };

        let public_key = private_key.public_key();
        let public_key_bytes = match public_key.to_bytes() {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!("Failed to encode public key from {:?}: {}", path, e);
                return None;
            }
        };
        let fingerprint = sha256_fingerprint(&public_key_bytes);
        let public_key_str = match public_key.to_openssh() {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to encode public key from {:?}: {}", path, e);
                return None;
            }
        };

        let comment = private_key.comment().to_string();
        let name = if comment.is_empty() {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string()
        } else {
            comment
        };
        let created_at = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .map(chrono::DateTime::<Utc>::from)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|_| Utc::now().to_rfc3339());

        Some(StoredSshKey {
            id: key_id,
            name,
            key_type: ssh_key_type_of(public_key),
            fingerprint,
            public_key_blob: public_key_str,
            private_key_data: key_data.to_vec(),
            comment: String::new(),
            // PROB-10：has_passphrase 如实反映密钥加密状态，不再恒为 false
            has_passphrase: private_key.is_encrypted(),
            created_at,
        })
    }

    /// 写入私钥文件；Unix 上创建时即以 0600 权限落盘，避免明文私钥窗口期可被其他用户读取
    async fn write_key_file(path: &std::path::Path, data: &[u8]) -> Result<(), CoreError> {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);

        let mut file = options.open(path).await.map_err(|e| {
            CoreError::Internal(format!("Failed to save key file {:?}: {}", path, e))
        })?;
        file.write_all(data).await.map_err(|e| {
            CoreError::Internal(format!("Failed to save key file {:?}: {}", path, e))
        })?;
        file.flush().await.map_err(|e| {
            CoreError::Internal(format!("Failed to save key file {:?}: {}", path, e))
        })?;
        // 创建时已按 0600 落盘；若覆盖写入既有文件，OpenOptions 的 mode 不生效，
        // 写入后统一收紧权限，确保私钥文件只对本用户可读（PROB-10）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|e| {
                    CoreError::Internal(format!(
                        "Failed to restrict key file permissions {:?}: {}",
                        path, e
                    ))
                })?;
        }
        Ok(())
    }

    /// 生成新的 SSH 密钥对
    ///
    /// 传入非空 passphrase 时，私钥以 OpenSSH 口令加密（bcrypt-pbkdf 派生 +
    /// AES256-CTR）落盘，`has_passphrase` 如实标记为 true；不静默忽略口令。
    pub async fn generate_key(
        &self,
        name: &str,
        key_type: SshKeyType,
        passphrase: Option<&str>,
    ) -> Result<SshKeyInfo, CoreError> {
        info!("Generating SSH key: name={}, type={:?}", name, key_type);

        // 使用 ssh-key crate 生成密钥
        let mut rng = os_rng();
        let mut private_key = match key_type {
            SshKeyType::ED25519 => {
                ssh_key::PrivateKey::random(&mut rng, ssh_key::Algorithm::Ed25519).map_err(|e| {
                    CoreError::Internal(format!("Failed to generate ED25519 key: {}", e))
                })?
            }
            SshKeyType::RSA2048 | SshKeyType::RSA4096 => {
                // R2-05：ssh-key 的 PrivateKey::random 对 RSA 固定使用
                // DEFAULT_RSA_KEY_SIZE(4096) 且无位数入参，两种请求都会生成
                // 4096 位密钥、重启 rebuild 后标签翻转。必须按请求类型用
                // RsaKeypair::random 以指定位数组装 KeypairData。
                let bit_size = if key_type == SshKeyType::RSA2048 {
                    2048
                } else {
                    4096
                };
                let rsa_keypair = ssh_key::private::RsaKeypair::random(&mut rng, bit_size)
                    .map_err(|e| CoreError::Internal(format!("Failed to generate RSA key: {e}")))?;
                ssh_key::PrivateKey::new(ssh_key::private::KeypairData::Rsa(rsa_keypair), "")
                    .map_err(|e| CoreError::Internal(format!("Failed to generate RSA key: {e}")))?
            }
            SshKeyType::ECDSA256 => ssh_key::PrivateKey::random(
                &mut rng,
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP256,
                },
            )
            .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {e}")))?,
            SshKeyType::ECDSA384 => ssh_key::PrivateKey::random(
                &mut rng,
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP384,
                },
            )
            .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {e}")))?,
            SshKeyType::ECDSA521 => ssh_key::PrivateKey::random(
                &mut rng,
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP521,
                },
            )
            .map_err(|e| CoreError::Internal(format!("Failed to generate ECDSA key: {e}")))?,
        };

        // 将密钥名写入注释：重启后从磁盘重建索引时可恢复名称
        private_key.set_comment(name);

        // 获取公钥
        let public_key = private_key.public_key();
        let public_key_blob = public_key
            .to_bytes()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        // 计算指纹
        let fingerprint = sha256_fingerprint(&public_key_blob);

        // 编码公钥为 OpenSSH 格式
        let public_key_str = public_key
            .to_openssh()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        // PROB-10：非空口令必须真实生效——先写注释再加密，注释随私钥段一起
        // 进入密文，解密后可恢复名称；公钥部分不受加密影响
        let has_passphrase = match effective_passphrase(passphrase) {
            Some(pass) => {
                private_key = private_key.encrypt(&mut os_rng(), pass).map_err(|e| {
                    CoreError::Internal(format!("Failed to encrypt private key: {}", e))
                })?;
                true
            }
            None => false,
        };

        // 编码私钥 - to_openssh 返回 Zeroizing<String>，需要转换为 bytes
        // （有口令时此处编码出的是加密 PEM，磁盘上不留明文私钥）
        let private_key_string = private_key
            .to_openssh(ssh_key::LineEnding::LF)
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
            has_passphrase,
            created_at: now.clone(),
        };

        // 保存到文件
        let key_file = self.keys_dir.join(format!("{}.key", id));
        Self::write_key_file(&key_file, &private_key_data).await?;

        // 存储到内存
        self.keys.write().await.insert(id, stored_key);

        let key_info = SshKeyInfo {
            id,
            name: name.to_string(),
            key_type,
            fingerprint,
            public_key_blob: public_key_str,
            comment: String::new(),
            has_passphrase,
            created_at: now,
        };

        // 发布事件（同步调用）
        self.event_bus.publish(AppEvent::SshKeyGenerated {
            key: key_info.clone(),
        });
        self.event_bus.publish(AppEvent::SshKeyListChanged);

        info!(
            "SSH key generated: id={}, fingerprint={}",
            id, key_info.fingerprint
        );
        Ok(key_info)
    }

    /// 导入私钥文件
    ///
    /// - 传入非空 passphrase 且源文件未加密：以该口令加密后再落盘，
    ///   `has_passphrase` 标记为 true，不静默忽略口令；
    /// - 传入非空 passphrase 且源文件已加密：先校验口令，错误口令显式报错；
    /// - 未传 passphrase：按源文件实际加密状态如实标记 `has_passphrase`。
    pub async fn import_private_key(
        &self,
        path: &std::path::Path,
        passphrase: Option<&str>,
    ) -> Result<SshKeyInfo, CoreError> {
        info!("Importing private key from: {:?}", path);

        let key_data = tokio::fs::read(path)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to read key file: {}", e)))?;

        // 尝试解码私钥
        let mut private_key = ssh_key::PrivateKey::from_openssh(key_data.as_slice())
            .map_err(|e| CoreError::Internal(format!("Failed to decode key: {}", e)))?;

        // 获取公钥信息
        let public_key = private_key.public_key();
        let public_key_blob = public_key
            .to_bytes()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;
        let fingerprint = sha256_fingerprint(&public_key_blob);
        let key_type = ssh_key_type_of(public_key);

        let public_key_str = public_key
            .to_openssh()
            .map_err(|e| CoreError::Internal(format!("Failed to encode public key: {}", e)))?;

        let id = Uuid::new_v4();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_string();
        let now = Utc::now().to_rfc3339();

        // 将名称写入注释后重新编码保存：重启后可从磁盘重建索引时恢复名称。
        // 注释在加密前写入：加密路径下注释随私钥段一起进入密文，解密后可恢复；
        // 源文件本已加密时，OpenSSH 信封不携带明文注释，重启后 name 退回文件名。
        private_key.set_comment(name.clone());

        // PROB-10：has_passphrase 必须如实反映加密状态，口令不被静默忽略——
        // 已加密密钥提供的口令先校验（错误口令显式报错），未加密密钥提供的
        // 口令则加密后再落盘
        let was_encrypted = private_key.is_encrypted();
        let has_passphrase = match effective_passphrase(passphrase) {
            Some(pass) if was_encrypted => {
                // 仅校验口令；仍按原加密形式落盘，避免无谓的重加密
                private_key.decrypt(pass).map_err(|_| {
                    CoreError::InvalidState(format!(
                        "Incorrect passphrase: cannot decrypt key from {:?}",
                        path
                    ))
                })?;
                true
            }
            Some(pass) => {
                private_key = private_key.encrypt(&mut os_rng(), pass).map_err(|e| {
                    CoreError::Internal(format!("Failed to encrypt private key: {}", e))
                })?;
                true
            }
            None => was_encrypted,
        };

        let stored_bytes = private_key
            .to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| CoreError::Internal(format!("Failed to encode key: {}", e)))?
            .to_string()
            .into_bytes();

        let stored_key = StoredSshKey {
            id,
            name: name.clone(),
            key_type,
            fingerprint: fingerprint.clone(),
            public_key_blob: public_key_str.clone(),
            private_key_data: stored_bytes.clone(),
            comment: String::new(),
            has_passphrase,
            created_at: now.clone(),
        };

        // 保存到密钥目录
        let key_file = self.keys_dir.join(format!("{}.key", id));
        Self::write_key_file(&key_file, &stored_bytes).await?;

        self.keys.write().await.insert(id, stored_key);

        let key_info = SshKeyInfo {
            id,
            name,
            key_type,
            fingerprint,
            public_key_blob: public_key_str,
            comment: String::new(),
            has_passphrase,
            created_at: now,
        };

        self.event_bus.publish(AppEvent::SshKeyGenerated {
            key: key_info.clone(),
        });
        self.event_bus.publish(AppEvent::SshKeyListChanged);

        info!(
            "SSH key imported: id={}, fingerprint={}",
            id, key_info.fingerprint
        );
        Ok(key_info)
    }

    /// 删除密钥（内存索引与磁盘文件同步清理）
    pub async fn delete_key(&self, key_id: Uuid) -> Result<(), CoreError> {
        info!("Deleting SSH key: {}", key_id);

        // 先删磁盘文件：失败则保留内存索引供重试，避免留下"内存无、磁盘有"的孤儿文件
        let key_file = self.keys_dir.join(format!("{}.key", key_id));
        match tokio::fs::remove_file(&key_file).await {
            Ok(()) => {}
            // 文件本就不存在（例如磁盘索引未覆盖）视为已删除
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::StorageError(format!(
                    "Failed to delete key file {:?}: {}",
                    key_file, e
                )));
            }
        }

        if self.keys.write().await.remove(&key_id).is_none() {
            warn!("Key not found in memory index: {}", key_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn new_manager(dir: &tempfile::TempDir) -> KeyManager {
        KeyManager::new(dir.path().to_path_buf(), Arc::new(EventBus::new()))
    }

    #[tokio::test]
    async fn generated_key_survives_manager_restart() {
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("work-key", SshKeyType::ED25519, None)
            .await
            .unwrap();
        assert!(dir.path().join(format!("{}.key", key.id)).exists());

        // 用同一 keys_dir 重建管理器，模拟应用重启：索引应从磁盘恢复
        let restarted = new_manager(&dir);
        let keys = restarted.list_keys().await;
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, key.id);
        assert_eq!(keys[0].fingerprint, key.fingerprint);
        assert_eq!(keys[0].name, "work-key");
    }

    #[tokio::test]
    async fn imported_key_survives_manager_restart() {
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("id_test");
        let generated =
            ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        let pem = generated.to_openssh(ssh_key::LineEnding::LF).unwrap();
        std::fs::write(&src, pem.as_bytes()).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let imported = manager.import_private_key(&src, None).await.unwrap();

        let restarted = new_manager(&dir);
        let keys = restarted.list_keys().await;
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].fingerprint, imported.fingerprint);
        assert_eq!(keys[0].name, "id_test");
    }

    #[tokio::test]
    async fn delete_removes_disk_file_and_memory_entry() {
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("doomed", SshKeyType::ED25519, None)
            .await
            .unwrap();

        manager.delete_key(key.id).await.unwrap();

        // 磁盘无 .key 残留，内存索引同步清空
        assert!(!dir.path().join(format!("{}.key", key.id)).exists());
        assert!(manager.list_keys().await.is_empty());

        // 重启后（索引来自磁盘）删除同样应清理磁盘文件
        let key = new_manager(&dir)
            .generate_key("doomed-again", SshKeyType::ED25519, None)
            .await
            .unwrap();
        let restarted = new_manager(&dir);
        restarted.delete_key(key.id).await.unwrap();
        assert!(!dir.path().join(format!("{}.key", key.id)).exists());
        assert!(restarted.list_keys().await.is_empty());
    }

    #[tokio::test]
    async fn unrecognized_files_are_skipped_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"junk").unwrap();
        std::fs::write(dir.path().join("stray.key"), b"not a valid private key").unwrap();

        let manager = new_manager(&dir);
        assert!(manager.list_keys().await.is_empty());
        // 无法识别的文件保留在磁盘上，不做静默删除
        assert!(dir.path().join("stray.key").exists());
    }

    #[tokio::test]
    async fn generated_key_with_passphrase_is_encrypted_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("secret-key", SshKeyType::ED25519, Some("hunter2"))
            .await
            .unwrap();
        assert!(key.has_passphrase);

        // PROB-10：传口令必须真实加密，磁盘上不留明文私钥，且口令可解密
        let bytes = std::fs::read(dir.path().join(format!("{}.key", key.id))).unwrap();
        let parsed = ssh_key::PrivateKey::from_openssh(bytes.as_slice()).unwrap();
        assert!(parsed.is_encrypted());
        let decrypted = parsed.decrypt("hunter2").unwrap();
        assert_eq!(
            sha256_fingerprint(&decrypted.public_key().to_bytes().unwrap()),
            key.fingerprint
        );
        assert!(parsed.decrypt("wrong-passphrase").is_err());

        // 重启后 has_passphrase 从磁盘按实际加密状态恢复
        let restarted = new_manager(&dir);
        let keys = restarted.list_keys().await;
        assert_eq!(keys.len(), 1);
        assert!(keys[0].has_passphrase);
    }

    #[tokio::test]
    async fn imported_unencrypted_key_with_passphrase_is_stored_encrypted() {
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("id_enc");
        let generated =
            ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        let pem = generated.to_openssh(ssh_key::LineEnding::LF).unwrap();
        std::fs::write(&src, pem.as_bytes()).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let imported = manager
            .import_private_key(&src, Some("key-pass"))
            .await
            .unwrap();
        assert!(imported.has_passphrase);

        // 口令不被静默忽略：落盘为加密 PEM，且口令可解密
        let bytes = std::fs::read(dir.path().join(format!("{}.key", imported.id))).unwrap();
        let parsed = ssh_key::PrivateKey::from_openssh(bytes.as_slice()).unwrap();
        assert!(parsed.is_encrypted());
        assert!(parsed.decrypt("key-pass").is_ok());

        let restarted = new_manager(&dir);
        let keys = restarted.list_keys().await;
        assert_eq!(keys.len(), 1);
        assert!(keys[0].has_passphrase);
    }

    #[tokio::test]
    async fn import_of_encrypted_key_rejects_wrong_passphrase() {
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("id_enc");
        let generated =
            ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        let encrypted = generated.encrypt(&mut os_rng(), "right-pass").unwrap();
        let pem = encrypted.to_openssh(ssh_key::LineEnding::LF).unwrap();
        std::fs::write(&src, pem.as_bytes()).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);

        // 错误口令显式报错，而不是静默入库
        let err = manager.import_private_key(&src, Some("wrong-pass")).await;
        assert!(
            matches!(err, Err(CoreError::InvalidState(_))),
            "expected explicit passphrase error, got {:?}",
            err
        );

        // 正确口令导入成功；不提供口令也导入成功，两种情况均如实标记
        let with_pass = manager.import_private_key(&src, Some("right-pass")).await;
        assert!(matches!(with_pass, Ok(ref info) if info.has_passphrase));
        let without_pass = manager.import_private_key(&src, None).await;
        assert!(matches!(without_pass, Ok(ref info) if info.has_passphrase));
    }

    #[tokio::test]
    async fn empty_passphrase_is_treated_as_no_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("plain-key", SshKeyType::ED25519, Some(""))
            .await
            .unwrap();
        assert!(!key.has_passphrase);

        let bytes = std::fs::read(dir.path().join(format!("{}.key", key.id))).unwrap();
        let parsed = ssh_key::PrivateKey::from_openssh(bytes.as_slice()).unwrap();
        assert!(!parsed.is_encrypted());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn key_file_is_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("perm-key", SshKeyType::ED25519, None)
            .await
            .unwrap();

        let mode = std::fs::metadata(dir.path().join(format!("{}.key", key.id)))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// R2-05 验收：RSA2048 请求必须真实生成 2048 位模数、key_type 标签保持
    /// RSA2048，重启 rebuild（ssh_key_type_of 按模数位归类）后标签不变。
    /// 回归时恒生成 4096 位密钥且重启后标签翻转为 RSA4096。
    #[tokio::test]
    async fn generated_rsa2048_key_has_2048_bit_modulus_and_stable_label() {
        let dir = tempfile::tempdir().unwrap();
        let manager = new_manager(&dir);
        let key = manager
            .generate_key("rsa2048", SshKeyType::RSA2048, None)
            .await
            .unwrap();
        assert_eq!(
            key.key_type,
            SshKeyType::RSA2048,
            "返回的 key_type 必须与请求一致"
        );

        // 私钥模数必须是 2048 位（回归：实际生成 4096 级密钥）。
        // Mpint 在最高位为 1 时带一个前导零字节（实测 2048 位模数计 257
        // 字节），所以这里用 ssh-key 的 `key_size()`，它已按同一规则算好，
        // 不再手工剥字节——后者正是回归当初写错的地方。
        let bytes = std::fs::read(dir.path().join(format!("{}.key", key.id))).unwrap();
        let parsed = ssh_key::PrivateKey::from_openssh(bytes.as_slice()).unwrap();
        let modulus_bits = match parsed.public_key().key_data() {
            ssh_key::public::KeyData::Rsa(rsa) => rsa.key_size() as usize,
            other => panic!("expected RSA key, got {:?}", other.algorithm()),
        };
        assert_eq!(modulus_bits, 2048, "RSA2048 请求必须生成 2048 位模数");

        // 重启 rebuild 后标签不变（修复前同一密钥会翻转为 RSA4096）
        let restarted = new_manager(&dir);
        let keys = restarted.list_keys().await;
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_type, SshKeyType::RSA2048);
    }
}
