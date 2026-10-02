//! SSH 隧道管理器
//!
//! 管理本地和动态端口转发隧道。
//!
//! 持久化:隧道规则写入 `data_local_dir/rshell/tunnels.toml`,
//! 启动时自动恢复。运行时只持久化**规则**,不恢复 listener（避免与
//! 上次进程残留端口冲突；恢复在重启时再次 create_tunnel）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

use rshell_api::events::AppEvent;
use rshell_api::types::{
    ActiveTunnelInfo, ConnectionState, ForwardDirection, PendingTunnelInfo, PortForwardRule,
    TunnelState, UnsupportedTunnelRule,
};

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SshClientHandle;

/// 会话断开时写入隧道状态的原因文案
const SESSION_DISCONNECTED_REASON: &str = "Session disconnected";

/// 判断 bind_address 是否为本机回环地址（PROB-23）。
///
/// 接受 `localhost`（大小写不敏感）、`127.0.0.0/8` 内任意 IPv4 与 `::1`
/// （含完整展开写法与 `[::1]` 方括号形式）。无法解析为已知回环地址的
/// 主机名一律按非回环处理：宁可多要一次确认，不静默对外监听。
fn is_loopback_bind_address(addr: &str) -> bool {
    let trimmed = addr.trim();
    let bare = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// 校验隧道 bind 地址（PROB-23）：非回环地址必须携带用户显式确认标志。
///
/// `create_tunnel` 与 `resume_tunnel` 共用；未确认的非回环规则一律拒绝，
/// 避免把端口转发 / 无认证 SOCKS5 代理静默暴露给局域网。回环地址行为不变。
fn validate_bind_address(rule: &PortForwardRule) -> Result<(), CoreError> {
    if rule.allow_non_loopback || is_loopback_bind_address(&rule.bind_address) {
        return Ok(());
    }
    Err(CoreError::InvalidState(format!(
        "bind_address '{}' is not a loopback address: listening would expose \
         the port forward / unauthenticated SOCKS5 proxy to the LAN. \
         Confirm the exposure in the UI and retry (allow_non_loopback)",
        rule.bind_address
    )))
}

/// 活动隧道
pub struct ActiveTunnel {
    pub id: Uuid,
    pub session_id: Uuid,
    pub rule: PortForwardRule,
    pub state: TunnelState,
    pub bytes_transferred: u64,
    pub connections_count: u32,
    /// 创建时刻：会话断开的清理据此只停用断开前已存在的隧道，
    /// 不会误停用户快速重连后基于新连接创建的隧道。
    pub created_at: Instant,
    /// 创建隧道时的 SSH 连接句柄：suspend 会 abort 监听任务，resume
    /// 重新 bind 时需要用它重建转发（PROB-21）。
    ssh_client: SshClientHandle,
    /// 监听任务的句柄
    listener_handle: Option<tokio::task::JoinHandle<()>>,
}

/// 手动实现 Debug：`SshClientHandle` 未实现 Debug，也不应把连接内部
/// 状态写进日志。
impl std::fmt::Debug for ActiveTunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveTunnel")
            .field("id", &self.id)
            .field("session_id", &self.session_id)
            .field("rule", &self.rule)
            .field("state", &self.state)
            .field("bytes_transferred", &self.bytes_transferred)
            .field("connections_count", &self.connections_count)
            .field("created_at", &self.created_at)
            .field("listening", &self.listener_handle.is_some())
            .finish()
    }
}

/// SSH 隧道管理器
pub struct TunnelManager {
    tunnels: Arc<RwLock<HashMap<Uuid, ActiveTunnel>>>,
    event_bus: Arc<EventBus>,
    /// 持久化文件路径 (None 表示不持久化)
    persist_path: Option<PathBuf>,
}

/// 磁盘上的隧道注册表
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PersistedTunnels {
    /// 按 session_id 分组,每个 session 下挂若干条规则
    #[serde(default)]
    rules: HashMap<Uuid, Vec<PersistedRule>>,
}

/// 仅用于读取旧配置；Remote 不会进入公开 API 或运行中的隧道。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedRule {
    bind_address: String,
    bind_port: u16,
    remote_host: String,
    remote_port: u16,
    direction: PersistedDirection,
    /// 非回环 bind 的用户确认标志（PROB-23）；旧文件缺省视为未确认。
    #[serde(default)]
    allow_non_loopback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum PersistedDirection {
    Local,
    Remote,
    Dynamic,
}

impl PersistedRule {
    fn from_supported(rule: PortForwardRule) -> Self {
        let direction = match rule.direction {
            ForwardDirection::Local => PersistedDirection::Local,
            ForwardDirection::Dynamic => PersistedDirection::Dynamic,
        };
        Self {
            bind_address: rule.bind_address,
            bind_port: rule.bind_port,
            remote_host: rule.remote_host,
            remote_port: rule.remote_port,
            direction,
            allow_non_loopback: rule.allow_non_loopback,
        }
    }

    fn into_supported(self) -> Option<PortForwardRule> {
        let direction = match self.direction {
            PersistedDirection::Local => ForwardDirection::Local,
            PersistedDirection::Dynamic => ForwardDirection::Dynamic,
            PersistedDirection::Remote => return None,
        };
        Some(PortForwardRule {
            bind_address: self.bind_address,
            bind_port: self.bind_port,
            remote_host: self.remote_host,
            remote_port: self.remote_port,
            direction,
            allow_non_loopback: self.allow_non_loopback,
        })
    }
}

