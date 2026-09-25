//! 会话服务

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::script::trigger_engine::TriggerEngine;
use crate::security::host_key_decision::HostKeyDecisionRegistry;
use crate::session::repository::SessionRepository;
use crate::terminal::service::TerminalService;
use rshell_api::types::{ConnectionInfo, ConnectionState, FileType, Protocol, RemoteFileEntry, SessionConfig, TriggerAction};
use rshell_protocol::ssh::SshClient;
use rshell_protocol::ssh::sftp::SftpClient;
use rshell_protocol::telnet::TelnetConnection;
use rshell_protocol::serial::{SerialConnection, SerialConfig as ProtocolSerialConfig};
use rshell_protocol::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

/// 活动连接的 SSH 客户端句柄别名（与 SshClient 内部使用了同样的 tokio RwLock）
pub type SshClientHandle = Arc<tokio::sync::RwLock<SshClient>>;

fn validate_session_config(config: &SessionConfig) -> Result<(), CoreError> {
    match config.protocol {
        Protocol::SSH | Protocol::Telnet => {
            if config.host.trim().is_empty() || config.port == 0 {
                return Err(CoreError::InvalidState("Host and TCP port are required".into()));
            }
        }
        Protocol::Serial => {
            let serial = config.serial_config.as_ref()
                .ok_or_else(|| CoreError::InvalidState("Serial settings are required".into()))?;
            if !serial.port.starts_with("/dev/") || serial.baud_rate == 0
                || !(5..=8).contains(&serial.data_bits) || !(1..=2).contains(&serial.stop_bits)
            {
                return Err(CoreError::InvalidState("Invalid macOS serial configuration".into()));
            }
        }
    }
    Ok(())
}

/// Destructive SFTP operations accept only an absolute, unambiguous non-root path.
pub(crate) fn validate_remote_mutation_path(path: &str) -> Result<(), CoreError> {
    if !path.starts_with('/') || path == "/" || path.ends_with('/')
        || path.split('/').skip(1).any(|part| part.is_empty() || part == "." || part == "..")
        || path.chars().any(|c| c == '\0' || c == '\\')
    {
        return Err(CoreError::InvalidState(format!("Unsafe remote file path: {path}")));
    }
    Ok(())
}

async fn append_trigger_log(path: &Path, output: &str) -> Result<(), CoreError> {
    if !path.is_absolute() || path.file_name().is_none() || path.is_dir() {
        return Err(CoreError::InvalidState("Trigger log path must be an absolute file".into()));
    }
    let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await
        .map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.write_all(output.as_bytes()).await.map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.write_all(b"\n").await.map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.flush().await.map_err(|e| CoreError::StorageError(e.to_string()))
}

fn trigger_action_summary(action: &TriggerAction) -> String {
    match action {
        TriggerAction::SendText(text) => format!("send_text({} chars)", text.len()),
        TriggerAction::ShowNotification(text) => format!("notify: {text}"),
        TriggerAction::Disconnect => "disconnect".into(),
        TriggerAction::LogToFile(path) => format!("log_to_file: {}", path.display()),
    }
}

async fn execute_trigger_action(
    action: &TriggerAction,
    output: &str,
    session_id: Uuid,
    client: Option<&SshClientHandle>,
) -> Result<(), CoreError> {
    match action {
        TriggerAction::SendText(text) => {
            let client = client
                .ok_or_else(|| CoreError::NotFound(format!("Connection {session_id} not found")))?;
            let result = client.read().await.send_data(text.as_bytes()).await
                .map_err(|e| CoreError::ConnectionError(e.to_string()));
            result
        }
        TriggerAction::ShowNotification(_) => Ok(()),
        TriggerAction::LogToFile(path) => append_trigger_log(path, output).await,
        TriggerAction::Disconnect => {
            let client = client
                .ok_or_else(|| CoreError::NotFound(format!("Connection {session_id} not found")))?;
            let result = client.write().await.disconnect_ssh().await
                .map_err(|e| CoreError::ConnectionError(e.to_string()));
            result
        }
    }
}

