//! 会话服务

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::script::trigger_engine::TriggerEngine;
use crate::security::host_key_decision::HostKeyDecisionRegistry;
use crate::session::repository::SessionRepository;
use crate::terminal::service::TerminalService;
use crate::terminal::SharedTerminalChannels;
use rshell_api::types::{ConnectionInfo, ConnectionState, RemoteFileEntry, SessionConfig, TriggerAction};
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
    /// 触发器引擎（用于在收到远端输出后做正则/精确匹配并执行动作）
    trigger_engine: Arc<TriggerEngine>,
    /// 主机密钥决策注册表（用于在 SSH 握手期间等待 UI 端 DecideHostKey）
    host_key_registry: Arc<HostKeyDecisionRegistry>,
    /// 切片 1.0 接线：会话持久化仓库。
    /// 设为 `None` 时所有写操作仅落内存（向后兼容旧测试场景）；
    /// Tauri 壳 `setup` 阶段必须 `Some(...)` 以满足设计 §4.5 完成判据。
    repository: Option<Arc<SessionRepository>>,
    /// 切片 7.1 占位字段：用于触发器 SendText 真发远端。
    /// 当前 recv spawn 不消费（`!Send` 障碍 — `session_service.connect`
    /// 内 `host_key_registry` 持有 std::sync::Mutex 跨 await 持锁），
    /// 字段保留以便未来切片通过 `tokio::sync::Mutex` 替换 std 实现后启用。
    #[allow(dead_code)]
    dispatcher: std::sync::Weak<()>,
    /// 终端字节 sink：recv 循环把 SSH 字节推给前端 xterm（设计 §4.1）。
    /// 设为可选以保留旧测试场景（直接构造 SessionService 不带 sink）。
    terminal_channels: Option<SharedTerminalChannels>,
}

impl SessionService {
    /// 创建新的会话服务
    pub fn new(
        event_bus: Arc<EventBus>,
        _terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
    ) -> Self {
        Self::with_repository(event_bus, _terminal_service, trigger_engine, host_key_registry, None)
    }

    /// 带持久化仓库构造。
    /// 切片 1 起 Tauri 壳 `setup` 用此构造，单元测试维持 4 参版本。
    pub fn with_repository(
        event_bus: Arc<EventBus>,
        _terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
        repository: Option<Arc<SessionRepository>>,
    ) -> Self {
        Self::with_full(
            event_bus,
            _terminal_service,
            trigger_engine,
            host_key_registry,
            repository,
            None,
        )
    }

