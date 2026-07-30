//! 会话服务

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::terminal::service::TerminalService;
use rshell_api::types::{ConnectionInfo, ConnectionState, RemoteFileEntry, SessionConfig};
use rshell_protocol::ssh::SshClient;
use rshell_protocol::ssh::sftp::SftpClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

/// 活动连接的 SSH 客户端句柄别名（与 SshClient 内部使用了同样的 tokio RwLock）
pub type SshClientHandle = Arc<tokio::sync::RwLock<SshClient>>;

/// 会话运行时状态
struct SessionState {
    config: SessionConfig,
    connection_state: ConnectionState,
    /// 连接信息（连接成功后填充）
    connection_info: Option<ConnectionInfo>,
}

/// 活动连接（使用 channel 来接收数据）
struct ActiveConnection {
    /// 用于发送数据到远程 shell
    client: SshClientHandle,
    /// 用于取消后台读取任务的通道
    _cancel_tx: mpsc::Sender<()>,
}

/// 会话服务 - 管理会话的生命周期
pub struct SessionService {
    /// 会话状态映射
    sessions: Arc<RwLock<HashMap<Uuid, SessionState>>>,
    /// 活动连接映射（tokio RwLock，因为持锁等待期需要跨 await）
    connections: Arc<RwLock<HashMap<Uuid, ActiveConnection>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
    /// 终端服务（用于在收到远端输出时解析 VT 并推回 snapshot）
    terminal_service: Arc<TerminalService>,
}