impl TunnelManager {
    /// 创建新的隧道管理器
    ///
    /// 会同时订阅 EventBus：会话断开（用户断开、远端 EOF、删除会话等路径）
    /// 都会广播 `ConnectionStateChanged { Disconnected }`，收到后按 session_id
    /// 停用该会话名下的隧道（标记 Error 并停止 listener），避免隧道挂着已
    /// 失效的 SSH 连接继续接受连接并逐一失败（PROB-08）。
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        let tunnels: Arc<RwLock<HashMap<Uuid, ActiveTunnel>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let watcher_tunnels = tunnels.clone();
        let watcher_bus = event_bus.clone();
        event_bus.subscribe(move |event| {
            let AppEvent::ConnectionStateChanged {
                session_id,
                state: ConnectionState::Disconnected,
                ..
            } = event
            else {
                return;
            };
            let session_id = *session_id;
            // 时间戳在回调内同步取得：此后（重连成功后）创建的隧道必然晚于
            // 该时刻，不会被这次迟到的清理误停。
            let deactivated_at = Instant::now();
            let tunnels = watcher_tunnels.clone();
            let event_bus = watcher_bus.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    deactivate_session_tunnels(&tunnels, &event_bus, session_id, deactivated_at)
                        .await;
                });
            } else {
                warn!(
                    session_id = %session_id,
                    "No tokio runtime available to deactivate tunnels on session disconnect"
                );
            }
        });
        Self {
            tunnels,
            event_bus,
            persist_path: None,
        }
    }

    /// 启用持久化:启动时从 `path` 读,create_tunnel / close_tunnel 自动 dump
    ///
    /// 不会**自动**调用 `create_tunnel` 恢复;调用方应在适当时机调
    /// `restore_pending_rules` 拿到 `Vec<(Uuid, PortForwardRule)>` 后
    /// 显式 recreate (本环境的设计是: 重启不抢占端口, 仅记录用户意图)。
    pub fn with_persistence(mut self, path: PathBuf) -> Self {
        self.persist_path = Some(path);
        self
    }

    /// 从磁盘读所有 (session_id, rule) 对,供调用方决定是否重建
    pub async fn restore_pending_rules(&self) -> Vec<(Uuid, PortForwardRule)> {
        self.restore_pending_rules_info().await.rules
    }

    /// 同时返回可恢复规则及被安全跳过的旧规则诊断。
    pub async fn restore_pending_rules_info(&self) -> PendingTunnelInfo {
        let Some(path) = self.persist_path.as_ref() else {
            return PendingTunnelInfo {
                rules: Vec::new(),
                unsupported: Vec::new(),
            };
        };
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<PersistedTunnels>(&content) {
                Ok(p) => {
                    let mut info = PendingTunnelInfo {
                        rules: Vec::new(),
                        unsupported: Vec::new(),
                    };
                    for (sid, rules) in p.rules {
                        for rule in rules {
                            if rule.direction == PersistedDirection::Remote {
                                warn!(session_id = %sid, "Skipping unsupported legacy Remote forwarding rule");
                                info.unsupported.push(UnsupportedTunnelRule {
                                    session_id: sid,
                                    reason: "Remote forwarding is not supported".into(),
                                });
                            } else if let Some(rule) = rule.into_supported() {
                                info.rules.push((sid, rule));
                            }
                        }
                    }
                    info
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    PendingTunnelInfo {
                        rules: Vec::new(),
                        unsupported: Vec::new(),
                    }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PendingTunnelInfo {
                rules: Vec::new(),
                unsupported: Vec::new(),
            },
            Err(e) => {
                warn!("Failed to read {}: {}", path.display(), e);
                PendingTunnelInfo {
                    rules: Vec::new(),
                    unsupported: Vec::new(),
                }
            }
        }
    }

    /// 把当前 `tunnels` 状态 dump 到磁盘
    async fn save_to_disk(&self) {
        let Some(path) = self.persist_path.as_ref() else {
            return;
        };
        let tunnels = self.tunnels.read().await;
        // 把 ActiveTunnel 简化成 PersistedTunnels (只存规则)
        let mut grouped: HashMap<Uuid, Vec<PersistedRule>> = HashMap::new();
        // 旧 Remote 规则保持在用户文件中，但永远不恢复或启动。
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<PersistedTunnels>(&content) {
                Ok(previous) => {
                    for (sid, rules) in previous.rules {
                        grouped.entry(sid).or_default().extend(
                            rules
                                .into_iter()
                                .filter(|rule| rule.direction == PersistedDirection::Remote),
                        );
                    }
                }
                Err(error) => {
                    warn!(%error, "Skipping tunnel save to preserve unreadable existing config");
                    return;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(%error, "Skipping tunnel save to preserve unreadable existing config");
                return;
            }
        }
        for t in tunnels.values() {
            grouped
                .entry(t.session_id)
                .or_default()
                .push(PersistedRule::from_supported(t.rule.clone()));
        }
        let persisted = PersistedTunnels { rules: grouped };
        match toml::to_string_pretty(&persisted) {
            Ok(s) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(path, s) {
                    warn!("Failed to write {}: {}", path.display(), e);
                }
            }
            Err(e) => warn!("Failed to serialize tunnels: {}", e),
        }
    }

    /// 创建端口转发隧道
    ///
    /// `ssh_client`: 与该隧道关联的 SSH 连接句柄；缺失时拒绝创建。
    ///
    /// LocalForward: 监听 `bind_address:bind_port`，每条接入连接通过 SSH direct-tcpip
    /// 通道转发到 `remote_host:remote_port`。
    /// DynamicForward(SOCKS): 解析 CONNECT 请求头后经 SSH 通道转发。
    pub async fn create_tunnel(
        &self,
        session_id: Uuid,
        rule: PortForwardRule,
        ssh_client: Option<SshClientHandle>,
    ) -> Result<Uuid, CoreError> {
        let ssh_client = ssh_client.ok_or_else(|| {
            CoreError::ConnectionError("SSH connection required for port forwarding".into())
        })?;
        // PROB-23：默认拒绝非回环 bind，除非规则携带 UI 显式确认标志。
        validate_bind_address(&rule)?;
        let tunnel_id = Uuid::new_v4();
        info!("Creating tunnel: id={}, rule={:?}", tunnel_id, rule);

        let bind_addr = format!("{}:{}", rule.bind_address, rule.bind_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to bind {}: {}", bind_addr, e)))?;

        let local_addr = listener
            .local_addr()
            .map_err(|e| CoreError::Internal(format!("Failed to get local addr: {}", e)))?;

        info!("Tunnel listening on: {}", local_addr);
        if !is_loopback_bind_address(&rule.bind_address) {
            warn!(
                tunnel_id = %tunnel_id,
                bind = %local_addr,
                "Tunnel listens on a non-loopback address: the port forward / \
                 unauthenticated SOCKS5 proxy is exposed to the network (user-confirmed)"
            );
        }

        // 启动监听任务（与 resume_tunnel 共用同一 accept 循环）。
        // ssh_client 实际参与转发：每条接入连接均通过 SSH direct-tcpip
        // 通道转发；同时保留一份在 ActiveTunnel 上供 resume 重建（PROB-21）。
        let handle = spawn_accept_loop(
            self.tunnels.clone(),
            tunnel_id,
            rule.clone(),
            ssh_client.clone(),
            listener,
        );

        let tunnel = ActiveTunnel {
            id: tunnel_id,
            session_id,
            rule,
            state: TunnelState::Active,
            bytes_transferred: 0,
            connections_count: 0,
            created_at: Instant::now(),
            ssh_client,
            listener_handle: Some(handle),
        };

        self.tunnels.write().await.insert(tunnel_id, tunnel);

        self.event_bus.publish(AppEvent::TunnelStateChanged {
            tunnel_id,
            state: TunnelState::Active,
        });
        self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

        info!("Tunnel created: id={}", tunnel_id);
        self.save_to_disk().await;
        Ok(tunnel_id)
    }

    /// 关闭隧道
    pub async fn close_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        info!("Closing tunnel: {}", tunnel_id);

        // 在 write guard 内做变更 + 取消 listener,之后立即释放 guard
        // (save_to_disk 内部会再 .read().await 拿 read guard)
        let removed = {
            let mut tunnels = self.tunnels.write().await;
            if let Some(mut tunnel) = tunnels.remove(&tunnel_id) {
                if let Some(handle) = tunnel.listener_handle.take() {
                    handle.abort();
                }
                true
            } else {
                false
            }
        };

        if removed {
            self.event_bus.publish(AppEvent::TunnelStateChanged {
                tunnel_id,
                state: TunnelState::Error("Closed".into()),
            });
            self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

            self.save_to_disk().await;
            info!("Tunnel closed: {}", tunnel_id);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!(
                "Tunnel not found: {}",
                tunnel_id
            )))
        }
    }

    /// 会话断开后按 session_id 停用对应隧道：标记 `TunnelState::Error` 并
    /// abort listener（停止接受新连接）。
    ///
    /// `new` 中的 EventBus 订阅会在收到 `ConnectionStateChanged { Disconnected }`
    /// 时自动触发；公开此方法供测试与显式接线复用。只处理 `deactivated_at`
    /// 之前创建的隧道，重连后基于新连接创建的隧道不受影响。
    pub async fn handle_session_disconnected(&self, session_id: Uuid) {
        deactivate_session_tunnels(&self.tunnels, &self.event_bus, session_id, Instant::now())
            .await;
    }

    /// 挂起隧道（PROB-21）：abort 监听任务、关闭监听端口，此后新连接被
    /// 拒绝；规则与 SSH 句柄保留，`resume_tunnel` 可重新 bind 恢复。
    ///
    /// 仅对 Active 状态的隧道生效：Error（会话已断开）不可挂起，
    /// 重复挂起也返回错误，避免静默假成功。
    pub async fn suspend_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        {
            let mut tunnels = self.tunnels.write().await;
            let tunnel = tunnels
                .get_mut(&tunnel_id)
                .ok_or_else(|| CoreError::NotFound(format!("Tunnel not found: {}", tunnel_id)))?;
            if !matches!(tunnel.state, TunnelState::Active) {
                return Err(CoreError::InvalidState(format!(
                    "Tunnel {} is not active (state: {:?})",
                    tunnel_id, tunnel.state
                )));
            }
            // abort 监听任务即 drop 监听 socket：新连接从此被拒绝。
            if let Some(handle) = tunnel.listener_handle.take() {
                handle.abort();
            }
            tunnel.state = TunnelState::Suspended;
        }

        self.event_bus.publish(AppEvent::TunnelStateChanged {
            tunnel_id,
            state: TunnelState::Suspended,
        });
        self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

        info!("Tunnel suspended: {}", tunnel_id);
        Ok(())
    }

    /// 恢复挂起的隧道（PROB-21）：重新 bind 规则地址并重启转发 listener。
    ///
    /// 端口被占用时 bind 失败并返回错误，隧道保持挂起，可在端口释放后
    /// 再次 resume。仅对 Suspended 状态的隧道生效。
    pub async fn resume_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        // 先在 read 锁下取规则与 SSH 句柄快照，再释放锁执行 bind：
        // bind 可能涉及域名解析，不应在持有 write guard 期间等待。
        let (rule, ssh_client) = {
            let tunnels = self.tunnels.read().await;
            let tunnel = tunnels
                .get(&tunnel_id)
                .ok_or_else(|| CoreError::NotFound(format!("Tunnel not found: {}", tunnel_id)))?;
            if !matches!(tunnel.state, TunnelState::Suspended) {
                return Err(CoreError::InvalidState(format!(
                    "Tunnel {} is not suspended (state: {:?})",
                    tunnel_id, tunnel.state
                )));
            }
            (tunnel.rule.clone(), tunnel.ssh_client.clone())
        };

        let bind_addr = format!("{}:{}", rule.bind_address, rule.bind_port);
        // PROB-23：resume 重新 bind 前同样校验；未确认的非回环规则拒绝恢复，
        // 隧道保持 Suspended，不会静默重新暴露给局域网。
        validate_bind_address(&rule)?;
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to bind {}: {}", bind_addr, e)))?;

        let handle = spawn_accept_loop(self.tunnels.clone(), tunnel_id, rule, ssh_client, listener);

        // 快照与 bind 之间隧道可能已被关闭或因会话断开停用：复核不通过
        // 则放弃刚启动的 listener，保持原状态不复活。（move 与 abort 必须
        // 处于同一 match 的不同分支，跨条件结构使用会触发 E0382。）
        {
            let mut tunnels = self.tunnels.write().await;
            match tunnels.get_mut(&tunnel_id) {
                Some(tunnel) if matches!(tunnel.state, TunnelState::Suspended) => {
                    tunnel.listener_handle = Some(handle);
                    tunnel.state = TunnelState::Active;
                }
                _ => {
                    handle.abort();
                    return Err(CoreError::InvalidState(format!(
                        "Tunnel {} is no longer suspended",
                        tunnel_id
                    )));
                }
            }
        }

        self.event_bus.publish(AppEvent::TunnelStateChanged {
            tunnel_id,
            state: TunnelState::Active,
        });
        self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

        info!("Tunnel resumed: {}", tunnel_id);
        Ok(())
    }

    /// 列出所有活动隧道
    pub async fn list_tunnels(&self) -> Vec<ActiveTunnelInfo> {
        let tunnels = self.tunnels.read().await;
        tunnels
            .values()
            .map(|t| ActiveTunnelInfo {
                id: t.id,
                session_id: t.session_id,
                rule: t.rule.clone(),
                state: t.state.clone(),
                bytes_transferred: t.bytes_transferred,
                connections_count: t.connections_count,
            })
            .collect()
    }

    /// 获取隧道信息
    pub async fn get_tunnel(&self, tunnel_id: Uuid) -> Option<ActiveTunnelInfo> {
        let tunnels = self.tunnels.read().await;
        tunnels.get(&tunnel_id).map(|t| ActiveTunnelInfo {
            id: t.id,
            session_id: t.session_id,
            rule: t.rule.clone(),
            state: t.state.clone(),
            bytes_transferred: t.bytes_transferred,
            connections_count: t.connections_count,
        })
    }
}