/// 会话运行时状态
struct SessionState {
    config: SessionConfig,
    connection_state: ConnectionState,
    /// 连接信息（连接成功后填充）
    connection_info: Option<ConnectionInfo>,
    attempt: Option<Uuid>,
    cancel_connect: Option<oneshot::Sender<()>>,
}

/// 活动连接（使用 channel 来接收数据）
struct ActiveConnection {
    /// 用于发送数据到远程 shell
    client: SshClientHandle,
    /// 用于取消后台读取任务的通道
    _cancel_tx: mpsc::Sender<()>,
}

enum ProtocolRequest {
    Send(Vec<u8>, oneshot::Sender<Result<(), CoreError>>),
    Resize(u16, u16, oneshot::Sender<Result<(), CoreError>>),
    Disconnect(oneshot::Sender<Result<(), CoreError>>),
}

/// 会话服务 - 管理会话的生命周期
pub struct SessionService {
    lifecycle: Arc<Mutex<()>>,
    /// 会话状态映射
    sessions: Arc<RwLock<HashMap<Uuid, SessionState>>>,
    /// 活动连接映射（tokio RwLock，因为持锁等待期需要跨 await）
    connections: Arc<RwLock<HashMap<Uuid, ActiveConnection>>>,
    protocol_connections: Arc<RwLock<HashMap<Uuid, mpsc::Sender<ProtocolRequest>>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
    terminal_service: Arc<TerminalService>,
    /// 触发器引擎（用于在收到远端输出后做正则/精确匹配并执行动作）
    trigger_engine: Arc<TriggerEngine>,
    /// 主机密钥决策注册表（用于在 SSH 握手期间等待 UI 端 DecideHostKey）
    host_key_registry: Arc<HostKeyDecisionRegistry>,
    /// 切片 1.0 接线：会话持久化仓库。
    /// 设为 `None` 时所有写操作仅落内存（向后兼容旧测试场景）；
    /// Tauri 壳 `setup` 阶段必须 `Some(...)` 以满足设计 §4.5 完成判据。
    repository: Option<Arc<SessionRepository>>,
}

impl SessionService {
    /// 创建新的会话服务
    pub fn new(
        event_bus: Arc<EventBus>,
        terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
    ) -> Self {
        Self::with_repository(event_bus, terminal_service, trigger_engine, host_key_registry, None)
    }

