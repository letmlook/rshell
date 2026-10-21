//! 同步输入服务
//!
//! 将用户输入同时发送到多个会话。
//! 管理同步输入会话组。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SessionService;
use rshell_api::AppEvent;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};
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
        let mut sync = self
            .sync_sessions
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        *sync = session_ids.clone();

        info!(count = session_ids.len(), "Sync input sessions updated");
        self.event_bus
            .publish(AppEvent::SyncInputSessionsChanged { session_ids });
        Ok(())
    }

    /// 获取当前同步输入会话列表
    pub fn get_sync_sessions(&self) -> Result<Vec<Uuid>, CoreError> {
        let sync = self
            .sync_sessions
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(sync.clone())
    }

    /// 是否处于同步输入模式
    pub fn is_sync_active(&self) -> Result<bool, CoreError> {
        let sync = self
            .sync_sessions
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(!sync.is_empty())
    }

    /// 发送数据到所有同步输入会话
    ///
    /// `source_session_id` 是发起本次输入的会话：它**必须被排除**，否则源会话
    /// 自己在同步列表里时，输入会先由 `SendInput` 直接发一次、再由这里发第二次，
    /// 远端收到双份字节。
    ///
    /// 单个目标发送失败不阻断其余目标，但会为每个失败会话发布
    /// `SyncInputSendFailed` 事件——静默丢弃会让用户以为同步成功。
    pub async fn send_to_synced_sessions(
        &self,
        source_session_id: Uuid,
        data: &[u8],
        session_service: &SessionService,
    ) -> Result<(), CoreError> {
        let sessions = {
            let sync = self
                .sync_sessions
                .read()
                .map_err(|e| CoreError::Internal(e.to_string()))?;
            sync.clone()
        };

        for session_id in sync_targets(source_session_id, &sessions) {
            if let Err(e) = session_service.send_data(session_id, None, data).await {
                warn!(session_id = %session_id, error = %e, "Failed to send to synced session");
                self.event_bus.publish(AppEvent::SyncInputSendFailed {
                    session_id,
                    error: e.to_string(),
                });
            }
        }

        Ok(())
    }
}

/// 同步目标集合：排除源会话
///
/// 源会话的输入已由调用方（`SendInput`）直接发送过一次，若它也在同步列表里，
/// 会被这里再发一次——远端收到双份字节。过滤放在纯函数里，便于单测锁定。
fn sync_targets(source_session_id: Uuid, sessions: &[Uuid]) -> Vec<Uuid> {
    sessions
        .iter()
        .copied()
        .filter(|id| *id != source_session_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_svc() -> SyncInputService {
        SyncInputService::new(Arc::new(EventBus::new()))
    }

    #[test]
    fn test_toggle_sets_sessions() {
        let svc = make_svc();
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        svc.toggle_sync_input(vec![s1, s2]).unwrap();
        let got = svc.get_sync_sessions().unwrap();
        assert_eq!(got, vec![s1, s2]);
        assert!(svc.is_sync_active().unwrap());
    }

    #[test]
    fn test_toggle_empty_disables() {
        let svc = make_svc();
        svc.toggle_sync_input(vec![Uuid::new_v4()]).unwrap();
        svc.toggle_sync_input(vec![]).unwrap();
        assert!(!svc.is_sync_active().unwrap());
        assert!(svc.get_sync_sessions().unwrap().is_empty());
    }

    #[test]
    fn test_subscribe_publishes_change_event() {
        let bus = Arc::new(EventBus::new());
        let svc = SyncInputService::new(bus.clone());
        let got = Arc::new(std::sync::Mutex::new(0u32));
        let g = got.clone();
        bus.subscribe(move |event| {
            if matches!(event, AppEvent::SyncInputSessionsChanged { .. }) {
                *g.lock().unwrap() += 1;
            }
        });
        svc.toggle_sync_input(vec![Uuid::new_v4()]).unwrap();
        svc.toggle_sync_input(vec![]).unwrap();
        assert_eq!(*got.lock().unwrap(), 2);
    }

    /// 源会话在同步列表里时不能收到第二份输入。
    ///
    /// 此前 `send_to_synced_sessions` 把列表里的每个会话都发一遍，包括源会话
    /// 自己——而源会话的输入已经由 `SendInput` 直接发送过一次，远端因此收到
    /// 双份字节（回车会被执行两次）。
    #[test]
    fn source_session_is_excluded_from_sync_targets() {
        let source = Uuid::new_v4();
        let other = Uuid::new_v4();
        let targets = sync_targets(source, &[source, other]);
        assert_eq!(targets, vec![other]);
    }

    #[test]
    fn sync_targets_keeps_every_other_session() {
        let source = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let targets = sync_targets(source, &[source, a, b]);
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&a) && targets.contains(&b));
        assert!(!targets.contains(&source));
    }

    #[test]
    fn sync_targets_of_empty_group_is_empty() {
        assert!(sync_targets(Uuid::new_v4(), &[]).is_empty());
    }
}