/// 启动隧道 accept 循环：接受接入连接并按规则经 SSH 转发。
///
/// `create_tunnel` 与 `resume_tunnel`（PROB-21）共用；任务被 abort 或
/// drop 时监听 socket 一并关闭，新连接随即被拒绝。
fn spawn_accept_loop(
    tunnels: Arc<RwLock<HashMap<Uuid, ActiveTunnel>>>,
    tunnel_id: Uuid,
    rule: PortForwardRule,
    ssh_client: SshClientHandle,
    listener: TcpListener,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((inbound, peer_addr)) => {
                    info!("Tunnel {}: new connection from {}", tunnel_id, peer_addr);

                    // 更新连接计数
                    {
                        let mut tunnels = tunnels.write().await;
                        if let Some(t) = tunnels.get_mut(&tunnel_id) {
                            t.connections_count += 1;
                        }
                    }

                    let remote_host = rule.remote_host.clone();
                    let remote_port = rule.remote_port;
                    let direction = rule.direction;
                    let tunnels_clone = tunnels.clone();
                    let tid = tunnel_id;

                    // 为每条连接 spawn 一个转发任务
                    let ssh_client_for_task = ssh_client.clone();
                    tokio::spawn(async move {
                        let res = match direction {
                            ForwardDirection::Local => {
                                forward_local(
                                    ssh_client_for_task,
                                    inbound,
                                    &remote_host,
                                    remote_port,
                                    tid,
                                )
                                .await
                            }
                            ForwardDirection::Dynamic => {
                                // SOCKS5 DynamicForward: 解析客户端握手得到目标 host:port,
                                // 再用 SSH direct-tcpip 转发(无 SSH client 时走 plain TCP)。
                                forward_dynamic_socks5(ssh_client_for_task, inbound, tid).await
                            }
                        };
                        if let Err(msg) = res {
                            warn!("Tunnel {}: {}", tid, msg);
                        }

                        // 连接关闭后更新计数
                        let mut tunnels = tunnels_clone.write().await;
                        if let Some(t) = tunnels.get_mut(&tid) {
                            t.connections_count = t.connections_count.saturating_sub(1);
                        }
                    });
                }
                Err(e) => {
                    error!("Tunnel {} accept error: {}", tunnel_id, e);
                    break;
                }
            }
        }
    })
}