impl SessionService {
    /// 创建新的会话服务
    pub fn new(event_bus: Arc<EventBus>, terminal_service: Arc<TerminalService>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            terminal_service,
        }
    }

    /// 连接到会话
    #[instrument(skip(self))]
    pub async fn connect(&self, session_id: Uuid) -> Result<(), CoreError> {
        info!(session_id = %session_id, "Connecting session");

        // 获取会话配置
        let config = {
            let sessions = self.sessions.read().await;
            sessions
                .get(&session_id)
                .ok_or_else(|| CoreError::NotFound(format!("Session {} not found", session_id)))?
                .config
                .clone()
        };

        // 更新状态为 Connecting
        {
            let mut sessions = self.sessions.write().await;
            if let Some(state) = sessions.get_mut(&session_id) {
                state.connection_state = ConnectionState::Connecting;
            }
        }

        // 发布状态变化事件
        self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Connecting,
            info: None,
        });

        // 创建 SSH 客户端并连接
        //
        // 后续工作(TODO 下一轮):把 `_decision_tx` 留作 `SessionService::connect` 返回
        // 类型的一部分(连同会话 ID),让 `CommandDispatcher::Connect` 分支能 await 用户
        // 在 UI 上的"信任/拒绝"决定后 send。当下用户若触发未知 host key,SshHandler
        // 的 oneshot rx 接收方将收到 `Err` (channel close),连接被保守拒绝。
        let mut client = SshClient::new(config.clone());

        match client.connect_ssh().await {
            Ok(_decision_tx) => {
                info!(session_id = %session_id, "SSH connection established");

                // 创建取消通道
                let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                // 包装客户端为 Arc<RwLock>
                let client = Arc::new(tokio::sync::RwLock::new(client));

                // 保存活动连接
                {
                    let mut connections = self.connections.write().await;
                    connections.insert(session_id, ActiveConnection {
                        client: client.clone(),
                        _cancel_tx: cancel_tx,
                    });
                }

                // 更新状态为 Connected
                {
                    let mut sessions = self.sessions.write().await;
                    if let Some(state) = sessions.get_mut(&session_id) {
                        state.connection_state = ConnectionState::Connected;
                        state.connection_info = Some(ConnectionInfo {
                            protocol: config.protocol,
                            host: config.host.clone(),
                            port: config.port,
                            state: ConnectionState::Connected,
                            bytes_sent: 0,
                            bytes_received: 0,
                            latency_ms: None,
                        });
                    }
                }

                // 发布连接成功事件
                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: ConnectionState::Connected,
                    info: Some(ConnectionInfo {
                        protocol: config.protocol,
                        host: config.host.clone(),
                        port: config.port,
                        state: ConnectionState::Connected,
                        bytes_sent: 0,
                        bytes_received: 0,
                        latency_ms: None,
                    }),
                });

                // 启动后台数据读取任务
                let event_bus = self.event_bus.clone();
                let terminal_service = self.terminal_service.clone();
                let client_clone = client.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = cancel_rx.recv() => {
                                debug!(session_id = %session_id, "Data reader cancelled");
                                break;
                            }
                            result = async {
                                let mut client = client_clone.write().await;
                                client.recv_data().await
                            } => {
                                match result {
                                    Ok(Some(data)) => {
                                        event_bus.publish(rshell_api::AppEvent::TerminalOutput {
                                            session_id,
                                            data: data.clone(),
                                        });
                                        if let Err(e) = terminal_service.process_output(session_id, &data) {
                                            warn!(session_id = %session_id, error = %e, "process_output failed");
                                        }
                                        if let Ok(snapshot) = terminal_service.get_buffer_snapshot(session_id) {
                                            event_bus.publish(rshell_api::AppEvent::TerminalBufferUpdated {
                                                session_id,
                                                snapshot,
                                            });
                                        }
                                    }
                                    Ok(None) => {
                                        debug!(session_id = %session_id, "recv_data stream ended");
                                        break;
                                    }
                                    Err(e) => {
                                        warn!(session_id = %session_id, error = %e, "recv_data error");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });

                info!(session_id = %session_id, "Session connected successfully");
                Ok(())
            }
            Err(e) => {
                error!(session_id = %session_id, error = %e, "SSH connection failed");

                // 更新状态为 Disconnected
                {
                    let mut sessions = self.sessions.write().await;
                    if let Some(state) = sessions.get_mut(&session_id) {
                        state.connection_state = ConnectionState::Disconnected;
                    }
                }

                // 发布连接失败事件
                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: ConnectionState::Disconnected,
                    info: None,
                });

                Err(CoreError::ConnectionError(e.to_string()))
            }
        }
    }

    /// 断开连接
    #[instrument(skip(self))]
    pub async fn disconnect(&self, session_id: Uuid) -> Result<(), CoreError> {
        info!(session_id = %session_id, "Disconnecting session");

        // 关闭活动连接:从 connections 中取出,然后在 guard 释放后再做 await
        let active = {
            let mut connections = self.connections.write().await;
            connections.remove(&session_id)
        };
        if let Some(conn) = active {
            // 发送取消信号
            let _ = conn._cancel_tx.send(()).await;
            // 断开 SSH 连接
            let mut client = conn.client.write().await;
            if let Err(e) = client.disconnect_ssh().await {
                warn!(session_id = %session_id, error = %e, "Error disconnecting SSH");
            }
        }

        // 更新状态为 Disconnected
        {
            let mut sessions = self.sessions.write().await;
            if let Some(state) = sessions.get_mut(&session_id) {
                state.connection_state = ConnectionState::Disconnected;
                state.connection_info = None;
            } else {
                return Err(CoreError::NotFound(format!("Session {} not found", session_id)));
            }
        }

        self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Disconnected,
            info: None,
        });

        info!(session_id = %session_id, "Session disconnected");
        Ok(())
    }

    /// 发送数据到会话
    pub async fn send_data(&self, session_id: Uuid, data: &[u8]) -> Result<(), CoreError> {
        // 先在锁内克隆出 client 句柄，立刻释放 connections guard，避免跨 await 持锁
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| CoreError::NotFound(format!("Connection {} not found", session_id)))?
        };
        let client = client.read().await;
        client
            .send_data(data)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// 调整终端大小
    pub async fn resize_terminal(&self, session_id: Uuid, cols: u32, rows: u32) -> Result<(), CoreError> {
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| CoreError::NotFound(format!("Connection {} not found", session_id)))?
        };
        let client = client.read().await;
        client
            .resize_terminal(cols, rows)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// 创建会话
    #[instrument(skip(self, config))]
    pub async fn create_session(&self, config: SessionConfig) -> Result<Uuid, CoreError> {
        let id = config.id;
        info!(session_id = %id, name = %config.name, "Creating session");

        let state = SessionState {
            config,
            connection_state: ConnectionState::Disconnected,
            connection_info: None,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(id, state);

        self.event_bus.publish(rshell_api::AppEvent::SessionListChanged);

        debug!(session_id = %id, "Session created");
        Ok(id)
    }

    /// 更新会话
    #[instrument(skip(self, config))]
    pub async fn update_session(&self, id: Uuid, config: SessionConfig) -> Result<(), CoreError> {
        info!(session_id = %id, "Updating session");

        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(&id) {
            state.config = config;
        } else {
            return Err(CoreError::NotFound(format!("Session {} not found", id)));
        }

        self.event_bus.publish(rshell_api::AppEvent::SessionUpdated { session_id: id });

        debug!(session_id = %id, "Session updated");
        Ok(())
    }

    /// 删除会话
    #[instrument(skip(self))]
    pub async fn delete_session(&self, id: Uuid) -> Result<(), CoreError> {
        info!(session_id = %id, "Deleting session");

        // 先断开连接（如果已连接）
        let _ = self.disconnect(id).await;

        let mut sessions = self.sessions.write().await;
        sessions.remove(&id);

        self.event_bus.publish(rshell_api::AppEvent::SessionListChanged);

        debug!(session_id = %id, "Session deleted");
        Ok(())
    }

    /// 获取会话状态
    pub async fn get_state(&self, session_id: Uuid) -> Result<ConnectionState, CoreError> {
        let sessions = self.sessions.read().await;
        sessions
            .get(&session_id)
            .map(|s| s.connection_state)
            .ok_or_else(|| CoreError::NotFound(format!("Session {} not found", session_id)))
    }

    /// 获取连接信息
    pub async fn get_connection_info(&self, session_id: Uuid) -> Result<Option<ConnectionInfo>, CoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&session_id).and_then(|s| s.connection_info.clone()))
    }

    /// 获取活动连接的 SSH 客户端引用（用于 SFTP 操作等）
    pub async fn get_ssh_client(&self, session_id: Uuid) -> Result<SshClientHandle, CoreError> {
        let connections = self.connections.read().await;
        connections
            .get(&session_id)
            .map(|c| c.client.clone())
            .ok_or_else(|| CoreError::NotFound(format!("Connection {} not found", session_id)))
    }

    /// 浏览远程目录
    pub async fn browse_remote_dir(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Vec<RemoteFileEntry>, CoreError> {
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;

        let channel = ssh
            .open_sftp_channel()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;

        let sftp = SftpClient::new(channel)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;

        let entries = sftp
            .list_dir(path)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;

        // 发布 RemoteDirListed 事件
        self.event_bus.publish(rshell_api::AppEvent::RemoteDirListed {
            session_id,
            path: path.to_string(),
            entries: entries.clone(),
        });

        Ok(entries)
    }

    /// 列出所有会话
    pub async fn list_sessions(&self) -> Result<Vec<SessionConfig>, CoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.values().map(|s| s.config.clone()).collect())
    }
}
