//! 撰写窗格服务
//!
//! 多行文本编辑并批量发送到目标会话。
//! 支持发送到当前会话、所有会话或选中会话。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SessionService;
use rshell_api::types::ComposeTarget;
use rshell_api::AppEvent;
use tracing::{info, warn};
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

        // 空目标列表（如所有会话均已关闭）与"全部失败"同属静默成功,必须报错
        if target_sessions.is_empty() {
            return Err(CoreError::InvalidState(
                "Compose target has no sessions".to_string(),
            ));
        }

        info!(
            target_sessions = target_sessions.len(),
            content_len = content.len(),
            "Sending compose text"
        );

        let data = content.as_bytes().to_vec();
        let mut failed: Vec<(Uuid, CoreError)> = Vec::new();
        for session_id in &target_sessions {
            if let Err(e) = session_service.send_data(*session_id, None, &data).await {
                warn!(session_id = %session_id, error = %e, "Failed to send compose text to session");
                failed.push((*session_id, e));
                // 继续发送到其他会话
            }
        }

        // 全部失败：返回聚合错误,让 invoke 直接报错给前端,而不是静默成功
        if failed.len() == target_sessions.len() {
            let detail = failed
                .iter()
                .map(|(session_id, error)| format!("{}: {}", session_id, error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(CoreError::ConnectionError(format!(
                "Compose send failed for all {} target session(s): {}",
                target_sessions.len(),
                detail
            )));
        }

        // 部分失败：invoke 仍算成功,但每个失败会话发布一个事件供 UI 提示
        if !failed.is_empty() {
            warn!(
                failed = failed.len(),
                total = target_sessions.len(),
                "Compose send partially failed"
            );
            for (session_id, error) in &failed {
                self.event_bus.publish(AppEvent::ComposeSendFailed {
                    session_id: *session_id,
                    error: error.to_string(),
                });
            }
            info!(
                sent = target_sessions.len() - failed.len(),
                total = target_sessions.len(),
                "Compose text sent with partial failures"
            );
        }

        Ok(())
    }

    /// 获取事件总线引用
    pub fn event_bus(&self) -> &std::sync::Arc<EventBus> {
        &self.event_bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::trigger_engine::TriggerEngine;
    use crate::security::host_key_decision::HostKeyDecisionRegistry;
    use crate::terminal::service::TerminalService;
    use rshell_api::types::{AuthMethod, Protocol, SessionConfig};
    use std::sync::{Arc, Mutex};

    /// 与 session 模块测试同款的最小 SessionService（无持久化仓库）。
    fn make_session_service(bus: Arc<EventBus>) -> SessionService {
        let terminal = Arc::new(TerminalService::new(bus.clone()));
        let triggers = Arc::new(TriggerEngine::new(bus.clone()));
        let host_keys = Arc::new(HostKeyDecisionRegistry::new(bus.clone()));
        SessionService::new(bus, terminal, triggers, host_keys)
    }

    /// PROB-17:目标会话全部未连接时必须返回聚合错误,不得静默返回 Ok,
    /// 且全部失败走 invoke 错误路径,不发布部分失败事件。
    #[tokio::test]
    async fn all_targets_unreachable_returns_aggregated_error() {
        let bus = Arc::new(EventBus::new());
        let failure_events: Arc<Mutex<Vec<AppEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = failure_events.clone();
        bus.subscribe(move |event| {
            if matches!(event, AppEvent::ComposeSendFailed { .. }) {
                sink.lock().unwrap().push(event.clone());
            }
        });

        let session_service = make_session_service(bus.clone());
        let compose = ComposeService::new(bus);

        // 两个从未创建的会话 ID:send_data 均返回 NotFound
        let missing = [Uuid::new_v4(), Uuid::new_v4()];
        let err = compose
            .send_text(
                "hello",
                &ComposeTarget::SelectedSessions(missing.to_vec()),
                &session_service,
                None,
            )
            .await
            .expect_err("all targets unreachable must fail");

        // 聚合错误必须点名每一个失败会话
        let message = err.to_string();
        for id in missing {
            assert!(
                message.contains(&id.to_string()),
                "aggregated error should mention {}: {}",
                id,
                message
            );
        }
        assert!(failure_events.lock().unwrap().is_empty());
    }

    /// PROB-17:空目标列表同样不得静默成功。
    #[tokio::test]
    async fn empty_target_list_returns_error() {
        let bus = Arc::new(EventBus::new());
        let session_service = make_session_service(bus.clone());
        let compose = ComposeService::new(bus);

        assert!(compose
            .send_text(
                "hello",
                &ComposeTarget::SelectedSessions(vec![]),
                &session_service,
                None
            )
            .await
            .is_err());
    }

    /// PROB-17:部分成功时 invoke 仍成功,但必须为每个失败会话发布
    /// ComposeSendFailed 事件供 UI 提示。可达会话用本地 Telnet 假服务器
    /// 模拟（与 session 模块测试同一套路）。
    #[tokio::test]
    async fn partial_success_publishes_failure_event_for_unreachable_target() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // 客户端 connect 阶段固定发送 6 字节协商:IAC DO SGA + IAC DO NAWS
            let mut negotiation = [0u8; 6];
            socket.read_exact(&mut negotiation).await.unwrap();
            let mut received = vec![0u8; 5];
            socket.read_exact(&mut received).await.unwrap();
            received
        });

        let bus = Arc::new(EventBus::new());
        let failure_events: Arc<Mutex<Vec<(Uuid, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = failure_events.clone();
        bus.subscribe(move |event| {
            if let AppEvent::ComposeSendFailed { session_id, error } = event {
                sink.lock().unwrap().push((*session_id, error.clone()));
            }
        });

        let session_service = make_session_service(bus.clone());
        let config = SessionConfig {
            id: Uuid::new_v4(),
            name: "compose-telnet".to_string(),
            folder_id: None,
            host: "127.0.0.1".to_string(),
            port,
            protocol: Protocol::Telnet,
            auth_method: AuthMethod::Password {
                username: "user".to_string(),
                has_password: true,
            },
            serial_config: None,
        };
        let reachable = session_service.create_session(config).await.unwrap();
        session_service.connect(reachable).await.unwrap();

        let unreachable = Uuid::new_v4();
        let compose = ComposeService::new(bus);
        compose
            .send_text(
                "hello",
                &ComposeTarget::SelectedSessions(vec![reachable, unreachable]),
                &session_service,
                None,
            )
            .await
            .expect("partial success must not fail the invoke");

        // 可达会话真的收到了文本,且只为不可达会话发布了一条失败事件
        assert_eq!(server.await.unwrap(), b"hello");
        let failed = failure_events.lock().unwrap().clone();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, unreachable);
        assert!(!failed[0].1.is_empty());
    }
}