/// 会话断开后的隧道清理：把 `session_id` 名下、在 `deactivated_at` 之前创建
/// 且尚未失效的隧道标记为 `TunnelState::Error`，并 abort 其 listener
/// （停止接受新连接）。
///
/// 以创建时间过滤是为了不误停"断开后快速重连"所创建的新隧道；
/// 已处于 Error 状态的隧道跳过，因此重复的断开广播是幂等的。
async fn deactivate_session_tunnels(
    tunnels: &RwLock<HashMap<Uuid, ActiveTunnel>>,
    event_bus: &EventBus,
    session_id: Uuid,
    deactivated_at: Instant,
) {
    let mut deactivated = Vec::new();
    {
        let mut map = tunnels.write().await;
        for (tunnel_id, tunnel) in map.iter_mut() {
            if tunnel.session_id != session_id
                || tunnel.created_at > deactivated_at
                || matches!(tunnel.state, TunnelState::Error(_))
            {
                continue;
            }
            if let Some(handle) = tunnel.listener_handle.take() {
                handle.abort();
            }
            tunnel.state = TunnelState::Error(SESSION_DISCONNECTED_REASON.into());
            deactivated.push(*tunnel_id);
        }
    }
    if deactivated.is_empty() {
        return;
    }
    info!(
        session_id = %session_id,
        count = deactivated.len(),
        "Tunnels deactivated after session disconnect"
    );
    for tunnel_id in deactivated {
        event_bus.publish(AppEvent::TunnelStateChanged {
            tunnel_id,
            state: TunnelState::Error(SESSION_DISCONNECTED_REASON.into()),
        });
    }
    event_bus.publish(AppEvent::ActiveTunnelsChanged);
}

/// 内部 helper: LocalForward — inbound → remote_host:remote_port
///
/// 始终通过 SSH direct-tcpip 转发。
async fn forward_local(
    ssh_client: SshClientHandle,
    mut inbound: TcpStream,
    remote_host: &str,
    remote_port: u16,
    tid: Uuid,
) -> Result<(), String> {
    {
        let ssh = ssh_client;
        let channel = {
            let client = ssh.read().await;
            client
                .open_direct_tcpip(remote_host, remote_port as u32)
                .await
        };
        match channel {
            Ok(mut ch) => {
                let (mut ri, mut wi) = inbound.split();
                let mut wo = ch.make_writer();
                let mut ro = ch.make_reader();
                let c2s = tokio::io::copy(&mut ri, &mut wo);
                let s2c = tokio::io::copy(&mut ro, &mut wi);
                let (c2s_res, s2c_res) = tokio::join!(c2s, s2c);
                drop(ro);
                drop(wo);
                let _ = ch.eof().await;
                if let Err(e) = c2s_res {
                    warn!("Tunnel {}: client→remote (ssh) copy error: {}", tid, e);
                }
                if let Err(e) = s2c_res {
                    warn!("Tunnel {}: remote→client (ssh) copy error: {}", tid, e);
                }
                Ok(())
            }
            Err(e) => Err(format!(
                "ssh direct-tcpip {}:{} failed: {}",
                remote_host, remote_port, e
            )),
        }
    }
}

