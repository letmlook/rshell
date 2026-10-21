//! 主密码系统
//!
//! 使用 PBKDF2 派生密钥，AES-256-GCM 加密存储敏感数据。
//!
//! 主密码的语义边界（见 docs/08）：**只用于应用锁定与验证**，不是凭据保险库，
//! 也不加密 macOS 钥匙串里的会话凭据。
//!
//! 持久化：盐与加密后的验证令牌写在 `master_password.json`（0600），
//! **派生密钥从不落盘**——重启后状态为 `Locked`，必须由用户重新验证解锁。
//! 只在内存里保存状态会让「设置主密码」跨重启即失效，属于静默假成功。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use rshell_api::events::AppEvent;
use rshell_infra::crypto::aes;
use rshell_infra::crypto::hash;

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 持久化文件格式版本
const PERSIST_VERSION: u32 = 1;
/// PBKDF2 迭代次数
const DEFAULT_ITERATIONS: u32 = 100_000;
/// 验证令牌明文（加密后落盘，用于校验密码正确性）
const VERIFICATION_TOKEN: &[u8] = b"RShell_MasterPassword_Verification_Token";

/// 主密码状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterPasswordState {
    /// 未设置
    NotSet,
    /// 已设置但未验证
    Locked,
    /// 已验证（内存中持有派生密钥）
    Unlocked,
}

impl MasterPasswordState {
    /// 状态名（IPC 与持久化共用）
    pub fn as_str(&self) -> &'static str {
        match self {
            MasterPasswordState::NotSet => "NotSet",
            MasterPasswordState::Locked => "Locked",
            MasterPasswordState::Unlocked => "Unlocked",
        }
    }
}

/// 持久化文件内容
#[derive(Debug, Serialize, Deserialize)]
struct PersistedMasterPassword {
    version: u32,
    iterations: u32,
    /// hex 编码的盐
    salt: String,
    /// hex 编码的加密验证令牌
    token: String,
}

/// 主密码管理器
pub struct MasterPassword {
    state: Arc<RwLock<MasterPasswordState>>,
    /// 派生密钥（仅在 Unlocked 状态有效）
    derived_key: Arc<RwLock<Option<[u8; 32]>>>,
    /// 加密盐
    salt: Arc<RwLock<Option<[u8; 16]>>>,
    /// 加密的验证令牌
    encrypted_token: Arc<RwLock<Option<Vec<u8>>>>,
    /// PBKDF2 迭代次数
    iterations: u32,
    /// 持久化文件路径；None 表示不落盘（仅内存，供测试使用）
    persist_path: Option<PathBuf>,
    event_bus: Arc<EventBus>,
}

