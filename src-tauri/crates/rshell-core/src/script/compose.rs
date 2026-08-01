//! 撰写窗格服务
//!
//! 多行文本编辑并批量发送到目标会话。
//! 支持发送到当前会话、所有会话或选中会话。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SessionService;
use rshell_api::types::ComposeTarget;
use tracing::{debug, info};
use uuid::Uuid;

/// 撰写窗格服务
pub struct ComposeService {
    /// 事件总线
    event_bus: std::sync::Arc<EventBus>,
}

impl ComposeService {
    /// 创建新的撰写窗格服务
    pub fn new(event_bus: std::sync::Arc<EventBus>) -> Self {
        Self { event_bus }
    }

    /// 发送文本到目标会话
    pub async fn send_text(
        &self,
        content: &str,
        target: &ComposeTarget,
        session_service: &SessionService,
        active_session: Option<Uuid>,
    ) -> Result<(), CoreError> {
        let target_sessions = match target {
            ComposeTarget::CurrentSession => {
                if let Some(sid) = active_session {
                    vec![sid]
                } else {
                    return Err(CoreError::Internal("No active session".to_string()));
                }
            }
            ComposeTarget::AllSessions => {
                let sessions = session_service.list_sessions().await?;
                sessions.iter().map(|s| s.id).collect()
            }
            ComposeTarget::SelectedSessions(ids) => ids.clone(),
        };

        info!(
            target_sessions = target_sessions.len(),
            content_len = content.len(),
            "Sending compose text"
        );

        let data = content.as_bytes().to_vec();
        for session_id in &target_sessions {
            if let Err(e) = session_service.send_data(*session_id, &data).await {
                debug!(session_id = %session_id, error = %e, "Failed to send to session");
                // 继续发送到其他会话
            }
        }

        Ok(())
    }

    /// 获取事件总线引用
    pub fn event_bus(&self) -> &std::sync::Arc<EventBus> {
        &self.event_bus
    }
}
