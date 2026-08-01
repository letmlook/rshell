//! 主密码系统
//!
//! 使用 PBKDF2 派生密钥，AES-256-GCM 加密存储敏感数据。

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use rshell_api::events::AppEvent;
use rshell_infra::crypto::aes;
use rshell_infra::crypto::hash;

use crate::error::CoreError;
use crate::event_bus::EventBus;

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
    event_bus: Arc<EventBus>,
}

impl MasterPassword {
    /// 创建新的主密码管理器
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            state: Arc::new(RwLock::new(MasterPasswordState::NotSet)),
            derived_key: Arc::new(RwLock::new(None)),
            salt: Arc::new(RwLock::new(None)),
            encrypted_token: Arc::new(RwLock::new(None)),
            iterations: 100_000,
            event_bus,
        }
    }

    /// 设置主密码（首次设置）
    pub async fn setup(&self, password: &str) -> Result<(), CoreError> {
        let current_state = self.state.read().await.clone();
        if current_state != MasterPasswordState::NotSet {
            return Err(CoreError::InvalidState("Master password already set".into()));
        }

        info!("Setting up master password");

        // 生成随机盐
        let salt = hash::generate_salt();

        // 派生密钥
        let derived = hash::derive_key(password, &salt, self.iterations);

        // 创建验证令牌（一个已知的明文）
        let verification_token = b"RShell_MasterPassword_Verification_Token";
        let encrypted_token = aes::encrypt(&derived, verification_token)
            .map_err(|e| CoreError::Internal(format!("Failed to encrypt token: {}", e)))?;

        // 存储
        *self.salt.write().await = Some(salt);
        *self.encrypted_token.write().await = Some(encrypted_token);
        *self.derived_key.write().await = Some(derived);
        *self.state.write().await = MasterPasswordState::Unlocked;

        self.event_bus.publish(AppEvent::MasterPasswordChanged { is_set: true });

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
            return Err(CoreError::InvalidState("Master password data missing".into()));
        };

        // 派生密钥
        let derived = hash::derive_key(password, salt, self.iterations);

        // 尝试解密验证令牌
        match aes::decrypt(&derived, encrypted_token) {
            Ok(decrypted) => {
                if decrypted == b"RShell_MasterPassword_Verification_Token" {
                    *self.derived_key.write().await = Some(derived);
                    *self.state.write().await = MasterPasswordState::Unlocked;
                    info!("Master password verified successfully");
                    self.event_bus.publish(AppEvent::MasterPasswordVerified { success: true });
                    Ok(true)
                } else {
                    warn!("Master password verification failed: wrong token");
                    self.event_bus.publish(AppEvent::MasterPasswordVerified { success: false });
                    Ok(false)
                }
            }
            Err(_) => {
                warn!("Master password verification failed: decryption error");
                self.event_bus.publish(AppEvent::MasterPasswordVerified { success: false });
                Ok(false)
            }
        }
    }

    /// 修改主密码
    pub async fn change_password(&self, old_password: &str, new_password: &str) -> Result<(), CoreError> {
        // 先验证旧密码
        if !self.verify(old_password).await? {
            return Err(CoreError::AuthenticationFailed("Old password incorrect".into()));
        }

        info!("Changing master password");

        // 生成新盐
        let new_salt = hash::generate_salt();

        // 派生新密钥
        let new_derived = hash::derive_key(new_password, &new_salt, self.iterations);

        // 重新加密验证令牌
        let verification_token = b"RShell_MasterPassword_Verification_Token";
        let new_encrypted_token = aes::encrypt(&new_derived, verification_token)
            .map_err(|e| CoreError::Internal(format!("Failed to encrypt token: {}", e)))?;

        // 更新存储
        *self.salt.write().await = Some(new_salt);
        *self.encrypted_token.write().await = Some(new_encrypted_token);
        *self.derived_key.write().await = Some(new_derived);
        *self.state.write().await = MasterPasswordState::Unlocked;

        info!("Master password changed successfully");
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

    /// 加密数据（需要已解锁）
    pub async fn encrypt_data(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        if !self.is_unlocked().await {
            return Err(CoreError::AuthenticationFailed("Master password not unlocked".into()));
        }

        let key = self.derived_key.read().await;
        let key = key.as_ref().ok_or_else(|| CoreError::Internal("No derived key".into()))?;

        aes::encrypt(key, plaintext)
            .map_err(|e| CoreError::Internal(format!("Encryption failed: {}", e)))
    }

    /// 解密数据（需要已解锁）
    pub async fn decrypt_data(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CoreError> {
        if !self.is_unlocked().await {
            return Err(CoreError::AuthenticationFailed("Master password not unlocked".into()));
        }

        let key = self.derived_key.read().await;
        let key = key.as_ref().ok_or_else(|| CoreError::Internal("No derived key".into()))?;

        aes::decrypt(key, ciphertext)
            .map_err(|e| CoreError::Internal(format!("Decryption failed: {}", e)))
    }

    /// 锁定（清除内存中的密钥）
    pub async fn lock(&self) {
        *self.derived_key.write().await = None;
        *self.state.write().await = MasterPasswordState::Locked;
        info!("Master password locked");
    }
}
