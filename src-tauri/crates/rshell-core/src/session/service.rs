//! 会话服务

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::script::trigger_engine::TriggerEngine;
use crate::security::host_key_decision::HostKeyDecisionRegistry;
use crate::session::repository::{has_credential, set_presence, SessionRepository};
use crate::terminal::service::TerminalService;
use rshell_api::types::{
    AuthMethod, ConnectionInfo, ConnectionState, CredentialUpdate, FileType, Protocol,
    RemoteFileEntry, SessionConfig, SessionCredential, SessionLoadIssue, TriggerAction,
};
use rshell_protocol::serial::{SerialConfig as ProtocolSerialConfig, SerialConnection};
use rshell_protocol::ssh::sftp::SftpClient;
use rshell_protocol::ssh::{ResolvedAuthMethod, SshClient};
use rshell_protocol::telnet::TelnetConnection;
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

/// R3-01：持有 `SshClient` 写锁执行 `disconnect_ssh()` 的时间上限。
///
/// 断开流程要 await 远端；一旦它挂住，写锁就把整个会话锁死（新标签、关闭标签、
/// 删除会话、重连、send_input 全部排队），用户只能重启应用。协议层已不再逐个
/// 等待 pty 应答，这里是防止其它未知阻塞点的最后一道兜底。
const DISCONNECT_SSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn validate_session_config(config: &SessionConfig) -> Result<(), CoreError> {
    match config.protocol {
        Protocol::SSH | Protocol::Telnet => {
            if config.host.trim().is_empty() || config.port == 0 {
                return Err(CoreError::InvalidState(
                    "Host and TCP port are required".into(),
                ));
            }
        }
        Protocol::Serial => {
            let serial = config
                .serial_config
                .as_ref()
                .ok_or_else(|| CoreError::InvalidState("Serial settings are required".into()))?;
            if !serial.port.starts_with("/dev/")
                || serial.baud_rate == 0
                || !(5..=8).contains(&serial.data_bits)
                || !(1..=2).contains(&serial.stop_bits)
            {
                return Err(CoreError::InvalidState(
                    "Invalid macOS serial configuration".into(),
                ));
            }
        }
    }
    Ok(())
}

fn resolved_auth(
    config: &SessionConfig,
    secret: Option<String>,
) -> Result<ResolvedAuthMethod, CoreError> {
    let missing = || CoreError::InvalidState("Stored session credential is unavailable".into());
    if has_credential(config) && secret.is_none() {
        return Err(missing());
    }
    match &config.auth_method {
        AuthMethod::Password { username, .. } => Ok(ResolvedAuthMethod::Password {
            username: username.clone(),
            password: secret.unwrap_or_default(),
        }),
        AuthMethod::PublicKey {
            username, key_path, ..
        } => Ok(ResolvedAuthMethod::PublicKey {
            username: username.clone(),
            key_path: key_path.clone(),
            passphrase: secret,
        }),
        AuthMethod::KeyboardInteractive { username, .. } => {
            Ok(ResolvedAuthMethod::KeyboardInteractive {
                username: username.clone(),
                password: secret,
            })
        }
    }
}

/// Destructive SFTP operations accept only an absolute, unambiguous non-root path.
pub(crate) fn validate_remote_mutation_path(path: &str) -> Result<(), CoreError> {
    if !path.starts_with('/')
        || path == "/"
        || path.ends_with('/')
        || path
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.chars().any(|c| c == '\0' || c == '\\')
    {
        return Err(CoreError::InvalidState(format!(
            "Unsafe remote file path: {path}"
        )));
    }
    Ok(())
}