/// 内部 helper: DynamicForward (SOCKS5) — 解析客户端握手得到目标,
/// 然后跟 LocalForward 一样转发到 host:port (从握手解析).
async fn forward_dynamic_socks5(
    ssh_client: SshClientHandle,
    mut inbound: TcpStream,
    tid: Uuid,
) -> Result<(), String> {
    let (host, port) = socks5_handshake(&mut inbound).await?;
    info!("Tunnel {}: SOCKS5 CONNECT to {}:{}", tid, host, port);

    forward_local(ssh_client, inbound, &host, port, tid).await
}

/// SOCKS5 握手: 读 greeting + request, 写回 reply.
async fn socks5_handshake(inbound: &mut TcpStream) -> Result<(String, u16), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // 1. 读 greeting: VER NMETHODS METHODS, 回 VER METHOD (0x00 = no auth)
    let mut buf = [0u8; 512];
    let n = inbound
        .read(&mut buf)
        .await
        .map_err(|e| format!("socks5 read greeting: {}", e))?;
    if n < 2 || buf[0] != 0x05 {
        return Err(format!("socks5 bad greeting: n={} ver={}", n, buf[0]));
    }
    let nmethods = buf[1] as usize;
    if n < 2 + nmethods {
        return Err(format!(
            "socks5 greeting truncated: n={} nmethods={}",
            n, nmethods
        ));
    }
    // 强制 no-auth (即便客户端没列, RFC 允许 server 选)
    inbound
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| format!("socks5 write method: {}", e))?;

    // 2. 读请求: VER CMD RSV ATYP DST.ADDR DST.PORT
    let n = inbound
        .read(&mut buf)
        .await
        .map_err(|e| format!("socks5 read request: {}", e))?;
    if n < 7 {
        return Err(format!("socks5 request too short: {}", n));
    }
    if buf[0] != 0x05 {
        return Err(format!("socks5 request bad ver: {}", buf[0]));
    }
    let cmd = buf[1];
    if cmd != 0x01 {
        // 仅支持 CONNECT
        let _ = inbound
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;
        return Err(format!("socks5 unsupported cmd: {}", cmd));
    }
    let atyp = buf[3];
    let (host, port) = match atyp {
        0x01 => {
            if n < 4 + 4 + 2 {
                return Err("socks5 ipv4 truncated".to_string());
            }
            let ip = std::net::Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
            let port = u16::from_be_bytes([buf[8], buf[9]]);
            (ip.to_string(), port)
        }
        0x03 => {
            let dlen = buf[4] as usize;
            if n < 5 + dlen + 2 {
                return Err("socks5 domain truncated".to_string());
            }
            let domain = std::str::from_utf8(&buf[5..5 + dlen])
                .map_err(|e| format!("socks5 domain utf-8: {}", e))?
                .to_string();
            let port = u16::from_be_bytes([buf[5 + dlen], buf[6 + dlen]]);
            (domain, port)
        }
        0x04 => {
            if n < 4 + 16 + 2 {
                return Err("socks5 ipv6 truncated".to_string());
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[4..20]);
            let ip = std::net::Ipv6Addr::from(octets);
            let port = u16::from_be_bytes([buf[20], buf[21]]);
            (ip.to_string(), port)
        }
        _ => {
            let _ = inbound
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return Err(format!("socks5 unsupported atyp: {}", atyp));
        }
    };

    // 3. 写 REP=0x00 成功响应 (BND 用 0.0.0.0:0 占位)
    inbound
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|e| format!("socks5 write reply: {}", e))?;

    Ok((host, port))
}

impl Drop for ActiveTunnel {
    fn drop(&mut self) {
        if let Some(handle) = self.listener_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::{
        AuthMethod, ForwardDirection, PortForwardRule, Protocol, SessionConfig,
    };
    use rshell_protocol::ssh::{ResolvedAuthMethod, SshClient};

    #[tokio::test]
    async fn legacy_remote_rule_is_skipped_without_changing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tunnels.toml");
        let sid = Uuid::new_v4();
        let content = format!(
            "[rules]\n\"{sid}\" = [\n  {{ bind_address = '127.0.0.1', bind_port = 0, remote_host = 'local', remote_port = 22, direction = 'Local' }},\n  {{ bind_address = '127.0.0.1', bind_port = 0, remote_host = 'legacy', remote_port = 22, direction = 'Remote' }},\n  {{ bind_address = '127.0.0.1', bind_port = 0, remote_host = '', remote_port = 0, direction = 'Dynamic' }}\n]\n"
        );
        std::fs::write(&path, &content).unwrap();
        let mgr = TunnelManager::new(Arc::new(EventBus::new())).with_persistence(path.clone());

        let pending = mgr.restore_pending_rules().await;

        assert_eq!(pending.len(), 2);
        assert!(pending
            .iter()
            .any(|(_, r)| r.direction == ForwardDirection::Local));
        assert!(pending
            .iter()
            .any(|(_, r)| r.direction == ForwardDirection::Dynamic));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

        let info = mgr.restore_pending_rules_info().await;
        assert_eq!(info.unsupported.len(), 1);
        assert_eq!(info.unsupported[0].session_id, sid);
        assert_eq!(
            info.unsupported[0].reason,
            "Remote forwarding is not supported"
        );

        mgr.save_to_disk().await;
        let after_save = std::fs::read_to_string(&path).unwrap();
        assert!(after_save.contains("direction = \"Remote\""));
        assert!(after_save.contains("legacy"));
    }

