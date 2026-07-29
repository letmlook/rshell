//! 同步输入服务
//!
//! 将用户输入同时发送到多个会话。
//! 管理同步输入会话组。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SessionService;
use rshell_api::AppEvent;
use std::sync::{Arc, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

/// 同步输入服务
pub struct SyncInputService {
    /// 同步输入会话组
    sync_sessions: Arc<RwLock<Vec<Uuid>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
}

impl SyncInputService {
    /// 创建新的同步输入服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            sync_sessions: Arc::new(RwLock::new(Vec::new())),
            event_bus,
        }
    }

    /// 切换同步输入模式
    ///
    /// 设置需要同步输入的会话列表。空列表表示关闭同步输入。
    pub fn toggle_sync_input(&self, session_ids: Vec<Uuid>) -> Result<(), CoreError> {
        let mut sync = self.sync_sessions.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        *sync = session_ids.clone();

        info!(count = session_ids.len(), "Sync input sessions updated");
        self.event_bus.publish(AppEvent::SyncInputSessionsChanged { session_ids });
        Ok(())
    }

    /// 获取当前同步输入会话列表
    pub fn get_sync_sessions(&self) -> Result<Vec<Uuid>, CoreError> {
        let sync = self.sync_sessions.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(sync.clone())
    }

    /// 是否处于同步输入模式
    pub fn is_sync_active(&self) -> Result<bool, CoreError> {
        let sync = self.sync_sessions.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(!sync.is_empty())
    }

    /// 发送数据到所有同步输入会话
    pub async fn send_to_synced_sessions(
        &self,
        data: &[u8],
        session_service: &SessionService,
    ) -> Result<(), CoreError> {
        let sessions = {
            let sync = self.sync_sessions.read().map_err(|e| CoreError::Internal(e.to_string()))?;
            sync.clone()
        };

        for session_id in &sessions {
            if let Err(e) = session_service.send_data(*session_id, data).await {
                debug!(session_id = %session_id, error = %e, "Failed to send to synced session");
            }
        }

        Ok(())
    }
}
