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
    ///
    /// 目标会话**已经断开**（连接不存在）时，把它从同步组里移除：留着只会
    /// 让之后每一次按键都失败一次（真机上实测一个断开的会话让日志刷出上百条
    /// 失败），而界面上仍然显示「同步输入已开启」——用户完全看不出同步其实
    /// 已经失效。移除后界面会收到 `SyncInputSessionsChanged` 而更新。
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

        let mut dropped: Vec<Uuid> = Vec::new();
        for session_id in sync_targets(source_session_id, &sessions) {
            if let Err(e) = session_service.send_data(session_id, None, data).await {
                warn!(session_id = %session_id, error = %e, "Failed to send to synced session");
                self.event_bus.publish(AppEvent::SyncInputSendFailed {
                    session_id,
                    error: e.to_string(),
                });
                if should_drop_from_sync(&e) {
                    dropped.push(session_id);
                }
            }
        }

        if !dropped.is_empty() {
            self.remove_sessions(&dropped);
        }

        Ok(())
    }

    /// 把给定会话移出同步组，并发布变更事件（界面据此更新「当前同步 N 个会话」）
    fn remove_sessions(&self, sessions: &[Uuid]) {
        let remaining = match self.sync_sessions.write() {
            Ok(mut sync) => {
                sync.retain(|id| !sessions.contains(id));
                sync.clone()
            }
            Err(_) => return,
        };
        info!(
            dropped = ?sessions,
            remaining = remaining.len(),
            "Removed disconnected sessions from sync group"
        );
        self.event_bus.publish(AppEvent::SyncInputSessionsChanged {
            session_ids: remaining,
        });
    }
}

/// 这类失败意味着会话**已经断开**（连接不存在）—— 应当把它移出同步组。
///
/// 其它失败（超时、权限、临时 IO 错误）不代表会话没了，移出同步组会让用户
/// 莫名其妙地丢掉配置，因此只在连接确实不存在时才移。
fn should_drop_from_sync(error: &CoreError) -> bool {
    matches!(error, CoreError::NotFound(_))
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

    /// 断开的会话必须被移出同步组。
    ///
    /// 留着的话，之后每一次按键都会失败一次（真机上实测一个断开的会话让日志
    /// 刷出上百条失败），而界面仍显示「同步输入已开启」——用户看不出同步
    /// 其实已经失效。
    #[test]
    fn disconnected_sessions_are_removed_from_sync_group() {
        let svc = make_svc();
        let alive = Uuid::new_v4();
        let dead = Uuid::new_v4();
        svc.toggle_sync_input(vec![alive, dead]).unwrap();

        svc.remove_sessions(&[dead]);

        assert_eq!(
            svc.get_sync_sessions().unwrap(),
            vec![alive],
            "断开的会话必须被移出同步组"
        );
    }

    /// 移除后要发布变更事件，界面的「当前同步 N 个会话」才会跟着更新
    #[test]
    fn removing_sessions_publishes_change_event() {
        let bus = Arc::new(EventBus::new());
        let svc = SyncInputService::new(bus.clone());
        let alive = Uuid::new_v4();
        let dead = Uuid::new_v4();
        svc.toggle_sync_input(vec![alive, dead]).unwrap();

        let got = Arc::new(std::sync::Mutex::new(Vec::new()));
        let g = got.clone();
        bus.subscribe(move |event| {
            if let AppEvent::SyncInputSessionsChanged { session_ids } = event {
                *g.lock().unwrap() = session_ids.to_vec();
            }
        });

        svc.remove_sessions(&[dead]);
        assert_eq!(*got.lock().unwrap(), vec![alive]);
    }

    /// 只有「连接不存在」才把会话移出同步组
    ///
    /// 其它错误（超时、权限等）不代表会话没了 —— 移出同步组会让用户莫名其妙
    /// 丢掉配置。
    #[test]
    fn only_missing_connection_is_dropped_from_sync_group() {
        assert!(should_drop_from_sync(&CoreError::NotFound(
            "Connection x not found".into()
        )));
        assert!(!should_drop_from_sync(&CoreError::ConnectionError(
            "timeout".into()
        )));
        assert!(!should_drop_from_sync(&CoreError::Internal("boom".into())));
    }

    /// 移除不存在的会话不改变组，也不报错
    #[test]
    fn removing_unknown_session_keeps_group_intact() {
        let svc = make_svc();
        let alive = Uuid::new_v4();
        svc.toggle_sync_input(vec![alive]).unwrap();

        svc.remove_sessions(&[Uuid::new_v4()]);

        assert_eq!(svc.get_sync_sessions().unwrap(), vec![alive]);
    }
}