async fn append_trigger_log(path: &Path, output: &str) -> Result<(), CoreError> {
    if !path.is_absolute() || path.file_name().is_none() || path.is_dir() {
        return Err(CoreError::InvalidState(
            "Trigger log path must be an absolute file".into(),
        ));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.write_all(output.as_bytes())
        .await
        .map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.write_all(b"\n")
        .await
        .map_err(|e| CoreError::StorageError(e.to_string()))?;
    file.flush()
        .await
        .map_err(|e| CoreError::StorageError(e.to_string()))
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
            let result = client
                .read()
                .await
                .send_data(text.as_bytes())
                .await
                .map_err(|e| CoreError::ConnectionError(e.to_string()));
            result
        }
        TriggerAction::ShowNotification(_) => Ok(()),
        TriggerAction::LogToFile(path) => append_trigger_log(path, output).await,
        TriggerAction::Disconnect => {
            let client = client
                .ok_or_else(|| CoreError::NotFound(format!("Connection {session_id} not found")))?;
            let result = client
                .write()
                .await
                .disconnect_ssh()
                .await
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

/// R2-15：会话删除回调（壳层据此清理 TerminalChannels 等每会话资源）
type SessionDeletedCallback = Arc<dyn Fn(Uuid) + Send + Sync>;

/// 会话服务 - 管理会话的生命周期
pub struct SessionService {
    load_issues: RwLock<Vec<SessionLoadIssue>>,
    mutations: Mutex<()>,
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
    /// R2-15：会话删除回调。壳层据此清理每会话资源（如 TerminalChannels 的
    /// 双态 sink 条目）。仅在删除时触发——断开不触发，断开后重连的终端面板
    /// 仍持有 Channel 句柄，清理会冻结其输出。
    on_session_deleted: std::sync::RwLock<Option<SessionDeletedCallback>>,
}

impl SessionService {
    pub async fn list_load_issues(&self) -> Vec<SessionLoadIssue> {
        self.load_issues.read().await.clone()
    }
    fn resolve_auth(&self, config: &SessionConfig) -> Result<ResolvedAuthMethod, CoreError> {
        let secret = match &self.repository {
            Some(repo) => repo.credential(config).map_err(|_| {
                CoreError::InvalidState("Stored session credential is unavailable".into())
            })?,
            None => None,
        };
        resolved_auth(config, secret)
    }
    /// 创建新的会话服务
    pub fn new(
        event_bus: Arc<EventBus>,
        terminal_service: Arc<TerminalService>,
        trigger_engine: Arc<TriggerEngine>,
        host_key_registry: Arc<HostKeyDecisionRegistry>,
    ) -> Self {
        Self::with_repository(
            event_bus,
            terminal_service,
            trigger_engine,
            host_key_registry,
            None,
        )
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
        let mut load_issues = Vec::new();
        if let Some(repo) = &repository {
            match repo.list_all() {
                Ok(report) => {
                    load_issues = report.issues;
                    for config in report.sessions {
                        restored.insert(
                            config.id,
                            SessionState {
                                config,
                                connection_state: ConnectionState::Disconnected,
                                connection_info: None,
                                attempt: None,
                                cancel_connect: None,
                            },
                        );
                    }
                }
                Err(_) => load_issues.push(Self::storage_load_issue()),
            }
        }
        Self {
            load_issues: RwLock::new(load_issues),
            mutations: Mutex::new(()),
            lifecycle: Arc::new(Mutex::new(())),
            sessions: Arc::new(RwLock::new(restored)),
            connections: Arc::new(RwLock::new(HashMap::new())),
            protocol_connections: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            terminal_service,
            trigger_engine,
            host_key_registry,
            repository,
            on_session_deleted: std::sync::RwLock::new(None),
        }
    }

    /// R2-15：注册会话删除回调（壳层用它清理 TerminalChannels 的 sink 条目）。
    /// 同步 setter：Tauri setup 阶段（不能嵌套 block_on）即可注册。
    pub fn set_on_session_deleted(&self, callback: SessionDeletedCallback) {
        *self
            .on_session_deleted
            .write()
            .expect("on_session_deleted lock poisoned") = Some(callback);
    }

    fn storage_load_issue() -> SessionLoadIssue {
        SessionLoadIssue {
            session_id: None,
            message: "Could not read saved sessions. Check storage access, then retry.".into(),
        }
    }

    /// Retry failed loads without replacing an existing live session state.
    #[instrument(skip(self))]
    pub async fn load_from_disk(&self) {
        let _mutation = self.mutations.lock().await;
        let Some(repo) = self.repository.as_ref() else {
            debug!("load_from_disk: no repository configured, skip");
            return;
        };
        let report = match repo.list_all() {
            Ok(v) => v,
            Err(_) => {
                *self.load_issues.write().await = vec![Self::storage_load_issue()];
                return;
            }
        };
        *self.load_issues.write().await = report.issues;
        let mut sessions = self.sessions.write().await;
        for cfg in report.sessions {
            let id = cfg.id;
            sessions.entry(id).or_insert_with(|| SessionState {
                config: cfg,
                connection_state: ConnectionState::Disconnected,
                connection_info: None,
                attempt: None,
                cancel_connect: None,
            });
            debug!(session_id = %id, "load_from_disk: restored");
        }
        info!(count = sessions.len(), "load_from_disk complete");
        self.event_bus
            .publish(rshell_api::AppEvent::SessionListChanged);
    }

    /// 连接到会话
    #[instrument(skip(self))]
    pub async fn connect(&self, session_id: Uuid) -> Result<(), CoreError> {
        info!(session_id = %session_id, "Connecting session");
        let attempt = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let config = self
            .claim_connection(session_id, attempt, cancel_tx)
            .await?;
        let result = tokio::select! {
            result = self.connect_attempt(session_id, config, attempt) => result,
            _ = cancel_rx => Err(CoreError::InvalidState("Connection attempt cancelled".into())),
        };
        if result.is_err() {
            let _lifecycle = self.lifecycle.lock().await;
            let mut sessions = self.sessions.write().await;
            if let Some(state) = sessions
                .get_mut(&session_id)
                .filter(|state| state.attempt == Some(attempt))
            {
                state.attempt = None;
                state.cancel_connect = None;
                state.connection_state = ConnectionState::Disconnected;
                state.connection_info = None;
                self.event_bus
                    .publish(rshell_api::AppEvent::ConnectionStateChanged {
                        session_id,
                        state: ConnectionState::Disconnected,
                        info: None,
                    });
            }
        }
        result
    }

    /// Claim and snapshot before network I/O; lifecycle mutations coordinate here.
    async fn claim_connection(
        &self,
        session_id: Uuid,
        attempt: Uuid,
        cancel_tx: oneshot::Sender<()>,
    ) -> Result<SessionConfig, CoreError> {
        let _lifecycle = self.lifecycle.lock().await;
        let mut sessions = self.sessions.write().await;
        let state = sessions
            .get_mut(&session_id)
            .ok_or_else(|| CoreError::NotFound(format!("Session {} not found", session_id)))?;
        if matches!(
            state.connection_state,
            ConnectionState::Connecting | ConnectionState::Connected
        ) {
            return Err(CoreError::InvalidState(format!(
                "Session {session_id} is already connecting or connected"
            )));
        }
        state.connection_state = ConnectionState::Connecting;
        state.attempt = Some(attempt);
        state.cancel_connect = Some(cancel_tx);
        self.event_bus
            .publish(rshell_api::AppEvent::ConnectionStateChanged {
                session_id,
                state: ConnectionState::Connecting,
                info: None,
            });
        Ok(state.config.clone())
    }

    async fn connect_attempt(
        &self,
        session_id: Uuid,
        config: SessionConfig,
        attempt: Uuid,
    ) -> Result<(), CoreError> {
        if config.protocol != Protocol::SSH {
            return self.connect_protocol(session_id, config, attempt).await;
        }

        // 创建 SSH 客户端并连接 — 通过 host_key_registry 接入 host key 决策通道:
        // 遇到未知 host key 时,SshHandler::check_server_key 会在 EventBus 上发
        // HostKeyMismatch { decision_id, ... } 然后同步 block_on 等 UI 端的
        // AppCommand::DecideHostKey。
        let auth = self.resolve_auth(&config)?;
        let mut client = SshClient::new(config.clone(), auth);
        let sink: Arc<dyn rshell_protocol::ssh::HostKeyDecisionSink> =
            self.host_key_registry.clone();

        match client.connect_ssh(Some(sink)).await {
            Ok(()) => {
                info!(session_id = %session_id, "SSH connection established");
                if let Some((cols, rows)) = self.terminal_service.size(session_id) {
                    if let Err(e) = client.resize_terminal(cols as u32, rows as u32).await {
                        warn!(session_id = %session_id, error = %e, "initial terminal resize failed");
                    }
                }

                let mut output_rx = client
                    .take_data_receiver()
                    .ok_or_else(|| CoreError::InvalidState("SSH output channel missing".into()))?;
                // 主 pty 以 session_id 寻址（首标签不传 terminal_id）。
                // 附加标签用各自的 terminal_id，因此输出能按 pty 分流。
                let primary_terminal = session_id;
                // 创建取消通道
                let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                // 包装客户端为 Arc<RwLock>
                let client = Arc::new(tokio::sync::RwLock::new(client));

                let publication = self.lifecycle.lock().await;
                if !self
                    .sessions
                    .read()
                    .await
                    .get(&session_id)
                    .is_some_and(|state| state.attempt == Some(attempt))
                {
                    return Err(CoreError::InvalidState(
                        "Connection attempt cancelled".into(),
                    ));
                }

                // 保存活动连接
                {
                    let mut connections = self.connections.write().await;
                    connections.insert(
                        session_id,
                        ActiveConnection {
                            client: client.clone(),
                            _cancel_tx: cancel_tx,
                        },
                    );
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
                self.event_bus
                    .publish(rshell_api::AppEvent::ConnectionStateChanged {
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
                                        if let Err(e) = terminal_service.push_output(session_id, primary_terminal, data.clone()) {
                                            warn!(session_id = %session_id, terminal_id = %primary_terminal, error = %e, "terminal output delivery failed");
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
                    if let Some(state) = states
                        .get_mut(&session_id)
                        .filter(|state| state.attempt == Some(attempt))
                    {
                        connections.write().await.remove(&session_id);
                        state.attempt = None;
                        state.connection_state = ConnectionState::Disconnected;
                        state.connection_info = None;
                        event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                            session_id,
                            state: ConnectionState::Disconnected,
                            info: None,
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

    async fn connect_protocol(
        &self,
        session_id: Uuid,
        config: SessionConfig,
        attempt: Uuid,
    ) -> Result<(), CoreError> {
        let connection: Result<Box<dyn Connection>, CoreError> = match config.protocol {
            Protocol::Telnet => Ok(Box::new(TelnetConnection::new(&config.host, config.port))),
            Protocol::Serial => config
                .serial_config
                .as_ref()
                .map(|serial| {
                    Box::new(SerialConnection::new(ProtocolSerialConfig {
                        port: serial.port.clone(),
                        baud_rate: serial.baud_rate,
                        data_bits: serial.data_bits,
                        stop_bits: serial.stop_bits,
                        parity: match serial.parity {
                            rshell_api::types::SerialParity::None => {
                                rshell_protocol::serial::SerialParity::None
                            }
                            rshell_api::types::SerialParity::Even => {
                                rshell_protocol::serial::SerialParity::Even
                            }
                            rshell_api::types::SerialParity::Odd => {
                                rshell_protocol::serial::SerialParity::Odd
                            }
                        },
                        flow_control: match serial.flow_control {
                            rshell_api::types::SerialFlowControl::None => {
                                rshell_protocol::serial::SerialFlowControl::None
                            }
                            rshell_api::types::SerialFlowControl::Software => {
                                rshell_protocol::serial::SerialFlowControl::Software
                            }
                            rshell_api::types::SerialFlowControl::Hardware => {
                                rshell_protocol::serial::SerialFlowControl::Hardware
                            }
                        },
                    })) as Box<dyn Connection>
                })
                .ok_or_else(|| CoreError::InvalidState("Serial settings are missing".into())),
            Protocol::SSH => unreachable!(),
        };
        let mut connection = connection?;
        connection
            .connect()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        if let Some((cols, rows)) = self.terminal_service.size(session_id) {
            if let Err(e) = connection.resize(cols, rows).await {
                warn!(session_id = %session_id, error = %e, "initial terminal resize failed");
            }
        }
        let info = ConnectionInfo {
            protocol: config.protocol,
            host: config.host.clone(),
            port: config.port,
            state: ConnectionState::Connected,
            bytes_sent: 0,
            bytes_received: 0,
            latency_ms: None,
        };
        let (tx, mut rx) = mpsc::channel::<ProtocolRequest>(32);
        let publication = self.lifecycle.lock().await;
        if !self
            .sessions
            .read()
            .await
            .get(&session_id)
            .is_some_and(|state| state.attempt == Some(attempt))
        {
            return Err(CoreError::InvalidState(
                "Connection attempt cancelled".into(),
            ));
        }
        self.protocol_connections
            .write()
            .await
            .insert(session_id, tx);
        if let Some(state) = self.sessions.write().await.get_mut(&session_id) {
            state.connection_state = ConnectionState::Connected;
            state.cancel_connect = None;
            state.connection_info = Some(info.clone());
        }
        self.event_bus
            .publish(rshell_api::AppEvent::ConnectionStateChanged {
                session_id,
                state: ConnectionState::Connected,
                info: Some(info),
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
                            // Telnet/Serial 一条连接只有一个 pty，沿用 session_id 作为
                            // terminal 键（前端首标签的 terminal_id 就是 session_id）
                            if let Err(e) = terminal_service.push_output(session_id, session_id, data.clone()) {
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
            if let Some(state) = sessions
                .get_mut(&session_id)
                .filter(|state| state.attempt == Some(attempt))
            {
                connections.write().await.remove(&session_id);
                state.attempt = None;
                state.connection_state = ConnectionState::Disconnected;
                state.connection_info = None;
                event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: ConnectionState::Disconnected,
                    info: None,
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
            let state = sessions
                .get_mut(&session_id)
                .ok_or_else(|| CoreError::NotFound(format!("Session {session_id} not found")))?;
            state.attempt = None;
            if let Some(cancel) = state.cancel_connect.take() {
                let _ = cancel.send(());
            }
            state.connection_state = ConnectionState::Disconnected;
            state.connection_info = None;
            let sender = self.protocol_connections.write().await.remove(&session_id);
            let active = self.connections.write().await.remove(&session_id);
            if delete {
                sessions.remove(&session_id);
            }
            self.event_bus
                .publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: ConnectionState::Disconnected,
                    info: None,
                });
            (sender, active)
        };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(ProtocolRequest::Disconnect(reply_tx))
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            reply_rx
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))??;
            return Ok(());
        }

        if let Some(conn) = active {
            // 发送取消信号
            let _ = conn._cancel_tx.send(()).await;
            // 断开 SSH 连接。
            //
            // R3-01：这里持 `SshClient` **写锁**跨越 `disconnect_ssh()`，而后者
            // 会 await 远端。只要断开流程有可能挂住，写锁就永远不释放，新标签、
            // 关闭标签、删除会话、重连与每一次 send_input 会被一起卡死。
            // 协议层已不再逐个等待 pty 应答，这里再加一道兜底上限：最坏情况下
            // 只是放弃优雅关闭，锁仍会按时释放。
            let mut client = conn.client.write().await;
            match tokio::time::timeout(DISCONNECT_SSH_TIMEOUT, client.disconnect_ssh()).await {
                Ok(Err(e)) => {
                    warn!(session_id = %session_id, error = %e, "Error disconnecting SSH");
                }
                Ok(Ok(())) => {}
                Err(_) => {
                    warn!(
                        session_id = %session_id,
                        "disconnect_ssh exceeded {:?}; releasing the client lock anyway",
                        DISCONNECT_SSH_TIMEOUT
                    );
                }
            }
        }

        info!(session_id = %session_id, "Session disconnected");
        Ok(())
    }

    /// 发送数据到会话
    pub async fn send_data(
        &self,
        session_id: Uuid,
        terminal_id: Option<Uuid>,
        data: &[u8],
    ) -> Result<(), CoreError> {
        let sender = {
            self.protocol_connections
                .read()
                .await
                .get(&session_id)
                .cloned()
        };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(ProtocolRequest::Send(data.to_vec(), reply_tx))
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            return reply_rx
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
        }
        // 先在锁内克隆出 client 句柄，立刻释放 connections guard，避免跨 await 持锁
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!("Connection {} not found", session_id))
                })?
        };
        let client = client.read().await;
        // 指定 terminal_id 就投到那个 pty；None = 主 pty（首标签）
        if let Some(id) = terminal_id {
            client
                .send_data_to(id, data)
                .await
                .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
            return Ok(());
        }
        client
            .send_data(data)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// 调整终端大小
    pub async fn resize_terminal(
        &self,
        session_id: Uuid,
        terminal_id: Option<Uuid>,
        cols: u32,
        rows: u32,
    ) -> Result<(), CoreError> {
        let sender = {
            self.protocol_connections
                .read()
                .await
                .get(&session_id)
                .cloned()
        };
        if let Some(sender) = sender {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(ProtocolRequest::Resize(cols as u16, rows as u16, reply_tx))
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
            return reply_rx
                .await
                .map_err(|_| CoreError::ConnectionError("Protocol connection closed".into()))?;
        }
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!("Connection {} not found", session_id))
                })?
        };
        let client = client.read().await;
        if let Some(id) = terminal_id {
            client
                .resize_terminal_of(id, cols, rows)
                .await
                .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
            return Ok(());
        }
        client
            .resize_terminal(cols, rows)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// 在已连接的会话上另开一个 pty（= 新增一个独立标签会话）。
    ///
    /// 复用同一条 SSH 连接（不再握手、不再校验主机密钥），只新开一条
    /// session channel + pty + shell。为它单独起一个输出读取任务，
    /// 按 `terminal_id` 投递给壳层，因此各标签的输入/输出互不串台。
    pub async fn open_terminal(
        &self,
        session_id: Uuid,
        terminal_id: Uuid,
        cols: u32,
        rows: u32,
    ) -> Result<(), CoreError> {
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!("Connection {} not found", session_id))
                })?
        };
        let mut output_rx = {
            let mut guard = client.write().await;
            guard
                .open_terminal(terminal_id, cols, rows)
                .await
                .map_err(|e| CoreError::ConnectionError(e.to_string()))?
        };
        self.terminal_service
            .create_terminal(terminal_id, cols as u16, rows as u16)?;

        let terminal_service = self.terminal_service.clone();
        tokio::spawn(async move {
            while let Some(data) = output_rx.recv().await {
                if let Err(e) = terminal_service.push_output(session_id, terminal_id, data) {
                    warn!(session_id = %session_id, terminal_id = %terminal_id, error = %e, "extra pty output delivery failed");
                }
            }
            debug!(session_id = %session_id, terminal_id = %terminal_id, "extra pty output stream ended");
        });
        debug!(session_id = %session_id, terminal_id = %terminal_id, "extra pty opened");
        Ok(())
    }

    /// 关闭一个 pty（标签关闭）。未知的 terminal_id 视为已关闭，幂等。
    pub async fn close_terminal(
        &self,
        session_id: Uuid,
        terminal_id: Uuid,
    ) -> Result<(), CoreError> {
        let client = {
            let connections = self.connections.read().await;
            connections
                .get(&session_id)
                .map(|c| c.client.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!("Connection {} not found", session_id))
                })?
        };
        let mut guard = client.write().await;
        guard.close_terminal(terminal_id).await;
        drop(guard);
        let _ = self.terminal_service.destroy_terminal(terminal_id);
        Ok(())
    }

    /// 创建会话
    #[instrument(skip(self, config))]
    pub async fn create_session(&self, config: SessionConfig) -> Result<Uuid, CoreError> {
        self.create_session_with_credential(config, None).await
    }

    #[instrument(skip(self, config, credential))]
    pub async fn create_session_with_credential(
        &self,
        mut config: SessionConfig,
        credential: Option<SessionCredential>,
    ) -> Result<Uuid, CoreError> {
        validate_session_config(&config)?;
        let _mutation = self.mutations.lock().await;
        let _lifecycle = self.lifecycle.lock().await;
        let id = config.id;
        if self.sessions.read().await.contains_key(&id) {
            return Err(CoreError::InvalidState("Session already exists".into()));
        }
        set_presence(
            &mut config,
            credential.as_ref().is_some_and(|c| !c.secret.is_empty()),
        );
        if self.repository.is_none() && has_credential(&config) {
            return Err(CoreError::StorageError(
                "Credential repository is required".into(),
            ));
        }
        info!(session_id = %id, name = %config.name, "Creating session");

        // 切片 1.0：先落盘再入内存。落盘失败时阻断 create —— 避免出现
        // "内存有但磁盘无"的不可恢复分裂状态（设计 §4.5 完成判据前提）。
        if let Some(repo) = self.repository.as_ref() {
            repo.create(&config, credential).map_err(|e| {
                CoreError::StorageError(format!("save session {} failed: {}", id, e))
            })?;
        }

        let state = SessionState {
            config,
            connection_state: ConnectionState::Disconnected,
            connection_info: None,
            attempt: None,
            cancel_connect: None,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(id, state);

        self.event_bus
            .publish(rshell_api::AppEvent::SessionListChanged);

        debug!(session_id = %id, "Session created");
        Ok(id)
    }

    /// 更新会话
    #[instrument(skip(self, config))]
    pub async fn update_session(&self, id: Uuid, config: SessionConfig) -> Result<(), CoreError> {
        self.update_session_with_credential(id, config, CredentialUpdate::Keep)
            .await
    }

    #[instrument(skip(self, config, credential))]
    pub async fn update_session_with_credential(
        &self,
        id: Uuid,
        mut config: SessionConfig,
        credential: CredentialUpdate,
    ) -> Result<(), CoreError> {
        validate_session_config(&config)?;
        let _mutation = self.mutations.lock().await;
        if id != config.id {
            return Err(CoreError::InvalidState(
                "Session ID cannot be changed".into(),
            ));
        }
        let _lifecycle = self.lifecycle.lock().await;
        let mut sessions = self.sessions.write().await;
        let state = sessions
            .get_mut(&id)
            .ok_or_else(|| CoreError::NotFound(format!("Session {id} not found")))?;
        // connect() has captured the destination but may not have resolved its
        // credential yet. Keep that pair stable until the attempt completes.
        if state.connection_state == ConnectionState::Connecting {
            return Err(CoreError::InvalidState(
                "Cannot update a session while it is connecting".into(),
            ));
        }
        let present = match &credential {
            CredentialUpdate::Keep => has_credential(&state.config),
            CredentialUpdate::Set(credential) => !credential.secret.is_empty(),
            CredentialUpdate::Clear => false,
        };
        set_presence(&mut config, present);
        if self.repository.is_none() && present {
            return Err(CoreError::StorageError(
                "Credential repository is required".into(),
            ));
        }
        info!(session_id = %id, "Updating session");

        if let Some(repo) = self.repository.as_ref() {
            repo.save(&config, credential).map_err(|e| {
                CoreError::StorageError(format!("save session {} failed: {}", id, e))
            })?;
        }

        state.config = config;
        drop(sessions);

        self.event_bus
            .publish(rshell_api::AppEvent::SessionUpdated { session_id: id });

        debug!(session_id = %id, "Session updated");
        Ok(())
    }

    /// 删除会话
    #[instrument(skip(self))]
    pub async fn delete_session(&self, id: Uuid) -> Result<(), CoreError> {
        let _mutation = self.mutations.lock().await;
        info!(session_id = %id, "Deleting session");

        let mut cleanup_error = None;
        if let Some(repo) = self.repository.as_ref() {
            if let Err(error) = repo.delete(id) {
                let error = CoreError::StorageError(format!("delete session {id} failed: {error}"));
                // Metadata deletion is committed before Keychain cleanup. Do
                // not leave a connectable in-memory session after that commit.
                if !matches!(repo.load(id), Ok(None)) {
                    return Err(error);
                }
                cleanup_error = Some(error);
            }
        }

        // Invalidate and remove atomically; a concurrent reconnect cannot slip
        // between disconnect and deletion and publish into a deleted session.
        let _ = self.detach_session(id, true).await;

        // R2-15：会话已删除，通知壳层清理该会话的终端 sink 条目等资源。
        // 注意：仅删除触发；断开路径不触发（重连的终端面板仍持有 Channel）。
        if let Some(callback) = self
            .on_session_deleted
            .read()
            .expect("on_session_deleted lock poisoned")
            .as_ref()
        {
            callback(id);
        }

        self.event_bus
            .publish(rshell_api::AppEvent::SessionListChanged);

        debug!(session_id = %id, "Session deleted");
        match cleanup_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
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
    pub async fn get_connection_info(
        &self,
        session_id: Uuid,
    ) -> Result<Option<ConnectionInfo>, CoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .get(&session_id)
            .and_then(|s| s.connection_info.clone()))
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

    /// 远端用户的工作目录（登录后默认所在目录）
    pub async fn remote_home_dir(&self, session_id: Uuid) -> Result<String, CoreError> {
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;
        let channel = ssh
            .open_sftp_channel()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let sftp = SftpClient::new(channel)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        sftp.home_dir()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))
    }

    pub async fn create_remote_directory(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<(), CoreError> {
        validate_remote_mutation_path(path)?;
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;
        let channel = ssh
            .open_sftp_channel()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let sftp = SftpClient::new(channel)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        sftp.create_dir(path)
            .await
            .map_err(|e| CoreError::ServiceError(e.to_string()))
    }

    pub async fn delete_remote_entry(&self, session_id: Uuid, path: &str) -> Result<(), CoreError> {
        validate_remote_mutation_path(path)?;
        let client = self.get_ssh_client(session_id).await?;
        let ssh = client.read().await;
        let channel = ssh
            .open_sftp_channel()
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let sftp = SftpClient::new(channel)
            .await
            .map_err(|e| CoreError::ConnectionError(e.to_string()))?;
        let entry = sftp
            .metadata(path)
            .await
            .map_err(|e| CoreError::ServiceError(e.to_string()))?;
        if entry.file_type != FileType::File {
            return Err(CoreError::InvalidState(
                "Only regular remote files can be deleted".into(),
            ));
        }
        sftp.remove_file(path)
            .await
            .map_err(|e| CoreError::ServiceError(e.to_string()))
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
                has_password: true,
            },
            serial_config: None,
        }
    }

    #[test]
    fn unresolved_stored_credential_fails_closed_but_intentional_empty_password_is_valid() {
        let mut config = make_config("credential", "example.test");
        assert!(matches!(
            make_service().resolve_auth(&config),
            Err(CoreError::InvalidState(_))
        ));
        config.auth_method = AuthMethod::Password {
            username: "user".into(),
            has_password: false,
        };
        assert!(
            matches!(make_service().resolve_auth(&config), Ok(ResolvedAuthMethod::Password { password, .. }) if password.is_empty())
        );
        config.auth_method = AuthMethod::PublicKey {
            username: "user".into(),
            key_path: "/tmp/id_ed25519".into(),
            has_passphrase: true,
        };
        assert!(matches!(
            make_service().resolve_auth(&config),
            Err(CoreError::InvalidState(_))
        ));
        config.auth_method = AuthMethod::PublicKey {
            username: "user".into(),
            key_path: "/tmp/id_ed25519".into(),
            has_passphrase: false,
        };
        assert!(matches!(
            make_service().resolve_auth(&config),
            Ok(ResolvedAuthMethod::PublicKey {
                passphrase: None,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn keyboard_interactive_resolves_only_when_password_is_declared() {
        use crate::session::repository::tests::{set, MemoryCredentials};
        use rshell_api::credentials::{CredentialKey, CredentialKind};
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            dir.path().into(),
            credentials.clone(),
        ));
        let svc = make_service_with_repo(repo.clone());
        let mut cfg = make_config("interactive", "example.test");
        cfg.auth_method = AuthMethod::KeyboardInteractive {
            username: "interactive-user".into(),
            has_password: true,
        };
        assert!(svc.resolve_auth(&cfg).is_err());
        repo.save(&cfg, set("interactive-secret")).unwrap();
        assert!(
            matches!(svc.resolve_auth(&cfg), Ok(ResolvedAuthMethod::KeyboardInteractive { username, password: Some(password) }) if username == "interactive-user" && password == "interactive-secret")
        );
        credentials.entries.lock().unwrap().remove(&CredentialKey {
            session_id: cfg.id,
            kind: CredentialKind::Password,
        });
        assert!(svc.resolve_auth(&cfg).is_err());
        set_presence(&mut cfg, false);
        *credentials.fail.lock().unwrap() = true;
        assert!(
            matches!(svc.resolve_auth(&cfg), Ok(ResolvedAuthMethod::KeyboardInteractive { username, password: None }) if username == "interactive-user")
        );
    }

    #[tokio::test]
    async fn disconnect_cancels_pending_handshake_and_allows_reconnect() {
        let svc = Arc::new(make_service());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("pending", "127.0.0.1");
        cfg.auth_method = AuthMethod::Password {
            username: "user".into(),
            has_password: false,
        };
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        let first = {
            let svc = svc.clone();
            tokio::spawn(async move { svc.connect(id).await })
        };
        let (_socket, _) = listener.accept().await.unwrap();
        svc.disconnect(id).await.unwrap();
        let second = {
            let svc = svc.clone();
            tokio::spawn(async move { svc.connect(id).await })
        };
        let (_second_socket, _) = listener.accept().await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(300), first)
                .await
                .expect("cancel must end pending handshake")
                .unwrap()
                .is_err()
        );
        assert_eq!(
            svc.get_state(id).await.unwrap(),
            ConnectionState::Connecting
        );
        svc.delete_session(id).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(300), second)
                .await
                .expect("delete must cancel handshake")
                .unwrap()
                .is_err()
        );
        assert!(svc.get_state(id).await.is_err());
        assert!(!svc.connections.read().await.contains_key(&id));
    }

    #[tokio::test]
    async fn protocol_requests_release_lookup_lock_before_waiting_for_reply() {
        for operation in 0..3 {
            let svc = Arc::new(make_service());
            let id = svc
                .create_session(make_config("race", "host"))
                .await
                .unwrap();
            let (tx, mut rx) = mpsc::channel(1);
            svc.protocol_connections.write().await.insert(id, tx);
            let caller = {
                let svc = svc.clone();
                tokio::spawn(async move {
                    match operation {
                        0 => svc.send_data(id, None, b"x").await,
                        1 => svc.resize_terminal(id, None, 90, 30).await,
                        _ => svc.disconnect(id).await,
                    }
                })
            };
            let request = rx.recv().await.unwrap();
            // EOF cleanup needs this write lock before the request's reply is dropped.
            let cleanup = async {
                svc.protocol_connections.write().await.remove(&id);
                drop(request);
                drop(rx);
                assert!(caller.await.unwrap().is_err());
            };
            tokio::time::timeout(std::time::Duration::from_millis(300), cleanup)
                .await
                .expect("EOF/request race deadlocked");
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
        let (_, _, ready) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready, b"ready\n");
        svc.send_data(id, None, b"ping").await.unwrap();
        let (_, _, pong) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pong, b"pong\n");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn telnet_notification_includes_message_and_eof_clears_state() {
        use tokio::io::AsyncWriteExt;
        let svc = make_service();
        svc.trigger_engine
            .create_trigger(rshell_api::types::Trigger {
                id: Uuid::new_v4(),
                name: "notify".into(),
                enabled: true,
                condition: rshell_api::types::TriggerCondition::ExactMatch("ready".into()),
                action: TriggerAction::ShowNotification("Server is ready".into()),
            })
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        svc.event_bus.subscribe(move |event| {
            let _ = tx.send(event.clone());
        });
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
                    AppEvent::ConnectionStateChanged {
                        state: ConnectionState::Disconnected,
                        ..
                    } => break,
                    _ => {}
                }
            }
            assert!(notified);
            assert_eq!(
                svc.get_state(id).await.unwrap(),
                ConnectionState::Disconnected
            );
            assert!(!svc.protocol_connections.read().await.contains_key(&id));
        })
        .await
        .unwrap();
    }

    fn make_dispatcher(
        svc: Arc<SessionService>,
        path: &Path,
    ) -> crate::command_dispatcher::CommandDispatcher {
        use crate::command_dispatcher::{CommandDispatcher, Services};
        let bus = svc.event_bus.clone();
        CommandDispatcher::new(Services {
            session_service: svc.clone(),
            terminal_service: svc.terminal_service.clone(),
            transfer_service: Arc::new(crate::transfer::service::TransferService::new(bus.clone())),
            trigger_engine: svc.trigger_engine.clone(),
            key_manager: Arc::new(crate::security::key_manager::KeyManager::new(
                path.join("keys"),
                bus.clone(),
            )),
            master_password: Arc::new(crate::security::master_password::MasterPassword::new(
                bus.clone(),
            )),
            tunnel_manager: Arc::new(crate::security::tunnel_manager::TunnelManager::new(
                bus.clone(),
            )),
            host_key_manager: Arc::new(crate::security::host_key_manager::HostKeyManager::new(
                path.join("known_hosts"),
            )),
            theme_manager: Arc::new(crate::theme::ThemeManager::new(bus.clone())),
            event_bus: bus,
            host_key_registry: svc.host_key_registry.clone(),
        })
    }

    #[tokio::test]
    async fn dispatcher_resizes_live_telnet_and_remembers_preconnect_size() {
        use tokio::io::AsyncReadExt;
        let svc = Arc::new(make_service());
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(svc.clone(), dir.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = make_config("resize", "127.0.0.1");
        cfg.protocol = Protocol::Telnet;
        cfg.port = listener.local_addr().unwrap().port();
        let id = svc.create_session(cfg).await.unwrap();
        dispatcher
            .dispatch(rshell_api::AppCommand::ResizeTerminal {
                session_id: id,
                terminal_id: None,
                cols: 90,
                rows: 30,
            })
            .await
            .unwrap();
        svc.connect(id).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut initial = [0; 15];
        socket.read_exact(&mut initial).await.unwrap();
        assert_eq!(&initial[6..], &[255, 250, 31, 0, 90, 0, 30, 255, 240]);
        dispatcher
            .dispatch(rshell_api::AppCommand::ResizeTerminal {
                session_id: id,
                terminal_id: None,
                cols: 100,
                rows: 40,
            })
            .await
            .unwrap();
        let mut resized = [0; 9];
        tokio::time::timeout(
            std::time::Duration::from_millis(300),
            socket.read_exact(&mut resized),
        )
        .await
        .expect("live resize was not routed")
        .unwrap();
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
        let sender = svc
            .protocol_connections
            .read()
            .await
            .get(&id)
            .unwrap()
            .clone();
        let (reply, received) = oneshot::channel();
        sender
            .send(ProtocolRequest::Disconnect(reply))
            .await
            .unwrap();
        received.await.unwrap().unwrap();
        // The old actor is now queued for cleanup. Publish a replacement before
        // letting it acquire the lifecycle lock, exactly the reconnect race.
        let attempt = Uuid::new_v4();
        svc.sessions.write().await.get_mut(&id).unwrap().attempt = Some(attempt);
        let (replacement, _rx) = mpsc::channel(1);
        svc.protocol_connections
            .write()
            .await
            .insert(id, replacement.clone());
        tokio::task::yield_now().await;
        drop(lifecycle);
        let _finished = svc.lifecycle.lock().await;
        assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Connected);
        assert!(svc
            .protocol_connections
            .read()
            .await
            .get(&id)
            .unwrap()
            .same_channel(&replacement));
    }

    #[tokio::test]
    async fn serial_session_requires_valid_macos_device_settings() {
        let svc = make_service();
        let mut cfg = make_config("serial", "/dev/cu.test");
        cfg.protocol = Protocol::Serial;
        assert!(svc.create_session(cfg.clone()).await.is_err());
        cfg.serial_config = Some(rshell_api::types::SerialConfig {
            port: "/dev/cu.test".into(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: rshell_api::types::SerialParity::None,
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
        let err = svc
            .send_data(Uuid::new_v4(), None, b"hi")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_resize_terminal_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc
            .resize_terminal(Uuid::new_v4(), None, 80, 24)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn test_browse_remote_dir_unknown_returns_not_found() {
        let svc = make_service();
        let err = svc
            .browse_remote_dir(Uuid::new_v4(), "/")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    // ===== R2-15：会话删除回调（壳层据此清理 TerminalChannels sink 条目）=====

    /// 验收：会话删除后壳层收到回调（据此移除 terminal_channels 中该
    /// session_id 的条目——Buffering 固定预分配 256KiB，不能随历史会话累积）。
    #[tokio::test]
    async fn delete_session_fires_on_session_deleted_callback() {
        let svc = make_service();
        let deleted: Arc<std::sync::Mutex<Vec<Uuid>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = deleted.clone();
        svc.set_on_session_deleted(Arc::new(move |id| sink.lock().unwrap().push(id)));

        let id = svc.create_session(make_config("a", "host1")).await.unwrap();
        svc.delete_session(id).await.unwrap();

        assert_eq!(*deleted.lock().unwrap(), vec![id], "删除必须触发回调");
    }

    /// 设计决策：断开（非删除）不触发回调——断开后重连的终端面板仍持有
    /// Channel 句柄，若此时清理 sink 条目，重连后输出将无法送达该面板。
    #[tokio::test]
    async fn disconnect_does_not_fire_on_session_deleted_callback() {
        let svc = make_service();
        let deleted: Arc<std::sync::Mutex<Vec<Uuid>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = deleted.clone();
        svc.set_on_session_deleted(Arc::new(move |id| sink.lock().unwrap().push(id)));

        let id = svc.create_session(make_config("a", "host1")).await.unwrap();
        // 未连接会话上的断开是无害的 no-op，走与真实断开相同的 detach 路径
        let _ = svc.disconnect(id).await;
        assert!(deleted.lock().unwrap().is_empty(), "断开不得触发删除回调");

        // 随后真正删除才触发
        svc.delete_session(id).await.unwrap();
        assert_eq!(*deleted.lock().unwrap(), vec![id]);
    }

    #[tokio::test]
    async fn trigger_send_after_disconnect_is_an_error() {
        let err = execute_trigger_action(
            &TriggerAction::SendText("clear\n".into()),
            "",
            Uuid::new_v4(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn trigger_log_writes_matched_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trigger.log");
        append_trigger_log(&path, "match: hello").await.unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "match: hello\n");
        assert!(append_trigger_log(Path::new("relative.log"), "x")
            .await
            .is_err());
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
    async fn credentials_survive_restart_without_appearing_in_lists() {
        use crate::session::repository::tests::MemoryCredentials;
        use rshell_api::types::{CredentialUpdate, SessionCredential};
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            tmp.path().into(),
            credentials.clone(),
        ));
        let svc = make_service_with_repo(repo.clone());
        let cfg = make_config("secure", "example.test");
        let id = svc
            .create_session_with_credential(
                cfg,
                Some(SessionCredential {
                    secret: "lifecycle-secret".into(),
                }),
            )
            .await
            .unwrap();
        let mut listed = svc.list_sessions().await.unwrap().pop().unwrap();
        assert!(!serde_json::to_string(&listed)
            .unwrap()
            .contains("lifecycle-secret"));
        listed.name = "renamed".into();
        svc.update_session_with_credential(id, listed, CredentialUpdate::Keep)
            .await
            .unwrap();
        let restarted = make_service_with_repo(repo);
        let cfg = restarted.list_sessions().await.unwrap().pop().unwrap();
        assert!(
            matches!(restarted.resolve_auth(&cfg), Ok(ResolvedAuthMethod::Password { password, .. }) if password == "lifecycle-secret")
        );
        restarted.delete_session(id).await.unwrap();
        assert!(credentials.entries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn credential_free_creates_do_not_require_keychain() {
        use crate::session::repository::tests::MemoryCredentials;
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        *credentials.fail.lock().unwrap() = true;
        let repo = Arc::new(SessionRepository::new(tmp.path().into(), credentials));
        let svc = make_service_with_repo(repo);
        for protocol in [Protocol::Telnet, Protocol::Serial, Protocol::SSH] {
            for credential in [
                None,
                Some(SessionCredential {
                    secret: String::new(),
                }),
            ] {
                let mut cfg = make_config("no secret", "localhost");
                cfg.protocol = protocol;
                if protocol == Protocol::Serial {
                    cfg.serial_config = Some(rshell_api::types::SerialConfig {
                        port: "/dev/cu.test".into(),
                        baud_rate: 115200,
                        data_bits: 8,
                        stop_bits: 1,
                        parity: rshell_api::types::SerialParity::None,
                        flow_control: rshell_api::types::SerialFlowControl::None,
                    });
                }
                let id = svc
                    .create_session_with_credential(cfg, credential)
                    .await
                    .unwrap();
                let stored = svc
                    .list_sessions()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|s| s.id == id)
                    .unwrap();
                assert!(!has_credential(&stored));
            }
        }
        assert_eq!(svc.list_sessions().await.unwrap().len(), 6);
    }

    #[tokio::test]
    async fn migration_issues_survive_startup_and_retry_without_exposing_secrets() {
        use crate::session::repository::tests::MemoryCredentials;
        use rshell_api::{AppCommand, CommandOutcome};
        let tmp = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let path = tmp.path().join(format!("{id}.toml"));
        let bytes = format!("id = '{id}'\nname = 'legacy'\nhost = 'localhost'\nport = 22\nprotocol = 'SSH'\n[auth_method.Password]\nusername = 'user'\npassword = 'migration-secret'\n");
        std::fs::write(&path, &bytes).unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        *credentials.fail.lock().unwrap() = true;
        let repo = Arc::new(SessionRepository::new(
            tmp.path().into(),
            credentials.clone(),
        ));
        let svc = Arc::new(make_service_with_repo(repo));
        let dispatcher = make_dispatcher(svc.clone(), tmp.path());
        assert!(svc.list_sessions().await.unwrap().is_empty());
        assert!(svc.connect(id).await.is_err());
        let issues = svc.list_load_issues().await;
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].session_id, Some(id));
        assert!(!serde_json::to_string(&issues)
            .unwrap()
            .contains("migration-secret"));
        let outcome = dispatcher
            .dispatch(AppCommand::ListSessionLoadIssues)
            .await
            .unwrap();
        let CommandOutcome::SessionLoadIssues(stored) = outcome else {
            panic!("unexpected load issues outcome");
        };
        assert_eq!(stored, issues);
        // The Tauri command serializes the extracted payload, not the internal
        // tagged dispatcher enum (whose list variants are not JSON payloads).
        assert!(!serde_json::to_string(&stored)
            .unwrap()
            .contains("migration-secret"));
        assert_eq!(svc.list_load_issues().await, issues);
        dispatcher
            .dispatch(AppCommand::RetrySessionLoad)
            .await
            .unwrap();
        assert_eq!(svc.list_load_issues().await, issues);
        let mut replacement = make_config("replacement", "localhost");
        replacement.id = id;
        assert!(svc.create_session(replacement).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        *credentials.fail.lock().unwrap() = false;
        dispatcher
            .dispatch(AppCommand::RetrySessionLoad)
            .await
            .unwrap();
        assert!(svc.list_load_issues().await.is_empty());
        assert_eq!(svc.list_sessions().await.unwrap()[0].id, id);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("migration-secret"));
        // A repeat retry must not reset a live session or its connection attempt.
        svc.sessions
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .connection_state = ConnectionState::Connecting;
        dispatcher
            .dispatch(AppCommand::RetrySessionLoad)
            .await
            .unwrap();
        assert_eq!(
            svc.get_state(id).await.unwrap(),
            ConnectionState::Connecting
        );
    }

    #[tokio::test]
    async fn connecting_snapshot_cannot_resolve_replacement_hosts_password() {
        use crate::session::repository::tests::{set, MemoryCredentials};
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(tmp.path().into(), credentials));
        let svc = Arc::new(make_service_with_repo(repo.clone()));
        let original = make_config("original", "host-a.test");
        let id = svc
            .create_session_with_credential(
                original.clone(),
                Some(SessionCredential {
                    secret: "password-a".into(),
                }),
            )
            .await
            .unwrap();

        // Exercise the same snapshot and auth boundaries as connect(), but
        // deterministically pause in between while the update runs to completion.
        let (captured_tx, captured_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();
        let connecting = {
            let svc = svc.clone();
            tokio::spawn(async move {
                let (cancel_tx, _cancel_rx) = oneshot::channel();
                let snapshot = svc
                    .claim_connection(id, Uuid::new_v4(), cancel_tx)
                    .await
                    .unwrap();
                captured_tx.send(()).unwrap();
                resume_rx.await.unwrap();
                let ResolvedAuthMethod::Password { password, .. } =
                    svc.resolve_auth(&snapshot).unwrap()
                else {
                    panic!("expected password auth");
                };
                (snapshot.host, password)
            })
        };
        captured_rx.await.unwrap();
        let mut replacement = original;
        replacement.host = "host-b.test".into();
        let update = svc
            .update_session_with_credential(id, replacement.clone(), set("password-b"))
            .await;
        resume_tx.send(()).unwrap();
        let (host, password) = connecting.await.unwrap();
        assert_eq!(host, "host-a.test");
        assert!(
            password == "password-a",
            "original host must only receive its original password"
        );
        assert!(matches!(update, Err(CoreError::InvalidState(_))));
        let stored = repo.load(id).unwrap().unwrap();
        assert_eq!(stored.host, "host-a.test");
        assert!(repo.credential(&stored).unwrap().as_deref() == Some("password-a"));

        // Cancellation releases the update restriction and preserves reconnect.
        svc.disconnect(id).await.unwrap();
        svc.update_session_with_credential(id, replacement, set("password-b"))
            .await
            .unwrap();
        let (cancel_tx, _cancel_rx) = oneshot::channel();
        let next = svc
            .claim_connection(id, Uuid::new_v4(), cancel_tx)
            .await
            .unwrap();
        assert_eq!(next.host, "host-b.test");
        assert!(
            matches!(svc.resolve_auth(&next), Ok(ResolvedAuthMethod::Password { password, .. }) if password == "password-b")
        );
    }

    #[tokio::test]
    async fn missing_credential_fails_before_network_and_never_publishes_connected() {
        use crate::session::repository::tests::MemoryCredentials;
        use rshell_api::types::SessionCredential;
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            tmp.path().into(),
            credentials.clone(),
        ));
        let svc = make_service_with_repo(repo);
        let id = svc
            .create_session_with_credential(
                make_config("missing", "example.test"),
                Some(SessionCredential {
                    secret: "temporary".into(),
                }),
            )
            .await
            .unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected = events.clone();
        svc.event_bus
            .subscribe(move |event| collected.lock().unwrap().push(event.clone()));
        credentials.entries.lock().unwrap().clear();
        assert!(matches!(
            svc.connect(id).await,
            Err(CoreError::InvalidState(_))
        ));
        assert_eq!(
            svc.get_state(id).await.unwrap(),
            ConnectionState::Disconnected
        );
        for event in events.lock().unwrap().iter() {
            assert!(!matches!(
                event,
                AppEvent::ConnectionStateChanged {
                    state: ConnectionState::Connected,
                    ..
                }
            ));
        }
    }

    #[tokio::test]
    async fn failed_metadata_write_keeps_memory_and_credentials_unchanged() {
        use crate::session::repository::tests::{set, MemoryCredentials};
        use rshell_api::types::SessionCredential;
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            tmp.path().into(),
            credentials.clone(),
        ));
        let svc = make_service_with_repo(repo);
        let cfg = make_config("original", "example.test");
        let id = svc
            .create_session_with_credential(
                cfg.clone(),
                Some(SessionCredential {
                    secret: "original-secret".into(),
                }),
            )
            .await
            .unwrap();
        let file = tmp.path().join(format!("{id}.toml"));
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let mut changed = cfg.clone();
        changed.name = "changed".into();
        assert!(svc
            .update_session_with_credential(id, changed, set("new-secret"))
            .await
            .is_err());
        assert_eq!(svc.list_sessions().await.unwrap()[0].name, "original");
        assert!(
            matches!(svc.resolve_auth(&cfg), Ok(ResolvedAuthMethod::Password { password, .. }) if password == "original-secret")
        );
        let create = make_config("failed-create", "example.test");
        std::fs::create_dir(tmp.path().join(format!("{}.toml", create.id))).unwrap();
        assert!(svc
            .create_session_with_credential(
                create,
                Some(SessionCredential {
                    secret: "new-secret".into()
                })
            )
            .await
            .is_err());
        assert_eq!(svc.list_sessions().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn credential_cleanup_failure_still_removes_deleted_metadata_from_memory() {
        use crate::session::repository::tests::MemoryCredentials;
        use rshell_api::types::SessionCredential;
        let tmp = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            tmp.path().into(),
            credentials.clone(),
        ));
        let svc = make_service_with_repo(repo.clone());
        let id = svc
            .create_session_with_credential(
                make_config("delete", "example.test"),
                Some(SessionCredential {
                    secret: "delete-secret".into(),
                }),
            )
            .await
            .unwrap();
        *credentials.fail.lock().unwrap() = true;
        assert!(svc.delete_session(id).await.is_err());
        assert!(repo.load(id).unwrap().is_none());
        assert!(svc.list_sessions().await.unwrap().is_empty());
        *credentials.fail.lock().unwrap() = false;
        svc.delete_session(id).await.unwrap();
        assert!(credentials.entries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatcher_routes_create_and_update_credentials_without_echoing_secrets() {
        use crate::session::repository::tests::{set, MemoryCredentials};
        use rshell_api::{AppCommand, CommandOutcome};
        let dir = tempfile::tempdir().unwrap();
        let credentials = Arc::new(MemoryCredentials::default());
        let repo = Arc::new(SessionRepository::new(
            dir.path().join("sessions"),
            credentials,
        ));
        let svc = Arc::new(make_service_with_repo(repo));
        let dispatcher = make_dispatcher(svc.clone(), dir.path());
        let config = make_config("dispatch", "example.test");
        let id = config.id;
        dispatcher
            .dispatch(AppCommand::CreateSession {
                config: config.clone(),
                credential: Some(SessionCredential {
                    secret: "dispatcher-secret".into(),
                }),
            })
            .await
            .unwrap();
        assert!(
            matches!(svc.resolve_auth(&config), Ok(ResolvedAuthMethod::Password { password, .. }) if password == "dispatcher-secret")
        );
        dispatcher
            .dispatch(AppCommand::UpdateSession {
                id,
                config: config.clone(),
                credential: set("replacement-secret"),
            })
            .await
            .unwrap();
        assert!(
            matches!(svc.resolve_auth(&config), Ok(ResolvedAuthMethod::Password { password, .. }) if password == "replacement-secret")
        );
        let result = dispatcher.dispatch(AppCommand::ListSessions).await.unwrap();
        let CommandOutcome::Sessions(sessions) = result else {
            panic!("expected sessions");
        };
        // Tauri unwraps the outcome and serializes this payload.
        let json = serde_json::to_string(&sessions).unwrap();
        assert!(!json.contains("dispatcher-secret"));
        assert!(!json.contains("replacement-secret"));
    }

    #[tokio::test]
    async fn test_session_persistence_roundtrip() {
        // 用 tempfile 给一个隔离目录,模拟"进程重启"。
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_path_buf();

        // 第一个生命周期:create ×2 → 落盘。
        let credentials = Arc::new(crate::session::repository::tests::MemoryCredentials::default());
        let repo_a = Arc::new(SessionRepository::new(path.clone(), credentials.clone()));
        let svc_a = make_service_with_repo(repo_a.clone());
        let cfg_a = make_config("alpha", "host-a");
        let id_a = svc_a.create_session(cfg_a).await.unwrap();
        let cfg_b = make_config("beta", "host-b");
        let id_b = svc_a.create_session(cfg_b).await.unwrap();
        drop(svc_a); // 显式 drop,模拟进程退出

        // 第二个生命周期:重新构造 → load_from_disk → 应能列回两条。
        let repo_b = Arc::new(SessionRepository::new(path, credentials));
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
        let repo = Arc::new(SessionRepository::new(
            path.clone(),
            Arc::new(crate::session::repository::tests::MemoryCredentials::default()),
        ));
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