    /// 带持久化仓库构造。
    /// 切片 1 起 Tauri 壳 `setup` 用此构造，单元测试维持 4 参版本。
    pub fn with_repository(
        event_bus: Arc<EventBus>,
        terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
        repository: Option<Arc<SessionRepository>>,
    ) -> Self {
        let mut restored = HashMap::new();
        if let Some(repo) = &repository {
            match repo.list_all() {
                Ok(configs) => {
                    for config in configs {
                        restored.insert(config.id, SessionState {
                            config, connection_state: ConnectionState::Disconnected, connection_info: None,
                            attempt: None, cancel_connect: None,
                        });
                    }
                }
                Err(e) => warn!(error = %e, "Could not restore saved sessions"),
            }
        }
        Self {
            lifecycle: Arc::new(Mutex::new(())),
            sessions: Arc::new(RwLock::new(restored)),
            connections: Arc::new(RwLock::new(HashMap::new())),
            protocol_connections: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            terminal_service,
            trigger_engine,
            host_key_registry,
            repository,
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
                    attempt: None, cancel_connect: None,
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
        let attempt = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        // Atomically claim the session before any network await.
        let config = {
            let _lifecycle = self.lifecycle.lock().await;
            let mut sessions = self.sessions.write().await;
            let state = sessions.get_mut(&session_id)
                .ok_or_else(|| CoreError::NotFound(format!("Session {} not found", session_id)))?;
            if matches!(state.connection_state, ConnectionState::Connecting | ConnectionState::Connected) {
                return Err(CoreError::InvalidState(format!("Session {session_id} is already connecting or connected")));
            }
            state.connection_state = ConnectionState::Connecting;
            state.attempt = Some(attempt);
            state.cancel_connect = Some(cancel_tx);
            self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                session_id, state: ConnectionState::Connecting, info: None,
            });
            state.config.clone()
        };
        let result = tokio::select! {
            result = self.connect_attempt(session_id, config, attempt) => result,
            _ = cancel_rx => Err(CoreError::InvalidState("Connection attempt cancelled".into())),
        };
        if result.is_err() {
            let _lifecycle = self.lifecycle.lock().await;
            let mut sessions = self.sessions.write().await;
            if let Some(state) = sessions.get_mut(&session_id).filter(|state| state.attempt == Some(attempt)) {
                state.attempt = None;
                state.cancel_connect = None;
                state.connection_state = ConnectionState::Disconnected;
                state.connection_info = None;
                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id, state: ConnectionState::Disconnected, info: None,
                });
            }
        }
        result
    }

    async fn connect_attempt(&self, session_id: Uuid, config: SessionConfig, attempt: Uuid) -> Result<(), CoreError> {
        if config.protocol != Protocol::SSH {
            return self.connect_protocol(session_id, config, attempt).await;
        }

        // 创建 SSH 客户端并连接 — 通过 host_key_registry 接入 host key 决策通道:
        // 遇到未知 host key 时,SshHandler::check_server_key 会在 EventBus 上发
        // HostKeyMismatch { decision_id, ... } 然后同步 block_on 等 UI 端的
        // AppCommand::DecideHostKey。
        let mut client = SshClient::new(config.clone());
        let sink: Arc<dyn rshell_protocol::ssh::HostKeyDecisionSink> = self.host_key_registry.clone();

        match client.connect_ssh(Some(sink)).await {
            Ok(()) => {
                info!(session_id = %session_id, "SSH connection established");
                if let Some((cols, rows)) = self.terminal_service.size(session_id) {
                    if let Err(e) = client.resize_terminal(cols as u32, rows as u32).await {
                        warn!(session_id = %session_id, error = %e, "initial terminal resize failed");
                    }
                }

                let mut output_rx = client.take_data_receiver()
                    .ok_or_else(|| CoreError::InvalidState("SSH output channel missing".into()))?;
                // 创建取消通道
                let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                // 包装客户端为 Arc<RwLock>
                let client = Arc::new(tokio::sync::RwLock::new(client));

                let publication = self.lifecycle.lock().await;
                if !self.sessions.read().await.get(&session_id).is_some_and(|state| state.attempt == Some(attempt)) {
                    return Err(CoreError::InvalidState("Connection attempt cancelled".into()));
                }

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
                        state.cancel_connect = None;
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
                drop(publication);

                // 输出接收器独立于 SSH 客户端锁，读等待不阻塞发送/SFTP。
                let trigger_engine = self.trigger_engine.clone();
                let sessions = self.sessions.clone();
                let connections = self.connections.clone();
                let event_bus = self.event_bus.clone();
                let terminal_service = self.terminal_service.clone();
                let lifecycle = self.lifecycle.clone();
                tokio::spawn(async move {
                    'reader: loop {
                        tokio::select! {
                            _ = cancel_rx.recv() => {
                                debug!(session_id = %session_id, "Data reader cancelled");
                                break;
                            }
                            result = output_rx.recv() => {
                                match result {
                                    Some(data) => {
                                        if let Err(e) = terminal_service.push_output(session_id, data.clone()) {
                                            warn!(session_id = %session_id, error = %e, "terminal output delivery failed");
                                        }

                                        // ── 触发器匹配（原始字节 → UTF-8 → 正则） ──
                                        if let Ok(text) = std::str::from_utf8(&data) {
                                            match trigger_engine.check_output(text, session_id) {
                                                Ok(matches) => {
                                                    for m in matches {
                                                        let summary = trigger_action_summary(&m.action);
                                                        let result = execute_trigger_action(&m.action, text, session_id, Some(&client)).await;
                                                        match result {
                                                            Ok(()) => trigger_engine.notify_fired(m.trigger_id, session_id, &summary),
                                                            Err(e) => {
                                                                warn!(trigger_id = %m.trigger_id, session_id = %session_id, error = %e, "trigger action failed");
                                                                event_bus.publish(rshell_api::AppEvent::TriggerActionFailed {
                                                                    trigger_id: m.trigger_id, session_id, error: e.to_string(),
                                                                });
                                                            }
                                                        }
                                                        if matches!(m.action, TriggerAction::Disconnect) { break 'reader; }
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
                                    None => {
                                        debug!(session_id = %session_id, "recv_data stream ended");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // Close transport resources on remote shell EOF as well as
                    // local cancellation before retiring this generation.
                    let _ = client.write().await.disconnect_ssh().await;
                    let _lifecycle = lifecycle.lock().await;
                    let mut states = sessions.write().await;
                    if let Some(state) = states.get_mut(&session_id).filter(|state| state.attempt == Some(attempt)) {
                        connections.write().await.remove(&session_id);
                        state.attempt = None;
                        state.connection_state = ConnectionState::Disconnected;
                        state.connection_info = None;
                        event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                            session_id, state: ConnectionState::Disconnected, info: None,
                        });
                    }
                });

                info!(session_id = %session_id, "Session connected successfully");
                Ok(())
            }
            Err(e) => {
                error!(session_id = %session_id, error = %e, "SSH connection failed");

                // 切片 2.1 占位:Recv spawn 已经 move 走 trigger_send_rx_for_loop 的所有权。
                // 完整 SendText 派发到 send_data 的逻辑留到切片 7 触发器域,本切片仅
                // 保证触发器不再做假回显（设计 §2.4 修复方向已落地）。

                Err(CoreError::ConnectionError(e.to_string()))
            }
        }
    }

    async fn connect_protocol(&self, session_id: Uuid, config: SessionConfig, attempt: Uuid) -> Result<(), CoreError> {
        let connection: Result<Box<dyn Connection>, CoreError> = match config.protocol {
            Protocol::Telnet => Ok(Box::new(TelnetConnection::new(&config.host, config.port))),
            Protocol::Serial => {
                config.serial_config.as_ref().map(|serial| Box::new(SerialConnection::new(ProtocolSerialConfig {
                    port: serial.port.clone(), baud_rate: serial.baud_rate,
                    data_bits: serial.data_bits, stop_bits: serial.stop_bits,
                    parity: match serial.parity {
                        rshell_api::types::SerialParity::None => rshell_protocol::serial::SerialParity::None,
                        rshell_api::types::SerialParity::Even => rshell_protocol::serial::SerialParity::Even,
                        rshell_api::types::SerialParity::Odd => rshell_protocol::serial::SerialParity::Odd,
                    },
                    flow_control: match serial.flow_control {
                        rshell_api::types::SerialFlowControl::None => rshell_protocol::serial::SerialFlowControl::None,
                        rshell_api::types::SerialFlowControl::Software => rshell_protocol::serial::SerialFlowControl::Software,
                        rshell_api::types::SerialFlowControl::Hardware => rshell_protocol::serial::SerialFlowControl::Hardware,
                    },
                })) as Box<dyn Connection>).ok_or_else(|| CoreError::InvalidState("Serial settings are missing".into()))
            }
            Protocol::SSH => unreachable!(),
        };
        let mut connection = connection?;
        connection.connect().await.map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        if let Some((cols, rows)) = self.terminal_service.size(session_id) {
            if let Err(e) = connection.resize(cols, rows).await {
                warn!(session_id = %session_id, error = %e, "initial terminal resize failed");
            }
        }
        let info = ConnectionInfo {
            protocol: config.protocol, host: config.host.clone(), port: config.port,
            state: ConnectionState::Connected, bytes_sent: 0, bytes_received: 0, latency_ms: None,
        };
        let (tx, mut rx) = mpsc::channel::<ProtocolRequest>(32);
        let publication = self.lifecycle.lock().await;
        if !self.sessions.read().await.get(&session_id).is_some_and(|state| state.attempt == Some(attempt)) {
            return Err(CoreError::InvalidState("Connection attempt cancelled".into()));
        }
        self.protocol_connections.write().await.insert(session_id, tx);
        if let Some(state) = self.sessions.write().await.get_mut(&session_id) {
            state.connection_state = ConnectionState::Connected;
            state.cancel_connect = None;
            state.connection_info = Some(info.clone());
        }
        self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
            session_id, state: ConnectionState::Connected, info: Some(info),
        });
        drop(publication);
        let sessions = self.sessions.clone();
        let connections = self.protocol_connections.clone();
        let event_bus = self.event_bus.clone();
        let terminal_service = self.terminal_service.clone();
        let trigger_engine = self.trigger_engine.clone();
        let lifecycle = self.lifecycle.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            loop {
                tokio::select! {
                    request = rx.recv() => match request {
                        Some(ProtocolRequest::Send(data, reply)) => {
                            let _ = reply.send(connection.send(&data).await.map_err(|e| CoreError::ConnectionError(e.to_string())));
                        }
                        Some(ProtocolRequest::Resize(cols, rows, reply)) => {
                            let _ = reply.send(connection.resize(cols, rows).await.map_err(|e| CoreError::ConnectionError(e.to_string())));
                        }
                        Some(ProtocolRequest::Disconnect(reply)) => {
                            let _ = reply.send(connection.disconnect().await.map_err(|e| CoreError::ConnectionError(e.to_string())));
                            break;
                        }
                        None => break,
                    },
                    result = connection.recv(&mut buf) => match result {
                        Ok(n) if n > 0 => {
                            let data = buf[..n].to_vec();
                            if let Err(e) = terminal_service.push_output(session_id, data.clone()) {
                                warn!(session_id = %session_id, error = %e, "terminal output delivery failed");
                            }
                            if let Ok(output) = std::str::from_utf8(&data) {
                                if let Ok(matches) = trigger_engine.check_output(output, session_id) {
                                    let mut should_disconnect = false;
                                    for matched in matches {
                                        let result = match &matched.action {
                                            TriggerAction::SendText(text) => connection.send(text.as_bytes()).await.map_err(|e| CoreError::ConnectionError(e.to_string())),
                                            TriggerAction::ShowNotification(_) => Ok(()),
                                            TriggerAction::LogToFile(path) => append_trigger_log(path, output).await,
                                            TriggerAction::Disconnect => {
                                                should_disconnect = true;
                                                connection.disconnect().await.map_err(|e| CoreError::ConnectionError(e.to_string()))
                                            }
                                        };
                                        match result {
                                            Ok(()) => trigger_engine.notify_fired(matched.trigger_id, session_id, &trigger_action_summary(&matched.action)),
                                            Err(e) => event_bus.publish(rshell_api::AppEvent::TriggerActionFailed {
                                                trigger_id: matched.trigger_id, session_id, error: e.to_string(),
                                            }),
                                        }
                                        if should_disconnect { break; }
                                    }
                                    if should_disconnect { break; }
                                }
                            }
                        }
                        Ok(_) => continue,
                        Err(e) => { warn!(session_id = %session_id, error = %e, "protocol read failed"); break; }
                    },
                }
            }
            // Release every queued reply before awaiting lifecycle/map locks.
            drop(rx);
            let _lifecycle = lifecycle.lock().await;
            let mut sessions = sessions.write().await;
            if let Some(state) = sessions.get_mut(&session_id).filter(|state| state.attempt == Some(attempt)) {
                connections.write().await.remove(&session_id);
                state.attempt = None;
                state.connection_state = ConnectionState::Disconnected;
                state.connection_info = None;
                event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id, state: ConnectionState::Disconnected, info: None,
                });
            }
        });
        Ok(())
    }

    /// 断开连接
    #[instrument(skip(self))]
    pub async fn disconnect(&self, session_id: Uuid) -> Result<(), CoreError> {
        info!(session_id = %session_id, "Disconnecting session");
        self.detach_session(session_id, false).await
    }

    async fn detach_session(&self, session_id: Uuid, delete: bool) -> Result<(), CoreError> {
        let (sender, active) = {
            let _lifecycle = self.lifecycle.lock().await;
            let mut sessions = self.sessions.write().await;
            let state = sessions.get_mut(&session_id)
                .ok_or_else(|| CoreError::NotFound(format!("Session {session_id} not found")))?;
            state.attempt = None;
            if let Some(cancel) = state.cancel_connect.take() { let _ = cancel.send(()); }
            state.connection_state = ConnectionState::Disconnected;
            state.connection_info = None;
            let sender = self.protocol_connections.write().await.remove(&session_id);
            let active = self.connections.write().await.remove(&session_id);
            if delete { sessions.remove(&session_id); }
            self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                session_id, state: ConnectionState::Disconnected, info: None,
            });
            (sender, active)
        };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender.send(ProtocolRequest::Disconnect(reply_tx)).await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            reply_rx.await.map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))??;
            return Ok(());
        }

        if let Some(conn) = active {
            // 发送取消信号
            let _ = conn._cancel_tx.send(()).await;
            // 断开 SSH 连接
            let mut client = conn.client.write().await;
            if let Err(e) = client.disconnect_ssh().await {
                warn!(session_id = %session_id, error = %e, "Error disconnecting SSH");
            }
        }

        info!(session_id = %session_id, "Session disconnected");
        Ok(())
    }

    /// 发送数据到会话
    pub async fn send_data(&self, session_id: Uuid, data: &[u8]) -> Result<(), CoreError> {
        let sender = { self.protocol_connections.read().await.get(&session_id).cloned() };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender.send(ProtocolRequest::Send(data.to_vec(), reply_tx)).await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            return reply_rx.await.map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
        }
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
        let sender = { self.protocol_connections.read().await.get(&session_id).cloned() };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender.send(ProtocolRequest::Resize(cols as u16, rows as u16, reply_tx)).await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            return reply_rx.await.map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
        }
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
        validate_session_config(&config)?;
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
            attempt: None, cancel_connect: None,
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
        validate_session_config(&config)?;
        if id != config.id {
            return Err(CoreError::InvalidState("Session ID cannot be changed".into()));
        }
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

        if let Some(repo) = self.repository.as_ref() {
            repo.delete(id).map_err(|e| {
                CoreError::StorageError(format!("delete session {} failed: {}", id, e))
            })?;
        }

        // Invalidate and remove atomically; a concurrent reconnect cannot slip
        // between disconnect and deletion and publish into a deleted session.
        let _ = self.detach_session(id, true).await;

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

    pub async fn create_remote_directory(&self, session_id: Uuid, path: &str) -> Result<(), CoreError> {
        validate_remote_mutation_path(path)?;
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;
        let channel = ssh.open_sftp_channel().await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let sftp = SftpClient::new(channel).await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        sftp.create_dir(path).await.map_err(|e| CoreError::ServiceError(e.to_string()))
    }

    pub async fn delete_remote_entry(&self, session_id: Uuid, path: &str) -> Result<(), CoreError> {
        validate_remote_mutation_path(path)?;
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;
        let channel = ssh.open_sftp_channel().await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let sftp = SftpClient::new(channel).await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let entry = sftp.metadata(path).await.map_err(|e| CoreError::ServiceError(e.to_string()))?;
        if entry.file_type != FileType::File {
            return Err(CoreError::InvalidState("Only regular remote files can be deleted".into()));
        }
        sftp.remove_file(path).await.map_err(|e| CoreError::ServiceError(e.to_string()))
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

    #[test]
    fn remote_mutation_path_rejects_root_empty_and_traversal() {
        for path in ["", "/", "//", "/a/../b", "/a/./b", "relative/file", "/a/"] {
            assert!(validate_remote_mutation_path(path).is_err(), "{path}");
        }
        assert!(validate_remote_mutation_path("/home/user/file.txt").is_ok());
    }

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
            serial_config: None,
        }
    }

    #[tokio::test]
    async fn disconnect_cancels_pending_handshake_and_allows_reconnect() {
        let svc = Arc::new(make_service());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("pending", "127.0.0.1");
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        let first = { let svc = svc.clone(); tokio::spawn(async move { svc.connect(id).await }) };
        let (_socket, _) = listener.accept().await.unwrap();
        svc.disconnect(id).await.unwrap();
        let second = { let svc = svc.clone(); tokio::spawn(async move { svc.connect(id).await }) };
        let (_second_socket, _) = listener.accept().await.unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), first).await.expect("cancel must end pending handshake").unwrap().is_err());
        assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Connecting);
        svc.delete_session(id).await.unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), second).await.expect("delete must cancel handshake").unwrap().is_err());
        assert!(svc.get_state(id).await.is_err());
        assert!(!svc.connections.read().await.contains_key(&id));
    }

    #[tokio::test]
    async fn protocol_requests_release_lookup_lock_before_waiting_for_reply() {
        for operation in 0..3 {
            let svc = Arc::new(make_service());
            let id = svc.create_session(make_config("race", "host")).await.unwrap();
            let (tx, mut rx) = mpsc::channel(1);
            svc.protocol_connections.write().await.insert(id, tx);
            let caller = { let svc = svc.clone(); tokio::spawn(async move {
                match operation {
                    0 => svc.send_data(id, b"x").await,
                    1 => svc.resize_terminal(id, 90, 30).await,
                    _ => svc.disconnect(id).await,
                }
            }) };
            let request = rx.recv().await.unwrap();
            // EOF cleanup needs this write lock before the request's reply is dropped.
            let cleanup = async {
                svc.protocol_connections.write().await.remove(&id);
                drop(request);
                drop(rx);
                assert!(caller.await.unwrap().is_err());
            };
            tokio::time::timeout(std::time::Duration::from_millis(300), cleanup).await.expect("EOF/request race deadlocked");
        }
    }

    #[tokio::test]
    async fn telnet_session_keeps_connection_and_routes_terminal_io() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut negotiation = [0u8; 6];
            socket.read_exact(&mut negotiation).await.unwrap();
            socket.write_all(&[255, 253, 3]).await.unwrap();
            let mut response = [0u8; 3];
            socket.read_exact(&mut response).await.unwrap();
            assert_eq!(response, [255, 251, 3]);
            socket.write_all(b"ready\n").await.unwrap();
            let mut input = [0u8; 4];
            socket.read_exact(&mut input).await.unwrap();
            assert_eq!(&input, b"ping");
            socket.write_all(b"pong\n").await.unwrap();
        });
        let svc = make_service();
        let (tx, mut rx) = mpsc::unbounded_channel();
        svc.terminal_service.set_output_sender(tx);
        let mut cfg = make_config("telnet", "127.0.0.1");
        cfg.protocol = Protocol::Telnet;
        cfg.port = port;
        let id = svc.create_session(cfg).await.unwrap();
        svc.connect(id).await.unwrap();
        assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Connected);
        let (_, ready) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!(ready, b"ready\n");
        svc.send_data(id, b"ping").await.unwrap();
        let (_, pong) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!(pong, b"pong\n");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn telnet_notification_includes_message_and_eof_clears_state() {
        use tokio::io::AsyncWriteExt;
        let svc = make_service();
        svc.trigger_engine.create_trigger(rshell_api::types::Trigger {
            id: Uuid::new_v4(), name: "notify".into(), enabled: true,
            condition: rshell_api::types::TriggerCondition::ExactMatch("ready".into()),
            action: TriggerAction::ShowNotification("Server is ready".into()),
        }).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        svc.event_bus.subscribe(move |event| { let _ = tx.send(event.clone()); });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("telnet", "127.0.0.1");
        cfg.protocol = Protocol::Telnet;
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        svc.connect(id).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"ready").await.unwrap();
        socket.shutdown().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut notified = false;
            while let Some(event) = rx.recv().await {
                match event {
                    AppEvent::TriggerFired { action_summary, .. } => {
                        assert_eq!(action_summary, "notify: Server is ready");
                        notified = true;
                    }
                    AppEvent::ConnectionStateChanged { state: ConnectionState::Disconnected, .. } => break,
                    _ => {},
                }
            }
            assert!(notified);
            assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Disconnected);
            assert!(!svc.protocol_connections.read().await.contains_key(&id));
        }).await.unwrap();
    }

    #[tokio::test]
    async fn dispatcher_resizes_live_telnet_and_remembers_preconnect_size() {
        use crate::command_dispatcher::{CommandDispatcher, Services};
        use tokio::io::AsyncReadExt;
        let svc = Arc::new(make_service());
        let bus = svc.event_bus.clone();
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = CommandDispatcher::new(Services {
            session_service: svc.clone(), terminal_service: svc.terminal_service.clone(),
            transfer_service: Arc::new(crate::transfer::service::TransferService::new(bus.clone())),
            trigger_engine: svc.trigger_engine.clone(),
            key_manager: Arc::new(crate::security::key_manager::KeyManager::new(dir.path().join("keys"), bus.clone())),
            master_password: Arc::new(crate::security::master_password::MasterPassword::new(bus.clone())),
            tunnel_manager: Arc::new(crate::security::tunnel_manager::TunnelManager::new(bus.clone())),
            host_key_manager: Arc::new(crate::security::host_key_manager::HostKeyManager::new(dir.path().join("known_hosts"), bus.clone())),
            theme_manager: Arc::new(crate::theme::ThemeManager::new(bus.clone())),
            event_bus: bus, host_key_registry: svc.host_key_registry.clone(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("resize", "127.0.0.1");
        cfg.protocol = Protocol::Telnet;
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        dispatcher.dispatch(rshell_api::AppCommand::ResizeTerminal { session_id: id, cols: 90, rows: 30 }).await.unwrap();
        svc.connect(id).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut initial = [0; 15];
        socket.read_exact(&mut initial).await.unwrap();
        assert_eq!(&initial[6..], &[255, 250, 31, 0, 90, 0, 30, 255, 240]);
        dispatcher.dispatch(rshell_api::AppCommand::ResizeTerminal { session_id: id, cols: 100, rows: 40 }).await.unwrap();
        let mut resized = [0; 9];
        tokio::time::timeout(std::time::Duration::from_millis(300), socket.read_exact(&mut resized)).await.expect("live resize was not routed").unwrap();
        assert_eq!(resized, [255, 250, 31, 0, 100, 0, 40, 255, 240]);
        svc.disconnect(id).await.unwrap();
    }

    #[tokio::test]
    async fn old_protocol_actor_cleanup_preserves_new_connection() {
        let svc = make_service();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("reconnect", "127.0.0.1");
        cfg.protocol = Protocol::Telnet;
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        svc.connect(id).await.unwrap();
        let (_socket, _) = listener.accept().await.unwrap();
        let lifecycle = svc.lifecycle.lock().await;
        let sender = svc.protocol_connections.read().await.get(&id).unwrap().clone();
        let (reply, received) = oneshot::channel();
        sender.send(ProtocolRequest::Disconnect(reply)).await.unwrap();
        received.await.unwrap().unwrap();
        // The old actor is now queued for cleanup. Publish a replacement before
        // letting it acquire the lifecycle lock, exactly the reconnect race.
        let attempt = Uuid::new_v4();
        svc.sessions.write().await.get_mut(&id).unwrap().attempt = Some(attempt);
        let (replacement, _rx) = mpsc::channel(1);
        svc.protocol_connections.write().await.insert(id, replacement.clone());
        tokio::task::yield_now().await;
        drop(lifecycle);
        let _finished = svc.lifecycle.lock().await;
        assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Connected);
        assert!(svc.protocol_connections.read().await.get(&id).unwrap().same_channel(&replacement));
    }

    #[tokio::test]
    async fn serial_session_requires_valid_macos_device_settings() {
        let svc = make_service();
        let mut cfg = make_config("serial", "/dev/cu.test");
        cfg.protocol = Protocol::Serial;
        assert!(svc.create_session(cfg.clone()).await.is_err());
        cfg.serial_config = Some(rshell_api::types::SerialConfig {
            port: "/dev/cu.test".into(), baud_rate: 115200, data_bits: 8,
            stop_bits: 1, parity: rshell_api::types::SerialParity::None,
            flow_control: rshell_api::types::SerialFlowControl::None,
        });
        assert!(svc.create_session(cfg).await.is_ok());
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
    async fn trigger_send_after_disconnect_is_an_error() {
        let err = execute_trigger_action(&TriggerAction::SendText("clear\n".into()), "", Uuid::new_v4(), None).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn trigger_log_writes_matched_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trigger.log");
        append_trigger_log(&path, "match: hello").await.unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "match: hello\n");
        assert!(append_trigger_log(Path::new("relative.log"), "x").await.is_err());
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