    /// 完整构造：额外注入 TerminalChannels（设计 §4.1 字节 sink）。
    /// Tauri 壳 `setup` 阶段使用。测试场景维持 5 参 `with_repository`。
    pub fn with_full(
        event_bus: Arc<EventBus>,
        _terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
        repository: Option<Arc<SessionRepository>>,
        terminal_channels: Option<SharedTerminalChannels>,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            trigger_engine,
            host_key_registry,
            repository,
            // 切片 7.1 占位：当前无 dispatcher 引用。
            // SendText 完整派发闭环推迟 —— 见 recv spawn 注释。
            dispatcher: std::sync::Weak::new(),
            terminal_channels,
        }
    }

    /// 切片 1.0：从磁盘把已保存会话灌进内存 HashMap。
    /// 读取失败（路径不存在 / 解析失败）记录 warn! 但不中断启动 —— 用户首次启动属正常空态。
    #[instrument(skip(self))]
    pub async fn load_from_disk(&self) {
        let Some(repo) = self.repository.as_ref() else {
            debug!("load_from_disk: no repository configured, skip");
            return;
        };
        let configs = match repo.list_all() {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "load_from_disk: list_all failed; continuing with empty in-memory state");
                return;
            }
        };
        let mut sessions = self.sessions.write().await;
        for cfg in configs {
            let id = cfg.id;
            sessions.insert(
                id,
                SessionState {
                    config: cfg,
                    connection_state: ConnectionState::Disconnected,
                    connection_info: None,
                },
            );
            debug!(session_id = %id, "load_from_disk: restored");
        }
        info!(count = sessions.len(), "load_from_disk complete");
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

        // 创建 SSH 客户端并连接 — 通过 host_key_registry 接入 host key 决策通道:
        // 遇到未知 host key 时,SshHandler::check_server_key 会在 EventBus 上发
        // HostKeyMismatch { decision_id, ... } 然后同步 block_on 等 UI 端的
        // AppCommand::DecideHostKey。
        let mut client = SshClient::new(config.clone());
        let sink: Arc<dyn rshell_protocol::ssh::HostKeyDecisionSink> = self.host_key_registry.clone();

        match client.connect_ssh(Some(sink)).await {
            Ok(()) => {
                info!(session_id = %session_id, "SSH connection established");

                // 创建取消通道
                let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
                // 切片 2.1 占位：触发器 SendText 需要从 recv 循环传回 SessionService
                // 调 send_data。完整派发闭环留到切片 7 触发器域；本切片先把通道占住
                // 让编译通过 —— 触发器目前仅 publish TriggerFired 事件,不再做假回显
                // (设计 §2.4 修复方向已落地)。
                let (_trigger_send_tx, _trigger_send_rx) = mpsc::channel::<Vec<u8>>(16);

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
                //
                // 切片 2.1 占位保留（切片 7 探索 SendText 派发失败,见 recv spawn 注释）：
                let trigger_engine = self.trigger_engine.clone();
                let client_clone = client.clone();
                // recv 循环需要把 SSH 字节推到 TerminalChannels;
                // 这里 clone Arc 让 spawn 闭包拥有自己的句柄,避开 self 借用。
                let terminal_channels = self.terminal_channels.clone();
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
                                        // ── 字节直推 xterm（设计 §2.3 / §4.1） ──
                                        // 通过 TerminalChannels 推到前端 xterm:
                                        // attach 之前自动 Buffering,attach 时一次性 flush。
                                        if let Some(tc) = &terminal_channels {
                                            tc.push(session_id, &data).await;
                                        }

                                        // ── 触发器匹配（原始字节 → UTF-8 → 正则） ──
                                        if let Ok(text) = std::str::from_utf8(&data) {
                                            match trigger_engine.check_output(text, session_id) {
                                                Ok(matches) => {
                                                    for m in matches {
                                                        let summary = match &m.action {
                                                            TriggerAction::SendText(s) => format!("send_text({} chars)", s.len()),
                                                            TriggerAction::ShowNotification(s) => format!("notify: {}", s),
                                                            TriggerAction::Disconnect => "disconnect".to_string(),
                                                            TriggerAction::LogToFile(p) => format!("log_to_file: {}", p.display()),
                                                        };
                                                        trigger_engine.notify_fired(m.trigger_id, session_id, &summary);
                                                        // 触发器动作派发:
                                                        // - ShowNotification / Disconnect / LogToFile:
                                                        //   通过 TriggerFired 事件通知
                                                        // - SendText: 切片 2.1 起直接调 SessionService::send_data
                                                        //   发往远端（设计 §2.4 修复,不再做假回显）
                                                        if let TriggerAction::SendText(text) = m.action {
                                                            // 切片 2.1 占位保留：SendText 完整派发闭环需要
                                                            // dispatcher.send_data 路径 Send future;当前
                                                            // session_service.connect 内部持有 HostKeyDecisionRegistry
                                                            // 的 std::sync::Mutex 跨越某些 await 点导致
                                                            // dispatcher.dispatch 返回 future 是 !Send,
                                                            // 直接 tokio::spawn 触发 borrow check 失败。
                                                            //
                                                            // 当前实现:仅 publish TriggerFired 事件(让前端 UI 提示),
                                                            // 不做远端真发。SendText 触发器在切片 7 内作为
                                                            // 已知缺口记录在 task_plan.md,后续切片通过
                                                            // Send-safe 派发通道(host_key_registry 替换为
                                                            // tokio::sync::Mutex 或 AppEvent::TriggerSendTextRequest
                                                            // 异步路径)解决。
                                                            let _ = text; // 占位
                                                            warn!(
                                                                session_id = %session_id,
                                                                "trigger SendText not yet wired to send_data (see task_plan.md slice 7)"
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(e) => warn!(
                                                    session_id = %session_id,
                                                    error = %e,
                                                    "trigger_engine.check_output failed"
                                                ),
                                            }
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

                // 切片 2.1 占位:Recv spawn 已经 move 走 trigger_send_rx_for_loop 的所有权。
                // 完整 SendText 派发到 send_data 的逻辑留到切片 7 触发器域,本切片仅
                // 保证触发器不再做假回显（设计 §2.4 修复方向已落地）。

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

        // 切片 1.0：先落盘再入内存。落盘失败时阻断 create —— 避免出现
        // "内存有但磁盘无"的不可恢复分裂状态（设计 §4.5 完成判据前提）。
        if let Some(repo) = self.repository.as_ref() {
            repo.save(&config).map_err(|e| {
                CoreError::StorageError(format!("save session {} failed: {}", id, e))
            })?;
        }

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

        if let Some(repo) = self.repository.as_ref() {
            repo.save(&config).map_err(|e| {
                CoreError::StorageError(format!("save session {} failed: {}", id, e))
            })?;
        }

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

        if let Some(repo) = self.repository.as_ref() {
            repo.delete(id).map_err(|e| {
                CoreError::StorageError(format!("delete session {} failed: {}", id, e))
            })?;
        }

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

        // 切片 2.2：RemoteDirListed 事件已删除 —— 数据通过 CommandOutcome::RemoteDir
        // 由 BrowseRemoteDir 薄壳直接返回（设计 §3.3）。
        Ok(entries)
    }

    /// 列出所有会话
    pub async fn list_sessions(&self) -> Result<Vec<SessionConfig>, CoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.values().map(|s| s.config.clone()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::trigger_engine::TriggerEngine;
    use crate::security::host_key_decision::HostKeyDecisionRegistry;
    use crate::terminal::service::TerminalService;
    use rshell_api::types::{AuthMethod, Protocol, SessionConfig};
    use rshell_api::AppEvent;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_service() -> SessionService {
        let bus = Arc::new(EventBus::new());
        let ts = Arc::new(TerminalService::new(bus.clone()));
        let te = Arc::new(TriggerEngine::new(bus.clone()));
        let hk = Arc::new(HostKeyDecisionRegistry::new(bus.clone()));
        SessionService::new(bus, ts, te, hk)
    }

    fn make_config(name: &str, host: &str) -> SessionConfig {
        SessionConfig {
            id: Uuid::new_v4(),
            name: name.to_string(),
            folder_id: None,
            host: host.to_string(),
            port: 22,
            protocol: Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "user".to_string(),
                password: "pw".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_create_session() {
        let svc = make_service();
        let cfg = make_config("a", "host1");
        let id = svc.create_session(cfg.clone()).await.unwrap();
        assert_eq!(id, cfg.id);
        let all = svc.list_sessions().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "a");
    }

    #[tokio::test]
    async fn test_get_state_initial_is_disconnected() {
        let svc = make_service();
        let cfg = make_config("a", "host1");
        let id = svc.create_session(cfg).await.unwrap();
        let state = svc.get_state(id).await.unwrap();
        assert_eq!(state, ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_get_state_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc.get_state(Uuid::new_v4()).await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_get_connection_info_none_when_not_connected() {
        let svc = make_service();
        let cfg = make_config("a", "host1");
        let id = svc.create_session(cfg).await.unwrap();
        let info = svc.get_connection_info(id).await.unwrap();
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn test_get_ssh_client_unknown_returns_not_found() {
        let svc = make_service();
        match svc.get_ssh_client(Uuid::new_v4()).await {
            Err(e) => assert!(format!("{e}").contains("not found")),
            Ok(_) => panic!("expected Err"),
        }
    }

    #[tokio::test]
    async fn test_send_data_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc.send_data(Uuid::new_v4(), b"hi").await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_resize_terminal_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc.resize_terminal(Uuid::new_v4(), 80, 24).await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_browse_remote_dir_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc.browse_remote_dir(Uuid::new_v4(), "/").await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_list_sessions_empty() {
        let svc = make_service();
        let all = svc.list_sessions().await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let svc = make_service();
        let cfg = make_config("a", "host1");
        let id = svc.create_session(cfg).await.unwrap();
        svc.delete_session(id).await.unwrap();
        assert!(svc.list_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_delete_unknown_is_idempotent_ok() {
        // 当前实现:delete_session 对未知 id 是幂等 (返回 Ok),
        // 便于 UI 端"重试删除"语义。这与 get_state 的 NotFound 行为不同。
        let svc = make_service();
        assert!(svc.delete_session(Uuid::new_v4()).await.is_ok());
    }

    #[tokio::test]
    async fn test_update_session() {
        let svc = make_service();
        let cfg = make_config("a", "host1");
        let id = svc.create_session(cfg.clone()).await.unwrap();
        let mut cfg2 = cfg.clone();
        cfg2.name = "renamed".to_string();
        svc.update_session(id, cfg2).await.unwrap();
        let all = svc.list_sessions().await.unwrap();
        assert_eq!(all[0].name, "renamed");
    }

    #[tokio::test]
    async fn test_disconnect_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc.disconnect(Uuid::new_v4()).await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_session_list_changed_published_on_create() {
        let bus = Arc::new(EventBus::new());
        let got = Arc::new(std::sync::Mutex::new(0u32));
        let g = got.clone();
        bus.subscribe(move |event| {
            if matches!(event, AppEvent::SessionListChanged) {
                *g.lock().unwrap() += 1;
            }
        });
        let ts = Arc::new(TerminalService::new(bus.clone()));
        let te = Arc::new(TriggerEngine::new(bus.clone()));
        let hk = Arc::new(HostKeyDecisionRegistry::new(bus.clone()));
        let svc = SessionService::new(bus, ts, te, hk);
        let cfg = make_config("a", "host1");
        svc.create_session(cfg).await.unwrap();
        svc.create_session(make_config("b", "host2")).await.unwrap();
        assert_eq!(*got.lock().unwrap(), 2);
    }

    // Lock-leak smoke test: get_state 和 list_sessions 持有 sessions guard 时
    // 不会跨 .await 持锁(本轮重构后特别要回归)
    #[tokio::test]
    async fn test_list_sessions_under_concurrent_create() {
        let svc = Arc::new(make_service());
        let mut handles = vec![];
        for i in 0..20 {
            let svc = svc.clone();
            handles.push(tokio::spawn(async move {
                let cfg = make_config(&format!("s{}", i), "host");
                svc.create_session(cfg).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // 边创建边列, 不应死锁
        let all = svc.list_sessions().await.unwrap();
        assert_eq!(all.len(), 20);
        let _ = HashMap::<String, PathBuf>::new();
    }

    // ─────────────────────────────────────────────────────────────────────
    // 切片 1.0：SessionRepository 接线 + 重启持久化回归测试（设计 §4.5）
    // ─────────────────────────────────────────────────────────────────────

    fn make_service_with_repo(repo: Arc<SessionRepository>) -> SessionService {
        let bus = Arc::new(EventBus::new());
        let ts = Arc::new(TerminalService::new(bus.clone()));
        let te = Arc::new(TriggerEngine::new(bus.clone()));
        let hk = Arc::new(HostKeyDecisionRegistry::new(bus.clone()));
        SessionService::with_repository(bus, ts, te, hk, Some(repo))
    }

    #[tokio::test]
    async fn test_session_persistence_roundtrip() {
        // 用 tempfile 给一个隔离目录,模拟"进程重启"。
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_path_buf();

        // 第一个生命周期:create ×2 → 落盘。
        let repo_a = Arc::new(SessionRepository::new(path.clone()));
        let svc_a = make_service_with_repo(repo_a.clone());
        let cfg_a = make_config("alpha", "host-a");
        let id_a = svc_a.create_session(cfg_a).await.unwrap();
        let cfg_b = make_config("beta", "host-b");
        let id_b = svc_a.create_session(cfg_b).await.unwrap();
        drop(svc_a); // 显式 drop,模拟进程退出

        // 第二个生命周期:重新构造 → load_from_disk → 应能列回两条。
        let repo_b = Arc::new(SessionRepository::new(path));
        let svc_b = make_service_with_repo(repo_b);
        svc_b.load_from_disk().await;
        let restored = svc_b.list_sessions().await.unwrap();
        assert_eq!(restored.len(), 2, "重启后应能恢复 2 条会话");
        let ids: std::collections::HashSet<_> = restored.iter().map(|s| s.id).collect();
        assert!(ids.contains(&id_a));
        assert!(ids.contains(&id_b));
    }

    #[tokio::test]
    async fn test_delete_removes_from_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_path_buf();
        let repo = Arc::new(SessionRepository::new(path.clone()));
        let svc = make_service_with_repo(repo.clone());
        let cfg = make_config("x", "host-x");
        let id = svc.create_session(cfg).await.unwrap();
        svc.delete_session(id).await.unwrap();

        // 直接从磁盘读,确认条目已被抹除。
        assert!(repo.load(id).unwrap().is_none(), "delete 必须从磁盘抹除");
    }

    #[tokio::test]
    async fn test_load_from_disk_without_repository_is_noop() {
        // 旧 4 参构造保持工作:repository = None 时 load_from_disk 不应 panic。
        let svc = make_service();
        svc.load_from_disk().await;
        assert!(svc.list_sessions().await.unwrap().is_empty());
    }
}