impl MasterPassword {
    /// 创建新的主密码管理器（不持久化）
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            state: Arc::new(RwLock::new(MasterPasswordState::NotSet)),
            derived_key: Arc::new(RwLock::new(None)),
            salt: Arc::new(RwLock::new(None)),
            encrypted_token: Arc::new(RwLock::new(None)),
            iterations: DEFAULT_ITERATIONS,
            persist_path: None,
            event_bus,
        }
    }

    /// 创建带持久化的主密码管理器
    ///
    /// 构造时读取既有文件：读到有效盐与令牌即进入 `Locked`（派生密钥不落盘，
    /// 必须重新验证）；文件缺失或损坏则保持 `NotSet` 并记 warn，绝不崩溃启动。
    pub fn with_path(event_bus: Arc<EventBus>, path: PathBuf) -> Self {
        let mut iterations = DEFAULT_ITERATIONS;
        let mut salt: Option<[u8; 16]> = None;
        let mut token: Option<Vec<u8>> = None;

        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<PersistedMasterPassword>(&content) {
                Ok(persisted) => {
                    if persisted.version != PERSIST_VERSION {
                        warn!(
                            version = persisted.version,
                            path = %path.display(),
                            "Unsupported master password file version; ignoring it"
                        );
                    } else {
                        match (
                            hash::hex_decode(&persisted.salt),
                            hash::hex_decode(&persisted.token),
                        ) {
                            (Ok(salt_bytes), Ok(token_bytes)) if salt_bytes.len() == 16 => {
                                let mut salt_array = [0u8; 16];
                                salt_array.copy_from_slice(&salt_bytes);
                                iterations = persisted.iterations.max(1);
                                salt = Some(salt_array);
                                token = Some(token_bytes);
                            }
                            _ => {
                                warn!(
                                    path = %path.display(),
                                    "Master password file is malformed; ignoring it"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse master password file; ignoring it"
                    );
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read master password file; ignoring it"
                );
            }
        }

        // 派生了盐与令牌即视为「已设置」，但派生密钥不在内存 → 必须锁定待验证
        let state = if salt.is_some() {
            MasterPasswordState::Locked
        } else {
            MasterPasswordState::NotSet
        };

        Self {
            state: Arc::new(RwLock::new(state)),
            derived_key: Arc::new(RwLock::new(None)),
            salt: Arc::new(RwLock::new(salt)),
            encrypted_token: Arc::new(RwLock::new(token)),
            iterations,
            persist_path: Some(path),
            event_bus,
        }
    }

    /// 设置主密码（首次设置）
    pub async fn setup(&self, password: &str) -> Result<(), CoreError> {
        let current_state = self.state.read().await.clone();
        if current_state != MasterPasswordState::NotSet {
            return Err(CoreError::InvalidState(
                "Master password already set".into(),
            ));
        }

        info!("Setting up master password");

        // 生成随机盐
        let salt = hash::generate_salt();

        // 派生密钥
        let derived = hash::derive_key(password, &salt, self.iterations);

        // 创建验证令牌（一个已知的明文）
        let encrypted_token = aes::encrypt(&derived, VERIFICATION_TOKEN)
            .map_err(|e| CoreError::Internal(format!("Failed to encrypt token: {}", e)))?;

        // 存储
        *self.salt.write().await = Some(salt);
        *self.encrypted_token.write().await = Some(encrypted_token);
        *self.derived_key.write().await = Some(derived);

        // 先落盘再改状态：写盘失败时不能报成功，否则重启后主密码凭空消失
        self.persist().await?;

        *self.state.write().await = MasterPasswordState::Unlocked;

        self.event_bus
            .publish(AppEvent::MasterPasswordChanged { is_set: true });

        info!("Master password set successfully");
        Ok(())
    }

    /// 验证主密码
    pub async fn verify(&self, password: &str) -> Result<bool, CoreError> {
        let state = self.state.read().await.clone();
        if state == MasterPasswordState::NotSet {
            return Err(CoreError::InvalidState("Master password not set".into()));
        }

        let salt = self.salt.read().await;
        let encrypted_token = self.encrypted_token.read().await;

        let (Some(salt), Some(encrypted_token)) = (salt.as_ref(), encrypted_token.as_ref()) else {
            return Err(CoreError::InvalidState(
                "Master password data missing".into(),
            ));
        };

        // 派生密钥
        let derived = hash::derive_key(password, salt, self.iterations);

        // 尝试解密验证令牌
        match aes::decrypt(&derived, encrypted_token) {
            Ok(decrypted) => {
                if decrypted == VERIFICATION_TOKEN {
                    *self.derived_key.write().await = Some(derived);
                    *self.state.write().await = MasterPasswordState::Unlocked;
                    info!("Master password verified successfully");
                    self.event_bus
                        .publish(AppEvent::MasterPasswordVerified { success: true });
                    Ok(true)
                } else {
                    warn!("Master password verification failed: wrong token");
                    self.event_bus
                        .publish(AppEvent::MasterPasswordVerified { success: false });
                    Ok(false)
                }
            }
            Err(_) => {
                warn!("Master password verification failed: decryption error");
                self.event_bus
                    .publish(AppEvent::MasterPasswordVerified { success: false });
                Ok(false)
            }
        }
    }

    /// 修改主密码
    pub async fn change_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), CoreError> {
        // 先验证旧密码
        if !self.verify(old_password).await? {
            return Err(CoreError::AuthenticationFailed(
                "Old password incorrect".into(),
            ));
        }

        info!("Changing master password");

        // 生成新盐
        let new_salt = hash::generate_salt();

        // 派生新密钥
        let new_derived = hash::derive_key(new_password, &new_salt, self.iterations);

        // 重新加密验证令牌
        let new_encrypted_token = aes::encrypt(&new_derived, VERIFICATION_TOKEN)
            .map_err(|e| CoreError::Internal(format!("Failed to encrypt token: {}", e)))?;

        // 更新存储
        *self.salt.write().await = Some(new_salt);
        *self.encrypted_token.write().await = Some(new_encrypted_token);
        *self.derived_key.write().await = Some(new_derived);

        // 先落盘再改状态：写盘失败时新密码没有真正生效，必须让调用方知道
        self.persist().await?;

        *self.state.write().await = MasterPasswordState::Unlocked;

        info!("Master password changed successfully");
        Ok(())
    }

    /// 停用主密码
    ///
    /// 必须提供**当前**密码：没有当前密码就能停用等于给锁留了一个绕过口。
    /// 停用后删除持久化文件并回到 `NotSet`。
    pub async fn disable(&self, password: &str) -> Result<(), CoreError> {
        if !self.verify(password).await? {
            return Err(CoreError::AuthenticationFailed(
                "Master password incorrect".into(),
            ));
        }

        self.remove_persisted()?;

        *self.salt.write().await = None;
        *self.encrypted_token.write().await = None;
        *self.derived_key.write().await = None;
        *self.state.write().await = MasterPasswordState::NotSet;

        self.event_bus
            .publish(AppEvent::MasterPasswordChanged { is_set: false });

        info!("Master password disabled");
        Ok(())
    }

    /// 获取当前状态
    pub async fn get_state(&self) -> MasterPasswordState {
        self.state.read().await.clone()
    }

    /// 是否已设置
    pub async fn is_set(&self) -> bool {
        *self.state.read().await != MasterPasswordState::NotSet
    }

    /// 是否已解锁
    pub async fn is_unlocked(&self) -> bool {
        *self.state.read().await == MasterPasswordState::Unlocked
    }

    // 派生密钥**只用于验证**：主密码的契约是「应用锁定/验证」，不是凭据保险库
    // （见 docs/08）。这里刻意不提供 encrypt/decrypt 入口——一旦存在却没有
    // 消费方，它就成了一个「看起来能加密凭据、实际没有」的假能力。

    /// 锁定（清除内存中的密钥）
    ///
    /// 锁定后必须重新验证才能再次解锁；持久化文件保持不变（主密码本身没变）。
    pub async fn lock(&self) {
        *self.derived_key.write().await = None;
        *self.state.write().await = MasterPasswordState::Locked;
        self.event_bus
            .publish(AppEvent::MasterPasswordChanged { is_set: true });
        info!("Master password locked");
    }

    // ===== 持久化 =====

    /// 把盐与加密令牌写入磁盘
    ///
    /// 用同目录临时文件 + rename 原子替换；Unix 上文件权限 0600。
    /// 未配置 `persist_path` 时是纯内存模式，直接成功。
    async fn persist(&self) -> Result<(), CoreError> {
        let Some(path) = self.persist_path.as_ref() else {
            return Ok(());
        };

        let salt = self
            .salt
            .read()
            .await
            .ok_or_else(|| CoreError::Internal("Master password salt missing".into()))?;
        let token = self
            .encrypted_token
            .read()
            .await
            .clone()
            .ok_or_else(|| CoreError::Internal("Master password token missing".into()))?;

        let payload = PersistedMasterPassword {
            version: PERSIST_VERSION,
            iterations: self.iterations,
            salt: hash::hex_encode(&salt),
            token: hash::hex_encode(&token),
        };
        let content = serde_json::to_string_pretty(&payload).map_err(|e| {
            CoreError::Internal(format!("Failed to serialize master password: {}", e))
        })?;

        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| {
            CoreError::Internal(format!("Failed to create master password directory: {}", e))
        })?;

        let mut temporary = tempfile::NamedTempFile::new_in(dir).map_err(|e| {
            CoreError::Internal(format!("Failed to create master password temp file: {}", e))
        })?;
        use std::io::Write;
        temporary.write_all(content.as_bytes()).map_err(|e| {
            CoreError::Internal(format!("Failed to write master password file: {}", e))
        })?;
        temporary.as_file().sync_all().map_err(|e| {
            CoreError::Internal(format!("Failed to flush master password file: {}", e))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| {
                    CoreError::Internal(format!(
                        "Failed to restrict master password file permissions: {}",
                        e
                    ))
                })?;
        }
        // 同目录 rename 是提交点：提交后不再返回失败，避免调用方回滚已落盘的状态
        temporary.persist(path).map_err(|e| {
            CoreError::Internal(format!(
                "Failed to commit master password file: {}",
                e.error
            ))
        })?;

        info!(path = %path.display(), "Master password persisted");
        Ok(())
    }

    /// 删除持久化文件（停用主密码）
    fn remove_persisted(&self) -> Result<(), CoreError> {
        let Some(path) = self.persist_path.as_ref() else {
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CoreError::Internal(format!(
                "Failed to remove master password file: {}",
                e
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bus() -> Arc<EventBus> {
        Arc::new(EventBus::new())
    }

    /// 设置主密码后必须落盘：重启（新实例读同一文件）时状态是 Locked，
    /// 且用同一个密码能验证通过。只在内存里保存状态会让「设置主密码」
    /// 跨重启即失效，属于静默假成功。
    #[tokio::test]
    async fn setup_persists_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");

        let first = MasterPassword::with_path(make_bus(), path.clone());
        assert_eq!(first.get_state().await, MasterPasswordState::NotSet);
        first.setup("correct horse").await.unwrap();
        assert_eq!(first.get_state().await, MasterPasswordState::Unlocked);
        assert!(path.exists(), "setup must write the persistence file");

        // 重启：换一个实例读同一文件
        let second = MasterPassword::with_path(make_bus(), path);
        assert_eq!(
            second.get_state().await,
            MasterPasswordState::Locked,
            "persisted master password must come back locked"
        );
        assert!(
            !second.is_unlocked().await,
            "derived key must never be restored from disk"
        );
        assert!(
            second.verify("correct horse").await.unwrap(),
            "the persisted password must still verify"
        );
        assert_eq!(second.get_state().await, MasterPasswordState::Unlocked);
    }

    /// 错误密码在重启后依然被拒绝。
    #[tokio::test]
    async fn wrong_password_still_fails_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");

        let first = MasterPassword::with_path(make_bus(), path.clone());
        first.setup("right").await.unwrap();

        let second = MasterPassword::with_path(make_bus(), path);
        assert!(!second.verify("wrong").await.unwrap());
        assert_eq!(second.get_state().await, MasterPasswordState::Locked);
    }

    /// 锁定必须清掉内存里的派生密钥，并要求重新验证。
    #[tokio::test]
    async fn lock_clears_derived_key_and_requires_verify() {
        let dir = tempfile::tempdir().unwrap();
        let mp = MasterPassword::with_path(make_bus(), dir.path().join("master_password.json"));
        mp.setup("secret").await.unwrap();
        assert!(mp.is_unlocked().await);

        mp.lock().await;
        assert_eq!(mp.get_state().await, MasterPasswordState::Locked);
        assert!(!mp.is_unlocked().await);

        assert!(mp.verify("secret").await.unwrap());
        assert!(mp.is_unlocked().await);
    }

    /// 停用主密码：删除持久化文件并回到 NotSet；错误密码不得停用。
    #[tokio::test]
    async fn disable_requires_current_password_and_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");
        let mp = MasterPassword::with_path(make_bus(), path.clone());
        mp.setup("secret").await.unwrap();
        assert!(path.exists());

        assert!(mp.disable("wrong").await.is_err());
        assert!(path.exists(), "failed disable must keep the file");
        assert_eq!(mp.get_state().await, MasterPasswordState::Unlocked);

        mp.disable("secret").await.unwrap();
        assert!(!path.exists(), "disable must remove the persistence file");
        assert_eq!(mp.get_state().await, MasterPasswordState::NotSet);
    }

    /// 损坏的持久化文件不得让应用崩溃：忽略它并保持 NotSet。
    #[tokio::test]
    async fn malformed_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");
        std::fs::write(&path, b"{not json").unwrap();

        let mp = MasterPassword::with_path(make_bus(), path);
        assert_eq!(mp.get_state().await, MasterPasswordState::NotSet);
    }

    /// 长度不对的盐同样按损坏处理。
    #[tokio::test]
    async fn malformed_salt_length_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");
        std::fs::write(
            &path,
            br#"{"version":1,"iterations":100000,"salt":"aabb","token":"ccdd"}"#,
        )
        .unwrap();

        let mp = MasterPassword::with_path(make_bus(), path);
        assert_eq!(mp.get_state().await, MasterPasswordState::NotSet);
    }

    /// 文件缺失时保持 NotSet（首次启动的正常路径）。
    #[tokio::test]
    async fn missing_file_stays_not_set() {
        let dir = tempfile::tempdir().unwrap();
        let mp = MasterPassword::with_path(make_bus(), dir.path().join("master_password.json"));
        assert_eq!(mp.get_state().await, MasterPasswordState::NotSet);
        assert!(!mp.is_set().await);
    }

    /// 持久化文件必须只对本用户可读。
    #[cfg(unix)]
    #[tokio::test]
    async fn persisted_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");
        let mp = MasterPassword::with_path(make_bus(), path.clone());
        mp.setup("secret").await.unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "master password file must be 0600");
    }

    /// 纯内存模式（无持久化路径）仍然可用，且不会写任何文件。
    #[tokio::test]
    async fn in_memory_mode_still_works() {
        let mp = MasterPassword::new(make_bus());
        mp.setup("secret").await.unwrap();
        assert_eq!(mp.get_state().await, MasterPasswordState::Unlocked);
        assert!(mp.verify("secret").await.unwrap());
    }

    /// 修改密码后旧密码失效、新密码生效，且重启后仍是新密码。
    #[tokio::test]
    async fn change_password_persists_new_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master_password.json");
        let mp = MasterPassword::with_path(make_bus(), path.clone());
        mp.setup("old").await.unwrap();
        mp.change_password("old", "new").await.unwrap();

        let restarted = MasterPassword::with_path(make_bus(), path);
        assert!(!restarted.verify("old").await.unwrap());
        assert!(restarted.verify("new").await.unwrap());
    }
}