    #[tokio::test]
    async fn tunnel_needs_an_ssh_connection() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let result = mgr
            .create_tunnel(Uuid::new_v4(), make_rule("example.com", 0), None)
            .await;
        assert!(result.is_err());
    }

    fn make_rule(host: &str, port: u16) -> PortForwardRule {
        PortForwardRule {
            bind_address: "127.0.0.1".to_string(),
            bind_port: port,
            remote_host: host.to_string(),
            remote_port: port,
            direction: ForwardDirection::Local,
            allow_non_loopback: false,
        }
    }

    /// （PROB-23）回环判定：覆盖 127/8、localhost、::1 与方括号 IPv6；
    /// 无法识别的主机名按非回环处理。
    #[test]
    fn loopback_detection_matches_documented_forms() {
        for addr in [
            "127.0.0.1",
            "127.9.9.9",
            "localhost",
            "LOCALHOST",
            "::1",
            "[::1]",
        ] {
            assert!(
                is_loopback_bind_address(addr),
                "'{addr}' must be treated as loopback"
            );
        }
        for addr in ["0.0.0.0", "192.168.1.10", "::", "example.com", ""] {
            assert!(
                !is_loopback_bind_address(addr),
                "'{addr}' must NOT be treated as loopback"
            );
        }
    }

    /// （PROB-23）非回环 bind 默认拒绝；UI 显式确认（allow_non_loopback）
    /// 后放行。回环地址行为不变。
    #[tokio::test]
    async fn non_loopback_bind_requires_explicit_confirmation() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_id = Uuid::new_v4();

        // 未确认：拒绝创建，且不产生任何隧道
        let mut unconfirmed = make_rule("127.0.0.1", 9);
        unconfirmed.bind_address = "0.0.0.0".into();
        unconfirmed.bind_port = 0;
        let err = mgr
            .create_tunnel(session_id, unconfirmed.clone(), Some(make_ssh_handle()))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("not a loopback address"),
            "unexpected error: {err}"
        );
        assert!(mgr.list_tunnels().await.is_empty());

        // 显式确认后放行
        let mut confirmed = unconfirmed;
        confirmed.allow_non_loopback = true;
        let tunnel_id = mgr
            .create_tunnel(session_id, confirmed, Some(make_ssh_handle()))
            .await
            .unwrap();
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Active
        ));

        // 回环地址不需要标志，行为与之前一致
        let loopback = make_rule("127.0.0.1", 0);
        assert!(mgr
            .create_tunnel(session_id, loopback, Some(make_ssh_handle()))
            .await
            .is_ok());
    }

    /// （PROB-23）resume 重新 bind 前同样校验：确认标志缺失的非回环规则
    /// 拒绝恢复并保持 Suspended。
    #[tokio::test]
    async fn resume_rejects_unconfirmed_non_loopback_bind() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_id = Uuid::new_v4();

        let mut rule = make_rule("127.0.0.1", 9);
        rule.bind_address = "0.0.0.0".into();
        rule.bind_port = 0;
        rule.allow_non_loopback = true;
        let tunnel_id = mgr
            .create_tunnel(session_id, rule, Some(make_ssh_handle()))
            .await
            .unwrap();
        mgr.suspend_tunnel(tunnel_id).await.unwrap();

        // 模拟"确认标志丢失"的规则（如旧版本持久化数据）
        mgr.tunnels
            .write()
            .await
            .get_mut(&tunnel_id)
            .unwrap()
            .rule
            .allow_non_loopback = false;

        let err = mgr.resume_tunnel(tunnel_id).await.unwrap_err();
        assert!(
            err.to_string().contains("not a loopback address"),
            "unexpected error: {err}"
        );
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Suspended
        ));
    }

    /// 未真正建立 SSH 连接的客户端占位：隧道转发会失败并 warn，
    /// 但不影响 listener 生命周期相关断言。
    fn make_ssh_handle() -> SshClientHandle {
        let config = SessionConfig {
            id: Uuid::new_v4(),
            name: "tunnel-test".into(),
            folder_id: None,
            host: "127.0.0.1".into(),
            port: 22,
            protocol: Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "user".into(),
                has_password: false,
            },
            serial_config: None,
        };
        let auth = ResolvedAuthMethod::Password {
            username: "user".into(),
            password: String::new(),
        };
        Arc::new(tokio::sync::RwLock::new(SshClient::new(config, auth)))
    }

    /// 取一个当前空闲的回环端口（先绑定再释放，与 create_tunnel 的 bind
    /// 之间存在理论竞争，但在本机测试中足够稳定）。
    async fn free_loopback_port() -> u16 {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    }

    /// 集成验收（PROB-08）：建隧道 → 会话断开 → 隧道离开 Active、listener
    /// 停止接受新连接、TunnelStateChanged 已发布；重连后旧隧道不复活。
    ///
    /// 断开通过发布 SessionService 各断开路径（用户断开 detach_session、
    /// 远端 EOF 读循环清理、删除会话）共同广播的
    /// `ConnectionStateChanged { Disconnected }` 事件模拟。
    #[tokio::test]
    async fn session_disconnect_deactivates_tunnel_and_stops_listener() {
        let bus = Arc::new(EventBus::new());
        let mgr = Arc::new(TunnelManager::new(bus.clone()));
        let session_id = Uuid::new_v4();

        let port = free_loopback_port().await;
        let mut rule = make_rule("127.0.0.1", 9);
        rule.bind_port = port;
        let tunnel_id = mgr
            .create_tunnel(session_id, rule, Some(make_ssh_handle()))
            .await
            .unwrap();
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Active
        ));

        // listener 活着：本地 connect 立即成功
        let bind_addr = format!("127.0.0.1:{}", port);
        tokio::net::TcpStream::connect(&bind_addr)
            .await
            .expect("tunnel listener must accept before disconnect");

        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        bus.subscribe(move |event| {
            if let AppEvent::TunnelStateChanged { .. } = event {
                sink.lock().unwrap().push(event.clone());
            }
        });

        // 模拟会话断开广播
        bus.publish(AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Disconnected,
            info: None,
        });

        // 状态离开 Active 且发布了 TunnelStateChanged { Error }
        let state = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let state = mgr.get_tunnel(tunnel_id).await.unwrap().state;
                if !matches!(state, TunnelState::Active) {
                    break state;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("tunnel must leave Active after session disconnect");
        assert!(matches!(state, TunnelState::Error(_)));
        let mut deactivated_event_seen = false;
        for event in events.lock().unwrap().iter() {
            if let AppEvent::TunnelStateChanged {
                tunnel_id: id,
                state,
            } = event
            {
                if *id == tunnel_id && matches!(state, TunnelState::Error(_)) {
                    deactivated_event_seen = true;
                }
            }
        }
        assert!(
            deactivated_event_seen,
            "TunnelStateChanged(Error) must be published on session disconnect"
        );

        // listener 停止接受新连接：重试后连接必须被拒绝
        let mut stopped = false;
        for _ in 0..40 {
            if tokio::net::TcpStream::connect(&bind_addr).await.is_err() {
                stopped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            stopped,
            "tunnel listener must stop accepting connections after session disconnect"
        );

        // 重连（Connected 广播）后旧隧道不复活
        bus.publish(AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Connected,
            info: None,
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Error(_)
        ));

        // 重复断开广播幂等：状态不变，也不再重复发布事件
        let published = events.lock().unwrap().len();
        bus.publish(AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Disconnected,
            info: None,
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Error(_)
        ));
        assert_eq!(events.lock().unwrap().len(), published);
    }

    /// 集成验收（PROB-21）：suspend 后 listener 停止接受新连接；resume
    /// 重新 bind 并恢复接受连接。
    #[tokio::test]
    async fn suspend_stops_listener_and_resume_rebinds() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_id = Uuid::new_v4();

        let port = free_loopback_port().await;
        let mut rule = make_rule("127.0.0.1", 9);
        rule.bind_port = port;
        let tunnel_id = mgr
            .create_tunnel(session_id, rule, Some(make_ssh_handle()))
            .await
            .unwrap();

        let bind_addr = format!("127.0.0.1:{}", port);
        tokio::net::TcpStream::connect(&bind_addr)
            .await
            .expect("tunnel listener must accept before suspend");

        // 挂起：状态离开 Active，新连接被拒绝
        mgr.suspend_tunnel(tunnel_id).await.unwrap();
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Suspended
        ));
        let mut stopped = false;
        for _ in 0..40 {
            if tokio::net::TcpStream::connect(&bind_addr).await.is_err() {
                stopped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(stopped, "suspended tunnel must stop accepting connections");

        // 恢复：重新 bind，重新接受连接
        mgr.resume_tunnel(tunnel_id).await.unwrap();
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Active
        ));
        let mut accepted = false;
        for _ in 0..40 {
            if tokio::net::TcpStream::connect(&bind_addr).await.is_ok() {
                accepted = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(accepted, "resumed tunnel must accept connections again");
    }

    /// 集成验收（PROB-21）：resume 时端口被占用必须报错，隧道保持挂起；
    /// 端口释放后可再次 resume 恢复。
    #[tokio::test]
    async fn resume_reports_port_conflict_and_stays_suspended() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_id = Uuid::new_v4();

        let port = free_loopback_port().await;
        let mut rule = make_rule("127.0.0.1", 9);
        rule.bind_port = port;
        let tunnel_id = mgr
            .create_tunnel(session_id, rule, Some(make_ssh_handle()))
            .await
            .unwrap();
        mgr.suspend_tunnel(tunnel_id).await.unwrap();

        // 等 suspend 关闭监听 socket 后，用第三方 socket 占住端口
        let mut squatter = None;
        for _ in 0..40 {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => {
                    squatter = Some(l);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        let squatter = squatter.expect("port must be free after suspend");

        // 端口冲突：resume 报错且状态保持 Suspended
        let err = mgr.resume_tunnel(tunnel_id).await.unwrap_err();
        assert!(
            err.to_string().contains("Failed to bind"),
            "unexpected resume error: {}",
            err
        );
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Suspended
        ));

        // 端口释放后可再次恢复
        drop(squatter);
        mgr.resume_tunnel(tunnel_id).await.unwrap();
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Active
        ));
    }

    /// 集成验收（PROB-21）：suspend/resume 只在 Active/Suspended 之间转换，
    /// 其余状态返回错误而非静默假成功。
    #[tokio::test]
    async fn suspend_resume_reject_invalid_state_transitions() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_id = Uuid::new_v4();

        // Active 上直接 resume：拒绝
        let tunnel_id = mgr
            .create_tunnel(
                session_id,
                make_rule("example.com", 0),
                Some(make_ssh_handle()),
            )
            .await
            .unwrap();
        assert!(mgr.resume_tunnel(tunnel_id).await.is_err());
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Active
        ));

        // Suspended 上重复 suspend：拒绝
        mgr.suspend_tunnel(tunnel_id).await.unwrap();
        assert!(mgr.suspend_tunnel(tunnel_id).await.is_err());
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Suspended
        ));

        // 不存在的隧道：NotFound
        assert!(mgr.suspend_tunnel(Uuid::new_v4()).await.is_err());
        assert!(mgr.resume_tunnel(Uuid::new_v4()).await.is_err());

        // 会话断开后隧道进入 Error：不可 suspend，也不可 resume
        mgr.handle_session_disconnected(session_id).await;
        assert!(matches!(
            mgr.get_tunnel(tunnel_id).await.unwrap().state,
            TunnelState::Error(_)
        ));
        assert!(mgr.suspend_tunnel(tunnel_id).await.is_err());
        assert!(mgr.resume_tunnel(tunnel_id).await.is_err());
    }

    /// 断开清理只作用于对应会话的隧道，且重复调用幂等。
    #[tokio::test]
    async fn handle_session_disconnected_only_deactivates_matching_session() {
        let mgr = TunnelManager::new(Arc::new(EventBus::new()));
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();
        let tunnel_a = mgr
            .create_tunnel(
                session_a,
                make_rule("example.com", 0),
                Some(make_ssh_handle()),
            )
            .await
            .unwrap();
        let tunnel_b = mgr
            .create_tunnel(
                session_b,
                make_rule("example.com", 0),
                Some(make_ssh_handle()),
            )
            .await
            .unwrap();

        mgr.handle_session_disconnected(session_a).await;

        assert!(matches!(
            mgr.get_tunnel(tunnel_a).await.unwrap().state,
            TunnelState::Error(_)
        ));
        assert!(matches!(
            mgr.get_tunnel(tunnel_b).await.unwrap().state,
            TunnelState::Active
        ));

        // 幂等：再次调用不改变任何状态
        mgr.handle_session_disconnected(session_a).await;
        assert!(matches!(
            mgr.get_tunnel(tunnel_a).await.unwrap().state,
            TunnelState::Error(_)
        ));
        assert!(matches!(
            mgr.get_tunnel(tunnel_b).await.unwrap().state,
            TunnelState::Active
        ));
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let tmp =
            std::env::temp_dir().join(format!("rshell-tunnels-{}.toml", uuid::Uuid::new_v4()));
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let mgr = TunnelManager::new(bus).with_persistence(tmp.clone());

        // restore empty (file not exist)
        let pending = mgr.restore_pending_rules().await;
        assert!(pending.is_empty());

        // 持久化测试只注入规则，不需要建立真实 SSH 连接或监听器。
        let sid = Uuid::new_v4();
        let tid = Uuid::new_v4();
        mgr.tunnels.write().await.insert(
            tid,
            ActiveTunnel {
                id: tid,
                session_id: sid,
                rule: make_rule("example.com", 0),
                state: TunnelState::Active,
                bytes_transferred: 0,
                connections_count: 0,
                created_at: std::time::Instant::now(),
                ssh_client: make_ssh_handle(),
                listener_handle: None,
            },
        );
        mgr.save_to_disk().await;
        let saved = std::fs::read_to_string(&tmp).unwrap();
        assert!(saved.contains(&sid.to_string()));
        assert!(saved.contains("example.com"));

        mgr.close_tunnel(tid).await.unwrap();

        // 现在应能从磁盘读出
        let mgr2 = TunnelManager::new(Arc::new(crate::event_bus::EventBus::new()))
            .with_persistence(tmp.clone());
        let pending = mgr2.restore_pending_rules().await;
        assert!(pending.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_restore_handles_missing_file() {
        let tmp =
            std::env::temp_dir().join(format!("rshell-missing-{}.toml", uuid::Uuid::new_v4()));
        let mgr =
            TunnelManager::new(Arc::new(crate::event_bus::EventBus::new())).with_persistence(tmp);
        let pending = mgr.restore_pending_rules().await;
        assert!(pending.is_empty());
    }

    /// SOCKS5 握手解析端到端测试: 用 socks5_handshake 直接验证 (绕开 forward 阻塞)
    #[tokio::test]
    async fn test_socks5_handshake_ipv4() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut inbound, _)) = socks.accept().await {
                // handshake 后 server 调 drop(inbound) 让 client 收 EOF
                let r = socks5_handshake(&mut inbound).await;
                drop(inbound);
                r
            } else {
                Err("accept failed".to_string())
            }
        });

        let mut client = tokio::net::TcpStream::connect(socks_addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();

        let mut resp = [0u8; 2];
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.read_exact(&mut resp),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(resp, [0x05, 0x00]);

        // IPv4 request
        let req = vec![0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, 0x00, 0x50]; // port 80
        client.write_all(&req).await.unwrap();

        // Server handshake result
        let r = tokio::time::timeout(std::time::Duration::from_millis(500), server)
            .await
            .unwrap()
            .unwrap();
        let (host, port) = r.unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 80);
    }

    /// Domain ATYP=0x03 解析
    #[tokio::test]
    async fn test_socks5_handshake_domain() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut inbound, _)) = socks.accept().await {
                let r = socks5_handshake(&mut inbound).await;
                drop(inbound);
                r
            } else {
                Err("accept failed".to_string())
            }
        });

        let mut client = tokio::net::TcpStream::connect(socks_addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut resp = [0u8; 2];
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.read_exact(&mut resp),
        )
        .await
        .unwrap()
        .unwrap();

        let domain = b"example.com";
        let mut req = vec![0x05, 0x01, 0x00, 0x03, domain.len() as u8];
        req.extend_from_slice(domain);
        req.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&req).await.unwrap();

        let r = tokio::time::timeout(std::time::Duration::from_millis(500), server)
            .await
            .unwrap()
            .unwrap();
        let (host, port) = r.unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
    }

    /// 不支持的 cmd (BIND=2) 应写 REP=0x07 拒绝
    #[tokio::test]
    async fn test_socks5_handshake_rejects_bind() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut inbound, _)) = socks.accept().await {
                socks5_handshake(&mut inbound).await
            } else {
                Err("accept failed".to_string())
            }
        });

        let mut client = tokio::net::TcpStream::connect(socks_addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut resp = [0u8; 2];
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.read_exact(&mut resp),
        )
        .await
        .unwrap()
        .unwrap();

        // CMD=2 (BIND), 应被拒
        let req = vec![0x05, 0x02, 0x00, 0x01, 127, 0, 0, 1, 0x00, 0x50];
        client.write_all(&req).await.unwrap();

        let r = tokio::time::timeout(std::time::Duration::from_millis(500), server)
            .await
            .unwrap()
            .unwrap();
        assert!(r.is_err());
    }

    /// SOCKS5 bad greeting: client 发 VER=4 应被拒
    #[tokio::test]
    async fn test_socks5_bad_version_rejected() {
        use tokio::io::AsyncWriteExt;

        let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut inbound, _)) = socks.accept().await {
                let r = socks5_handshake(&mut inbound).await;
                drop(inbound);
                r
            } else {
                Err("accept failed".to_string())
            }
        });

        let mut client = tokio::net::TcpStream::connect(socks_addr).await.unwrap();
        client.write_all(&[0x04, 0x01, 0x00]).await.unwrap(); // bad ver

        let r = tokio::time::timeout(std::time::Duration::from_millis(500), server)
            .await
            .unwrap()
            .unwrap();
        assert!(r.is_err());
    }
}
