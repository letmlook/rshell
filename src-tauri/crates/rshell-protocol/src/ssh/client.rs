//! SSH 客户端实现
//!
//! 基于 russh 实现 SSH 连接、认证、数据收发和终端大小调整。

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 每个 pty 的输出目的地，按 russh 通道号索引。
///
/// 一条 SSH 连接可以开多个 session channel（= 多个 pty = 多个独立标签会话），
/// 因此不能再用「一个 channel id + 一个 sender」：那样第二个 pty 的数据会被
/// 丢弃，或把第一个 pty 的去向顶掉。
#[derive(Default)]
struct ShellOutput {
    /// russh 通道号 → 该 pty 的输出发送端
    routes: HashMap<u32, mpsc::UnboundedSender<Vec<u8>>>,
}

impl ShellOutput {
    /// 为一个新开的 pty 登记输出去向。
    fn register(&mut self, channel: u32, sender: mpsc::UnboundedSender<Vec<u8>>) {
        self.routes.insert(channel, sender);
    }

    /// 收到某通道的数据：只投给该通道对应的 pty。
    fn data(&self, channel: u32, data: &[u8]) {
        if let Some(sender) = self.routes.get(&channel) {
            let _ = sender.send(data.to_vec());
        }
    }

    /// 关闭某个 pty（`Some(channel)`）或整条连接上的全部 pty（`None`）。
    fn close(&mut self, channel: Option<u32>) {
        match channel {
            Some(id) => {
                self.routes.remove(&id);
            }
            None => self.routes.clear(),
        }
    }
}

use rshell_api::types::SessionConfig;
use ssh_key::HashAlg;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{Connection, ProtocolError};

/// 空闲会话 keepalive 探测间隔。
///
/// 双向静默时每 15s 向服务器发送一次带应答的 keepalive
/// （GLOBAL_REQUEST `keepalive@openssh.com`），防止空闲会话被强制断开、
/// 同时让死链在约 60s 内被检出：russh 收到对端任何数据即重置探测计数，
/// 连续 `keepalive_max`（russh 默认 3）+1 次探测无响应才以
/// `KeepaliveTimeout` 断开。它取代旧的 `inactivity_timeout = 30s`——
/// 该超时在双向静默（长时间无输出的命令、只读观察）时会把会话无提示
/// 强制断开。
const SSH_KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// R3-01：等待 pty actor 应答的上限。
///
/// actor 在 `select!` 分支里内联 `await channel.data(...)`。对端不读 stdin
/// （或只是链路很慢）时发送窗口会被填满，这次写就一直等到对端回
/// `WINDOW_ADJUST` 才返回——actor 在此期间**回不到 `select!`**，后续
/// `Send`/`Resize`/`Close` 全部滞留在 mpsc 里不被处理。若调用方无上限地
/// 等应答（`close_terminal` / `disconnect_ssh` 的老写法），断开流程会被
/// 永久挂住，进而连带卡住持有 `SshClient` 写锁的上层。
///
/// 该上限只用来把「永久挂起」降级为「一次可见的失败」：超时后调用方放弃
/// 等待继续拆除，actor 与通道由 `handle.disconnect()` 收尾。
const PTY_ACTOR_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 断开连接时等待 russh 发送 disconnect 消息的上限。
const SSH_DISCONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// R3-03：请求对端开 `direct-tcpip` 通道的上限。
///
/// 调用方在隧道 accept 路径上持有 `SshClient` 读锁跨越这次调用；russh 的
/// `wait_channel_confirmation` 自身没有超时，因此这里必须自己兜底，否则一个
/// 不响应的对端就能把该会话的所有写操作（新标签、关闭、断开）永久堵死。
const DIRECT_TCPIP_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 构建 russh 传输层配置：启用空闲 keepalive，不设 inactivity_timeout。
///
/// 窗口与包长保持 russh 默认值（2 MiB / 32 KiB）。曾试过放大到 16 MiB /
/// 65535，实测吞吐**完全没有变化**（2.65～2.78 MB/s，与默认一致），
/// 而 65535 偏离 RFC 4253 建议的 ≤32768，会牺牲对老实现的互操作性，
/// 故撤回。真正的吞吐瓶颈不是这两个参数，见 docs/08「传输吞吐」。
fn transport_config(keepalive_interval: std::time::Duration) -> russh::client::Config {
    russh::client::Config {
        keepalive_interval: Some(keepalive_interval),
        ..Default::default()
    }
}

/// 建立到 `addr` 的 SSH 传输 socket，并关闭 Nagle 算法。
///
/// 不用 `russh::client::connect`：它内部直接 `TcpStream::connect`，从不设
/// `TCP_NODELAY`，且 `russh::client::Config` 没有对应开关。Nagle 会把小写入攒起来
/// 等对端确认，而对端 TCP 的延迟 ACK 又在攒数据等满一个包——两者在局域网上叠加
/// 出数百毫秒级停顿。SFTP 是高频小包协议，正好踩中，实测吞吐被压到个位数 MB/s
/// （同为局域网的 Xshell 可达数十 MB/s）。Xshell 这类原生客户端都关闭了 Nagle。
///
/// 单独抽出以便对真实 socket 断言 nodelay，而不是只测注释。
async fn open_nodelay_socket(addr: &str) -> Result<tokio::net::TcpStream, ProtocolError> {
    let socket = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;
    socket
        .set_nodelay(true)
        .map_err(|e| ProtocolError::ConnectionFailed(format!("failed to set TCP_NODELAY: {e}")))?;
    Ok(socket)
}

/// 一个 pty 的句柄：请求发送端（Send/Resize/Close）+ 它对应的 russh 通道号。
struct TerminalHandle {
    sender: mpsc::Sender<ShellRequest>,
    channel: u32,
}

/// R3-01：等待 pty actor 应答，并给等待加上限。
///
/// 抽成独立函数是为了让「有界」这一性质能被快速单测覆盖——生产超时是
/// [`PTY_ACTOR_REPLY_TIMEOUT`]，测试传毫秒级的值即可，不必让整个测试套件
/// 为一个 10 秒的 sleep 买单。
///
/// `what` 只用于错误文案，便于定位是输入还是 resize 被卡住。
async fn await_actor_reply(
    reply: oneshot::Receiver<Result<(), ProtocolError>>,
    timeout: std::time::Duration,
    what: &str,
) -> Result<(), ProtocolError> {
    match tokio::time::timeout(timeout, reply).await {
        Ok(Ok(result)) => result,
        // sender 被丢弃：actor 任务结束
        Ok(Err(_)) => Err(ProtocolError::ConnectionClosed),
        Err(_) => Err(ProtocolError::ProtocolError(format!(
            "pty actor did not accept {what} within {timeout:?}; \
             the remote end is not reading (SSH send window full)"
        ))),
    }
}

/// 在已建立的连接上开一个 session channel，请求 PTY 与 shell。
///
/// 抽成自由函数是为了让 `spawn_shell_channel` 能把连接句柄**临时取出**再传入
/// （`russh::client::Handle` 不是 `Clone`，而借用 `self.handle` 会与后续对
/// `self` 其他字段的访问冲突）。
async fn open_pty_channel(
    handle: &mut russh::client::Handle<SshHandler>,
    cols: u32,
    rows: u32,
) -> Result<russh::Channel<russh::client::Msg>, ProtocolError> {
    // russh 0.48 的 request_pty/request_shell 走 &self，无需可变借用
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

    channel
        .request_pty(
            false,            // want_reply
            "xterm-256color", // term
            cols,             // col_width
            rows,             // row_height
            0,                // pix_width
            0,                // pix_height
            &[],              // terminal_modes
        )
        .await
        .map_err(|e| ProtocolError::ProtocolError(format!("PTY request failed: {}", e)))?;

    channel
        .request_shell(false)
        .await
        .map_err(|e| ProtocolError::ProtocolError(format!("Shell request failed: {}", e)))?;

    Ok(channel)
}

/// SSH 客户端
pub struct SshClient {
    config: SessionConfig,
    auth: Option<ResolvedAuthMethod>,
    /// 连接句柄
    handle: Option<russh::client::Handle<SshHandler>>,
    /// 主 pty（连接时创建）的请求发送端；保留单值是为了兼容既有 Connection 契约
    channel: Option<mpsc::Sender<ShellRequest>>,
    /// 主 pty 的通道号
    channel_id: Option<u32>,
    /// 全部 pty：terminal_id → 句柄。同一连接上的多个标签各自一个 pty
    terminals: HashMap<Uuid, TerminalHandle>,
    /// 接收数据的通道（主 pty；额外 pty 的接收端交给调用方）
    data_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    /// `Connection::recv` 尚未消费完的上一条消息余量。
    /// `recv_data()` 返回的是一条完整消息，若调用方 `buf` 装不下，
    /// 余量必须留到下次 `recv`，否则终端输出会静默丢失
    /// （与 serial 的 `pending_bytes` 同一契约，见 serial/mod.rs）。
    recv_pending: VecDeque<u8>,
    /// 发送数据的通道（供 Handler 使用）
    shell_output: Arc<Mutex<ShellOutput>>,
    /// 空闲 keepalive 探测间隔，默认 [`SSH_KEEPALIVE_INTERVAL`]。
    /// 测试可调小以压缩时间线。
    keepalive_interval: std::time::Duration,
}

/// 每次连接临时解析的认证材料。不得序列化或存入 SessionConfig。
pub enum ResolvedAuthMethod {
    Password {
        username: String,
        password: String,
    },
    PublicKey {
        username: String,
        key_path: PathBuf,
        passphrase: Option<String>,
    },
    KeyboardInteractive {
        username: String,
        password: Option<String>,
    },
}

enum ShellRequest {
    Send(Vec<u8>, oneshot::Sender<Result<(), ProtocolError>>),
    Resize(u32, u32, oneshot::Sender<Result<(), ProtocolError>>),
    Close(oneshot::Sender<Result<(), ProtocolError>>),
}

/// 主机密钥决策（从 UI 传回 SSH 层）
#[derive(Debug, Clone)]
pub struct HostKeyDecision {
    pub fingerprint: String,
    pub key_blob: String,
    pub accept: bool,
    pub permanent: bool, // true = 写入 known_hosts
}

/// 主机密钥决策"接收 + 发布"抽象
///
/// `SshHandler::check_server_key` 在握手中等待 UI 异步响应决策。
/// `SshClient` 接受一个 `Arc<dyn HostKeyDecisionSink>`：
/// 未知 key 时调用 `register_decision` 注册一个等待项、拿到一个 oneshot::Receiver，
/// 再 `publish_request` 让 UI 看到、最后等待 receiver。
///
/// 协议层（`rshell-protocol`）不依赖 `rshell-core`,所以这里用 trait object 解耦;
/// `rshell-core::security::host_key_decision::HostKeyDecisionRegistry` 是 trait 的
/// 标准实现。
pub trait HostKeyDecisionSink: Send + Sync {
    /// 注册一个待决策项
    fn register_decision(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>);
    /// 向 UI 端发布"请决策"通知
    fn publish_request(&self, info: HostKeyDecisionRequest);
    fn cancel_decision(&self, decision_id: Uuid);
}

/// 待决策的主机密钥信息（用于发布给 UI 端）
#[derive(Debug, Clone)]
pub struct HostKeyDecisionRequest {
    pub decision_id: Uuid,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub expected: String,
    pub public_key_blob: String,
}

/// SSH Handler 实现
///
/// 负责接收服务端数据并通过通道转发给上层；同时验证服务器主机密钥。
pub(crate) struct SshHandler {
    shell_output: Arc<Mutex<ShellOutput>>,
    /// 正在连接的主机名/IP（用于 known_hosts 查找）
    host: String,
    /// 端口
    port: u16,
    /// 已知的 known_hosts 文件路径（受控来源，见 `build_known_hosts_paths`）
    known_hosts_paths: Vec<PathBuf>,
    /// 主机密钥决策 sink（未知 key 时通过它注册 + 等待 UI 决策）
    host_key_sink: Option<Arc<dyn HostKeyDecisionSink>>,
}

impl SshHandler {
    /// 在 known_hosts 文件中查找匹配 (host, port) 的主机密钥，并与给定的公钥比对指纹
    fn verify_known_hosts(&self, server_key: &ssh_key::PublicKey) -> (bool, Option<String>) {
        let expected_fp = server_key.fingerprint(HashAlg::Sha256).to_string();
        let mut changed = None;

        for path in &self.known_hosts_paths {
            if !path.exists() {
                continue;
            }
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    debug!("Failed to read known_hosts {}: {}", path.display(), e);
                    continue;
                }
            };

            let (matches, previous) = self.scan_known_hosts(&content, server_key, &expected_fp);
            if matches {
                debug!("Host key matched entry in {}", path.display());
                return (true, None);
            }
            changed = changed.or(previous);
        }

        (false, changed)
    }

    /// 扫描 known_hosts 内容，匹配 host 模式 + 密钥指纹
    fn scan_known_hosts(
        &self,
        content: &str,
        server_key: &ssh_key::PublicKey,
        expected_fp: &str,
    ) -> (bool, Option<String>) {
        let mut changed = None;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // OpenSSH known_hosts 行格式：
            //   <host_pattern> <keytype> <base64-key> [comment]
            // 或者哈希化条目：
            //   |1|base64(salt)|base64(hash) <keytype> <base64-key> [comment]
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let host_field = parts[0];
            let keytype = parts[1];
            let key_blob = parts[2];

            // 跳过 hashed 条目（无法在不解码的情况下匹配 host 模式）
            if host_field.starts_with("|1|") {
                continue;
            }

            // 检查 host 模式是否匹配（逗号分隔多个 host）
            let host_matches = host_field
                .split(',')
                .any(|pattern| self.pattern_matches(pattern));
            if !host_matches {
                continue;
            }

            // 解析存储的公钥并比对指纹
            // OpenSSH 格式：key_blob 是 base64 编码的 wire-format 公钥
            let stored_key = ssh_key::PublicKey::from_openssh(&format!("{keytype} {key_blob}"));
            let stored_key = match stored_key {
                Ok(k) => k,
                Err(e) => {
                    debug!("Failed to parse known_hosts key (type={}): {}", keytype, e);
                    continue;
                }
            };

            let stored_fp = stored_key.fingerprint(HashAlg::Sha256).to_string();
            if stored_fp == expected_fp {
                // 额外确认 keytype 一致（防 base64 巧合匹配）
                if stored_key.algorithm().as_str() == server_key.algorithm().as_str() {
                    return (true, None);
                }
            }
            changed = changed.or(Some(stored_fp));
        }

        (false, changed)
    }

    /// 检查 OpenSSH host pattern（支持 [host]:port 与 host,）是否匹配当前连接
    fn pattern_matches(&self, pattern: &str) -> bool {
        let pattern = pattern.trim_end_matches(',');
        if pattern.is_empty() {
            return false;
        }

        let (host_part, port_part) = if let Some(idx) = pattern.find("]:") {
            // [1.2.3.4]:2222 或 [::1]:2222
            let host = &pattern[..idx + 1]; // 含 ']'
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let port = &pattern[idx + 2..];
            (host.to_string(), Some(port.to_string()))
        } else if pattern.parse::<std::net::IpAddr>().is_ok() {
            // 裸 IPv6 字面量（如 "::1"）：冒号切分会得到错误 host/port。
            // OpenSSH 对端口 22 的 IPv6 主机即写裸地址，按无端口=22 匹配（R2-06）
            (pattern.to_string(), None)
        } else if let Some(idx) = pattern.rfind(':') {
            // host:port 或 host（无端口）
            let host = &pattern[..idx];
            let port = &pattern[idx + 1..];
            (host.to_string(), Some(port.to_string()))
        } else {
            // 只有 host
            (pattern.to_string(), None)
        };

        // 主机名匹配：精确比较，或通配符 * (简化：仅 *.example.com)
        let host_ok = host_part == self.host
            || (host_part.starts_with("*.") && self.host.ends_with(&host_part[1..]));

        if !host_ok {
            return false;
        }

        match port_part {
            None => self.port == 22,
            Some(p) => p.parse::<u16>().map(|p| p == self.port).unwrap_or(false),
        }
    }
}

// russh 0.62 的 Handler 改用 RPITIT（`-> impl Future`）声明回调，不再是
// `#[async_trait]`。保留该属性会引入 `#[async_trait]` 的生命周期形参，导致
// 每个方法的签名都与 trait 声明不匹配。
impl russh::client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        debug!("SSH auth banner: {}", banner);
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = server_public_key.fingerprint(HashAlg::Sha256).to_string();

        debug!(
            host = %self.host,
            port = self.port,
            fingerprint = %fp,
            "Verifying server host key against known_hosts"
        );

        let (known, expected) = self.verify_known_hosts(server_public_key);
        if known {
            return Ok(true);
        }

        // ===== 未在 known_hosts 中找到匹配条目 → 等待 UI 决策 =====
        // russh 的回调是异步的，直接等待用户决策，不能阻塞 Tokio worker。
        let Some(sink) = self.host_key_sink.clone() else {
            // 没有 sink:保守策略,直接拒绝。协议层单元测试场景下走这条路径。
            warn!(
                host = %self.host,
                port = self.port,
                fingerprint = %fp,
                "Host key not found in known_hosts — rejecting (no decision sink)"
            );
            return Err(anyhow::anyhow!(
                "Host key for {}:{} not found in known_hosts (fingerprint {})",
                self.host,
                self.port,
                fp
            ));
        };

        // 1. 注册等待项,拿到 decision_id + oneshot receiver
        let (decision_id, rx) = sink.register_decision();

        // 2. 通知 UI 端"请决策"
        let key_blob = server_public_key
            .to_openssh()
            .map_err(|e| anyhow::anyhow!("Cannot encode server public key: {e}"))?;
        sink.publish_request(HostKeyDecisionRequest {
            decision_id,
            host: self.host.clone(),
            port: self.port,
            key_type: format!("{:?}", server_public_key.algorithm()),
            fingerprint: fp.clone(),
            expected: expected.unwrap_or_default(),
            public_key_blob: key_blob,
        });

        // 3. 等待 UI 端通过 AppCommand::DecideHostKey 唤醒
        let user_decided = tokio::time::timeout(std::time::Duration::from_secs(60), rx).await;
        match user_decided {
            Ok(Ok(decision)) => {
                if decision.accept {
                    // TrustOnce 或 TrustPermanent 都被接受，russh 继续握手。
                    // TrustPermanent 的写入由调用方（SessionService::connect）处理。
                    info!(
                        host = %self.host,
                        port = self.port,
                        fingerprint = %fp,
                        permanent = decision.permanent,
                        "User accepted unknown host key"
                    );
                    return Ok(true);
                }
                warn!(
                    host = %self.host,
                    port = self.port,
                    fingerprint = %fp,
                    "User rejected host key"
                );
                Err(anyhow::anyhow!(
                    "User rejected host key for {}:{}",
                    self.host,
                    self.port
                ))
            }
            Ok(Err(_)) | Err(_) => {
                sink.cancel_decision(decision_id);
                warn!(
                    host = %self.host,
                    port = self.port,
                    fingerprint = %fp,
                    "Host key decision channel closed — rejecting"
                );
                Err(anyhow::anyhow!(
                    "Host key decision channel closed for {}:{}",
                    self.host,
                    self.port
                ))
            }
        }
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        // 将收到的数据通过通道转发给上层
        self.shell_output.lock().unwrap().data(channel.into(), data);
        Ok(())
    }

    async fn disconnected(
        &mut self,
        reason: russh::client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        info!("SSH disconnected: {:?}", reason);
        self.shell_output.lock().unwrap().close(None);
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: russh::ChannelId,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        self.shell_output
            .lock()
            .unwrap()
            .close(Some(channel.into()));
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: russh::ChannelId,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        self.shell_output
            .lock()
            .unwrap()
            .close(Some(channel.into()));
        Ok(())
    }
}

/// 从本次连接的认证材料提取用户名
fn get_username(auth: &ResolvedAuthMethod) -> &str {
    match auth {
        ResolvedAuthMethod::Password { username, .. } => username,
        ResolvedAuthMethod::PublicKey { username, .. } => username,
        ResolvedAuthMethod::KeyboardInteractive { username, .. } => username,
    }
}

/// 构建 known_hosts 搜索路径（按优先级），默认只含受控来源。
///
/// 信任主机密钥的决定必须来自用户。当前工作目录可能被启动环境
/// （如 CLI 指定的目录）影响，目录里预置的 `known_hosts` 会在
/// `verify_known_hosts` 中被当作已信任条目，绕过用户决策弹窗，
/// 因此 cwd 相对路径默认禁用；本地开发确需时通过
/// `RSHELL_ALLOW_CWD_KNOWN_HOSTS=1` 显式开启（见 `connect_ssh`）。
fn build_known_hosts_paths(allow_cwd_relative: bool) -> Vec<PathBuf> {
    let mut known_hosts_paths: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        known_hosts_paths.push(home.join(".ssh").join("known_hosts"));
    }
    // rshell 自有 known_hosts 文件（由 HostKeyManager 维护）
    if let Some(mut data_dir) = dirs::data_local_dir() {
        data_dir.push("rshell");
        data_dir.push("known_hosts");
        known_hosts_paths.push(data_dir);
    }
    if allow_cwd_relative {
        known_hosts_paths.push(PathBuf::from("known_hosts"));
    }
    known_hosts_paths
}

impl SshClient {
    /// 创建新的 SSH 客户端
    pub fn new(config: SessionConfig, auth: ResolvedAuthMethod) -> Self {
        Self {
            config,
            auth: Some(auth),
            handle: None,
            channel: None,
            channel_id: None,
            terminals: HashMap::new(),
            data_rx: None,
            recv_pending: VecDeque::new(),
            shell_output: Arc::new(Mutex::new(ShellOutput::default())),
            keepalive_interval: SSH_KEEPALIVE_INTERVAL,
        }
    }

    /// 连接到 SSH 服务器
    ///
    /// `host_key_sink`: 注入决策通道（生产环境 = `HostKeyDecisionRegistry`）。
    /// 测试场景可传 None,这时遇到未知 host key 会直接拒绝（保守策略）。
    ///
    /// 连接成功后,`data_rx` 由 `SshClient` 内部持有,通过 `recv_data()` 访问。
    pub async fn connect_ssh(
        &mut self,
        host_key_sink: Option<Arc<dyn HostKeyDecisionSink>>,
    ) -> Result<(), ProtocolError> {
        // The attempt owns the only resolved auth value. Taking it before the
        // first await also releases it if TCP/host-key negotiation is cancelled.
        let auth = self.auth.take().ok_or_else(|| {
            ProtocolError::AuthFailed("Authentication material already consumed".into())
        })?;
        info!(
            "Connecting to SSH server {}:{}",
            self.config.host, self.config.port
        );

        // 输出不再在这里预建唯一通道：每个 pty 在 spawn_shell_channel 时按
        // russh 通道号单独登记到 ShellOutput（一条连接可以挂多个 pty）。

        // 创建 SSH 配置：空闲 keepalive 每 15s 探测一次，服务端应答会重置
        // 探测计数，空闲会话长期存活；死链由 keepalive_max（默认 3）在约
        // 60s 内检出。inactivity_timeout 保持禁用（见 transport_config）。
        let ssh_config = Arc::new(transport_config(self.keepalive_interval));

        // 构建 known_hosts 搜索路径：默认仅受控来源；cwd 相对路径须用
        // 显式环境变量开启，防止受攻击者影响的目录预置信任
        // （见 build_known_hosts_paths 文档）。
        let allow_cwd_known_hosts =
            std::env::var("RSHELL_ALLOW_CWD_KNOWN_HOSTS").is_ok_and(|v| v == "1");
        let known_hosts_paths = build_known_hosts_paths(allow_cwd_known_hosts);

        // 创建 Handler（带 host_key_sink）
        let handler = SshHandler {
            shell_output: self.shell_output.clone(),
            host: self.config.host.clone(),
            port: self.config.port,
            known_hosts_paths,
            host_key_sink,
        };

        // 连接到服务器。自行建 socket 以关闭 Nagle（见 open_nodelay_socket）。
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let socket = open_nodelay_socket(&addr).await?;
        let handle = russh::client::connect_stream(ssh_config, socket, handler)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

        self.handle = Some(handle);

        info!("SSH TCP connection established");

        // 进行认证
        self.authenticate(auth).await?;

        info!("SSH authentication successful");

        // 打开会话通道
        self.open_session().await?;

        info!("SSH session channel opened");

        Ok(())
    }

    /// 执行 SSH 认证
    async fn authenticate(&mut self, auth: ResolvedAuthMethod) -> Result<(), ProtocolError> {
        let handle = self
            .handle
            .as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        let username = get_username(&auth);

        match &auth {
            ResolvedAuthMethod::Password { password, .. } => {
                // russh 0.62 起 authenticate_* 返回 AuthResult 而非 bool：
                // 失败时还会带回服务端建议的后续认证方法。
                let result = handle
                    .authenticate_password(username, password)
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                if !result.success() {
                    return Err(ProtocolError::AuthFailed(
                        "Password authentication failed".to_string(),
                    ));
                }
            }
            ResolvedAuthMethod::PublicKey {
                key_path,
                passphrase,
                ..
            } => {
                // 加载私钥。russh-keys 已并入 russh::keys。
                let key = russh::keys::load_secret_key(key_path, passphrase.as_deref())
                    .map_err(|e| ProtocolError::AuthFailed(format!("Failed to load key: {e}")))?;

                // RSA 私钥必须显式带上哈希算法：传 None 会退回已被多数服务端
                // 拒绝的 sha-rsa(SHA-1)。best_supported_rsa_hash 会与服务端协商。
                let hash_alg = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?
                    .flatten();

                let result = handle
                    .authenticate_publickey(
                        username,
                        russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                    )
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                if !result.success() {
                    return Err(ProtocolError::AuthFailed(
                        "Public key authentication failed".to_string(),
                    ));
                }
            }
            ResolvedAuthMethod::KeyboardInteractive { password, .. } => {
                // 键盘交互认证
                let response = handle
                    .authenticate_keyboard_interactive_start(username, None::<String>)
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                match response {
                    russh::client::KeyboardInteractiveAuthResponse::Success => {}
                    russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                        prompts, ..
                    } => {
                        // 使用配置中的密码（如果有），否则用空字符串
                        let pwd = password.as_deref().unwrap_or("");
                        let responses: Vec<String> =
                            prompts.iter().map(|_| pwd.to_string()).collect();

                        let response = handle
                            .authenticate_keyboard_interactive_respond(responses)
                            .await
                            .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                        match response {
                            russh::client::KeyboardInteractiveAuthResponse::Success => {}
                            _ => {
                                return Err(ProtocolError::AuthFailed(
                                    "Keyboard-interactive authentication failed".to_string(),
                                ));
                            }
                        }
                    }
                    russh::client::KeyboardInteractiveAuthResponse::Failure { .. } => {
                        return Err(ProtocolError::AuthFailed(
                            "Keyboard-interactive authentication failed".to_string(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// 主 pty 的 terminal_id。
    ///
    /// 约定：**主 pty 以会话 id 寻址**。前端首标签不做额外生成，直接把
    /// `session_id` 当 terminal_id 传下来（壳层的输出路由、核心层的
    /// `send_data`/`resize` 全用这个键）。若这里改成随机 uuid，输入会因为
    /// `terminals` 查不到句柄而被静默丢弃——输出正常、键盘无反应。
    fn primary_terminal_id(&self) -> Uuid {
        self.config.id
    }

    /// 打开会话通道并请求 PTY（主 pty，连接建立时创建）
    async fn open_session(&mut self) -> Result<(), ProtocolError> {
        let primary = self.primary_terminal_id();
        let (handle, rx) = self.spawn_shell_channel(primary, 80, 24).await?;
        // 主 pty 的请求端另外记一份，供既有 Connection 契约（send_data/resize）使用
        self.channel = Some(handle.sender.clone());
        self.channel_id = Some(handle.channel);
        self.terminals.insert(primary, handle);
        self.data_rx = Some(rx);
        Ok(())
    }

    /// 在**已建立的连接**上再开一个 pty（独立 session channel + 独立 shell）。
    ///
    /// 同一个 `terminal_id` 重复调用会报错而不是开出第二个 pty——否则同一个
    /// 标签会多挂一个用户看不见的 shell。
    pub async fn open_terminal(
        &mut self,
        terminal_id: Uuid,
        cols: u32,
        rows: u32,
    ) -> Result<mpsc::UnboundedReceiver<Vec<u8>>, ProtocolError> {
        if self.terminals.contains_key(&terminal_id) {
            return Err(ProtocolError::ProtocolError(format!(
                "terminal {terminal_id} already exists on this connection"
            )));
        }
        let (handle, rx) = self.spawn_shell_channel(terminal_id, cols, rows).await?;
        debug!(terminal_id = %terminal_id, channel = handle.channel, "extra pty opened");
        self.terminals.insert(terminal_id, handle);
        Ok(rx)
    }

    /// 建立一个 pty：开 session channel、请求 PTY 与 shell，起 actor 独占该 channel，
    /// 并把输出登记到 `ShellOutput`。返回 `(句柄, 输出接收端)`。
    ///
    /// `terminal_id` 由调用方给定：主 pty 传会话 id，附加 pty 传各自的 id。
    /// 这里**不能**自己生成随机 id——那样前端传来的 terminal_id 就查不到句柄。
    async fn spawn_shell_channel(
        &mut self,
        terminal_id: Uuid,
        cols: u32,
        rows: u32,
    ) -> Result<(TerminalHandle, mpsc::UnboundedReceiver<Vec<u8>>), ProtocolError> {
        debug!(terminal_id = %terminal_id, cols, rows, "opening pty session channel");
        // russh 的 Handle 不是 Clone，而 `self.handle.as_ref()` 的借用会与后面
        // 对 self 其他字段的访问冲突。改成「临时取出 → 建通道 → 无论成败都放回」。
        let mut handle = self
            .handle
            .take()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;
        let opened = open_pty_channel(&mut handle, cols, rows).await;
        self.handle = Some(handle);
        let mut channel = opened?;
        let channel_id: u32 = channel.id().into();

        let (out_tx, out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        self.shell_output
            .lock()
            .unwrap()
            .register(channel_id, out_tx);

        let (tx, mut requests) = mpsc::channel(32);
        let output = self.shell_output.clone();
        // russh 0.48 has no channel split API. Own the shell channel in an
        // actor so its unbounded receive queue is drained continuously while
        // callers can send/resize without holding a client lock for a read.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    request = requests.recv() => match request {
                        Some(ShellRequest::Send(data, reply)) => {
                            let _ = reply.send(channel.data(std::io::Cursor::new(data)).await
                                .map_err(|e| ProtocolError::ProtocolError(e.to_string())));
                        }
                        Some(ShellRequest::Resize(cols, rows, reply)) => {
                            let _ = reply.send(channel.window_change(cols, rows, 0, 0).await
                                .map_err(|e| ProtocolError::ProtocolError(e.to_string())));
                        }
                        Some(ShellRequest::Close(reply)) => {
                            let _ = reply.send(channel.close().await
                                .map_err(|e| ProtocolError::ProtocolError(e.to_string())));
                            break;
                        }
                        None => { let _ = channel.close().await; break; }
                    },
                    message = channel.wait() => {
                        // Output is routed once by SshHandler. Drain the library's
                        // duplicate messages, including status and window updates.
                        if matches!(message, None | Some(russh::ChannelMsg::Eof | russh::ChannelMsg::Close)) { break; }
                    }
                }
            }
            drop(requests);
            output.lock().unwrap().close(Some(channel_id));
        });

        Ok((
            TerminalHandle {
                sender: tx,
                channel: channel_id,
            },
            out_rx,
        ))
    }

    /// 关闭一个 pty（标签关闭）。未知 id 视为已关闭，幂等。
    pub async fn close_terminal(&mut self, terminal_id: Uuid) {
        if let Some(handle) = self.terminals.remove(&terminal_id) {
            let (reply, received) = oneshot::channel();
            if handle.sender.send(ShellRequest::Close(reply)).await.is_ok() {
                // R3-01：actor 卡在写上时永远回不到 select!，无限等应答会挂死
                // 断开流程（其调用方还持着 SshClient 写锁）。超时即放弃等待：
                // 丢弃 sender 后 actor 的 `None` 分支会自行 close 通道。
                if tokio::time::timeout(PTY_ACTOR_REPLY_TIMEOUT, received)
                    .await
                    .is_err()
                {
                    warn!(
                        terminal_id = %terminal_id,
                        "close_terminal: pty actor busy (send window full?), reply timed out; continuing teardown"
                    );
                }
            }
        }
    }

    /// 向指定 pty 发送输入。
    pub async fn send_data_to(&self, terminal_id: Uuid, data: &[u8]) -> Result<(), ProtocolError> {
        let handle = self.terminals.get(&terminal_id).ok_or_else(|| {
            ProtocolError::ProtocolError(format!(
                "terminal {terminal_id} not found on this connection"
            ))
        })?;
        let (reply, received) = oneshot::channel();
        handle
            .sender
            .send(ShellRequest::Send(data.to_vec(), reply))
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        // R3-01：actor 卡在一次写上时不会处理后续请求，这里必须有上限，
        // 否则一次按键就能把调用方永久挂住。
        await_actor_reply(received, PTY_ACTOR_REPLY_TIMEOUT, "input").await
    }

    /// 调整指定 pty 的窗口尺寸。
    pub async fn resize_terminal_of(
        &self,
        terminal_id: Uuid,
        cols: u32,
        rows: u32,
    ) -> Result<(), ProtocolError> {
        let handle = self.terminals.get(&terminal_id).ok_or_else(|| {
            ProtocolError::ProtocolError(format!(
                "terminal {terminal_id} not found on this connection"
            ))
        })?;
        let (reply, received) = oneshot::channel();
        handle
            .sender
            .send(ShellRequest::Resize(cols, rows, reply))
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        await_actor_reply(received, PTY_ACTOR_REPLY_TIMEOUT, "resize").await
    }

    /// 该连接上已登记的 pty 数量（诊断/测试用）。
    pub fn terminal_count(&self) -> usize {
        self.terminals.len()
    }

    /// 断开连接
    pub async fn disconnect_ssh(&mut self) -> Result<(), ProtocolError> {
        // R3-01：**不再**逐个 await close_terminal 的应答。旧写法在 actor 卡于
        // `channel.data()`（对端不读 stdin、发送窗口打满）时会永久挂住，而调用方
        // 此刻正持有 `SshClient` 写锁 —— 整个会话就此再也关不掉，只能重启应用。
        //
        // 直接丢弃 sender 即可：actor 的 `requests.recv()` 收到 `None` 会走
        // `None => { let _ = channel.close().await; break }` 自行收尾；即便它此刻
        // 正卡在写上，紧随其后的 `handle.disconnect()` 拆掉 russh 会话也会让那次
        // 写返回错误、任务结束。
        self.terminals.clear();
        self.channel = None;
        self.channel_id = None;

        if let Some(handle) = self.handle.take() {
            // 同样加上限：disconnect 不应成为第二个可能永久挂起的点。
            if tokio::time::timeout(SSH_DISCONNECT_TIMEOUT, async {
                let _ = handle
                    .disconnect(russh::Disconnect::ByApplication, "User disconnect", "en")
                    .await;
            })
            .await
            .is_err()
            {
                warn!("SSH disconnect did not complete within {SSH_DISCONNECT_TIMEOUT:?}; dropping handle");
            }
        }

        self.shell_output.lock().unwrap().close(None);
        self.data_rx = None;
        // 断开后残留缓冲作废，后续 recv 因 data_rx 已置 None 报 ConnectionClosed
        self.recv_pending.clear();

        info!("SSH disconnected");
        Ok(())
    }

    /// 发送数据到**主 pty**（`Connection::send` 契约）。
    ///
    /// 多标签走 [`Self::send_data_to`]：每个标签一个 pty，输入互不串台。
    pub async fn send_data(&self, data: &[u8]) -> Result<(), ProtocolError> {
        let channel = self
            .channel
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;

        let (reply, received) = oneshot::channel();
        channel
            .send(ShellRequest::Send(data.to_vec(), reply))
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        received
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?
    }

    /// 接收数据（从通道读取）
    pub async fn recv_data(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        let rx = self
            .data_rx
            .as_mut()
            .ok_or(ProtocolError::ConnectionClosed)?;

        match rx.recv().await {
            Some(data) => Ok(Some(data)),
            None => Err(ProtocolError::ConnectionClosed),
        }
    }

    /// Move the output stream into a dedicated reader task so it never holds the
    /// client write lock while waiting for output. Sending and SFTP can then run
    /// concurrently with the terminal reader.
    pub fn take_data_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<Vec<u8>>> {
        self.data_rx.take()
    }

    /// 调整终端大小
    pub async fn resize_terminal(&self, cols: u32, rows: u32) -> Result<(), ProtocolError> {
        let channel = self
            .channel
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;

        let (reply, received) = oneshot::channel();
        channel
            .send(ShellRequest::Resize(cols, rows, reply))
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        received
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?
    }

    /// 获取 SSH 连接句柄（用于打开 SFTP 通道等）
    #[allow(dead_code)]
    pub(crate) fn handle(&self) -> Option<&russh::client::Handle<SshHandler>> {
        self.handle.as_ref()
    }

    /// 打开一个新的 SFTP 通道
    pub async fn open_sftp_channel(
        &self,
    ) -> Result<russh::Channel<russh::client::Msg>, ProtocolError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;

        let channel = handle.channel_open_session().await.map_err(|e| {
            ProtocolError::ConnectionFailed(format!("SFTP channel open failed: {}", e))
        })?;

        // 请求 sftp 子系统
        channel
            .request_subsystem(false, "sftp")
            .await
            .map_err(|e| {
                ProtocolError::ProtocolError(format!("SFTP subsystem request failed: {}", e))
            })?;

        Ok(channel)
    }

    /// 打开一个 `direct-tcpip` 通道（RFC 4254 §7.2）
    ///
    /// 用于 SSH 隧道 / 端口转发：让服务器代为连接 `host:port`，
    /// 之后把客户端 TCP 流和该 channel 做双向 copy 即可。
    ///
    /// 调用方负责：
    /// 1. 拿到 channel 后调 `.make_reader()` / `.make_writer()` 拿 AsyncRead/AsyncWrite
    /// 2. 用 `tokio::io::copy_bidirectional` 在 `TcpStream` 和 channel 之间搬运
    /// 3. 关闭时调 `channel.eof()` + `channel.close()`
    ///
    /// R3-03：channel-open 必须**有上限**。调用方（隧道 accept 路径）持有
    /// `SshClient` 的读锁跨越这次调用，而 tokio `RwLock` 写优先：对端不回应
    /// channel-open 时 russh 的 `wait_channel_confirmation` 会一直等，既没有
    /// 自带超时也不该由我们无限等下去——那会让 `open_terminal` /
    /// `close_terminal` / `disconnect_ssh` 全部堵在写者队列里，会话再也关不掉。
    pub async fn open_direct_tcpip(
        &self,
        host: &str,
        port: u32,
    ) -> Result<russh::Channel<russh::client::Msg>, ProtocolError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;
        match tokio::time::timeout(
            DIRECT_TCPIP_OPEN_TIMEOUT,
            handle.channel_open_direct_tcpip(host, port, "127.0.0.1", 0),
        )
        .await
        {
            Ok(Ok(channel)) => Ok(channel),
            Ok(Err(e)) => Err(ProtocolError::ConnectionFailed(format!(
                "direct-tcpip open failed: {e}"
            ))),
            Err(_) => Err(ProtocolError::ConnectionFailed(format!(
                "direct-tcpip open to {host}:{port} timed out after {DIRECT_TCPIP_OPEN_TIMEOUT:?}; \
                 the SSH peer did not answer the channel request"
            ))),
        }
    }
}

#[async_trait::async_trait]
impl Connection for SshClient {
    async fn connect(&mut self) -> Result<(), ProtocolError> {
        // Connection trait 不带 sink,默认 None,遇到未知 host key 时保守拒绝。
        // 生产环境通过 SshClient::connect_ssh(sink) 注入决策通道。
        self.connect_ssh(None).await
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        self.disconnect_ssh().await
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError> {
        self.send_data(data).await
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        // 先消费上一条消息留在 recv_pending 里的余量
        if !self.recv_pending.is_empty() {
            let count = buf.len().min(self.recv_pending.len());
            for byte in &mut buf[..count] {
                *byte = self.recv_pending.pop_front().unwrap();
            }
            return Ok(count);
        }
        // 从通道读取一条完整消息；装不进 buf 的余量留待下次 recv，
        // 不能截断丢弃——那会造成终端输出静默丢失
        match self.recv_data().await? {
            Some(data) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                self.recv_pending.extend(&data[len..]);
                Ok(len)
            }
            None => Err(ProtocolError::ConnectionClosed),
        }
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ProtocolError> {
        self.resize_terminal(cols as u32, rows as u32).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::AuthMethod;

    /// 系统的密码学随机源。ssh-key 0.7 依赖的 rand_core 0.10 移除了 `OsRng`，
    /// 改用 `getrandom::SysRng`，并用 `UnwrapErr` 适配成 `CryptoRng`。
    fn os_rng() -> getrandom::rand_core::UnwrapErr<getrandom::SysRng> {
        getrandom::rand_core::UnwrapErr(getrandom::SysRng)
    }

    // 回归：曾用 russh::client::connect，它从不设 TCP_NODELAY。Nagle 与对端延迟
    // ACK 在局域网上叠加出数百毫秒停顿，把 SFTP 吞吐压到个位数 MB/s。这里对真实
    // socket 断言 nodelay 已生效，防止改回 russh 的封装。
    #[tokio::test]
    async fn transport_socket_disables_nagle() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let socket = open_nodelay_socket(&format!("127.0.0.1:{port}"))
            .await
            .expect("socket 应建立成功");

        assert!(
            socket.nodelay().unwrap(),
            "SSH 传输 socket 必须关闭 Nagle：Nagle 与延迟 ACK 叠加会让 SFTP \
             吞吐掉到个位数 MB/s，而同链路的专业客户端可达数十 MB/s"
        );
    }

    // 端口无人监听时必须返回可读错误，不能静默成功
    #[tokio::test]
    async fn transport_socket_reports_unreachable_peer() {
        // 绑定后立即 drop，得到一个几乎确定没有监听者的端口
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let err = open_nodelay_socket(&format!("127.0.0.1:{port}"))
            .await
            .expect_err("无监听者时必须失败");
        assert!(
            matches!(err, ProtocolError::ConnectionFailed(_)),
            "实际错误: {err:?}"
        );
    }

    #[test]
    fn transport_config_keeps_alive_and_drops_no_idle_sessions() {
        let config = transport_config(SSH_KEEPALIVE_INTERVAL);
        assert_eq!(config.keepalive_interval, Some(SSH_KEEPALIVE_INTERVAL));
        assert!(
            config.inactivity_timeout.is_none(),
            "inactivity_timeout 会把双向静默的空闲会话（无输出的长命令、只读观察）\
             在超时后无提示强制断开，必须保持禁用；死链检出由 keepalive_max 负责"
        );
    }

    #[test]
    fn resolved_password_is_kept_outside_public_session_config() {
        let config = SessionConfig {
            id: Uuid::nil(),
            name: "test".into(),
            folder_id: None,
            host: "example.test".into(),
            port: 22,
            protocol: rshell_api::types::Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "alice".into(),
                has_password: true,
            },
            serial_config: None,
        };
        let auth = ResolvedAuthMethod::Password {
            username: "alice".into(),
            password: "sample-secret-password".into(),
        };
        let client = SshClient::new(config, auth);
        assert!(matches!(
            client.config.auth_method,
            AuthMethod::Password {
                has_password: true,
                ..
            }
        ));
        assert!(
            matches!(client.auth, Some(ResolvedAuthMethod::Password { password, .. }) if password == "sample-secret-password")
        );
    }

    #[test]
    fn resolved_public_key_passphrase_is_kept_outside_public_session_config() {
        let config = SessionConfig {
            id: Uuid::nil(),
            name: "test".into(),
            folder_id: None,
            host: "example.test".into(),
            port: 22,
            protocol: rshell_api::types::Protocol::SSH,
            auth_method: AuthMethod::PublicKey {
                username: "alice".into(),
                key_path: "/tmp/id_ed25519".into(),
                has_passphrase: true,
            },
            serial_config: None,
        };
        let client = SshClient::new(
            config,
            ResolvedAuthMethod::PublicKey {
                username: "alice".into(),
                key_path: "/tmp/id_ed25519".into(),
                passphrase: Some("sample-secret-passphrase".into()),
            },
        );
        assert!(matches!(
            client.config.auth_method,
            AuthMethod::PublicKey {
                has_passphrase: true,
                ..
            }
        ));
        assert!(
            matches!(client.auth, Some(ResolvedAuthMethod::PublicKey { passphrase: Some(value), .. }) if value == "sample-secret-passphrase")
        );
    }

    struct AcceptTestHost;
    impl HostKeyDecisionSink for AcceptTestHost {
        fn register_decision(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>) {
            let (tx, rx) = oneshot::channel();
            tx.send(HostKeyDecision {
                fingerprint: String::new(),
                key_blob: String::new(),
                accept: true,
                permanent: false,
            })
            .unwrap();
            (Uuid::new_v4(), rx)
        }
        fn publish_request(&self, _: HostKeyDecisionRequest) {}
        fn cancel_decision(&self, _: Uuid) {}
    }

    #[derive(Default)]
    struct OutputTestServer {
        shell: Option<russh::ChannelId>,
        allowed_key: Option<ssh_key::PublicKey>,
    }

    // 与 SshHandler 同理：russh 0.62 的 server::Handler 也不再是 `#[async_trait]`。
    impl russh::server::Handler for OutputTestServer {
        type Error = russh::Error;
        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> Result<russh::server::Auth, Self::Error> {
            Ok(if user == "test" && password == "test" {
                russh::server::Auth::Accept
            } else {
                russh::server::Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                }
            })
        }
        async fn auth_publickey(
            &mut self,
            user: &str,
            key: &ssh_key::PublicKey,
        ) -> Result<russh::server::Auth, Self::Error> {
            Ok(
                if user == "test" && self.allowed_key.as_ref() == Some(key) {
                    russh::server::Auth::Accept
                } else {
                    russh::server::Auth::Reject {
                        proceed_with_methods: None,
                        partial_success: false,
                    }
                },
            )
        }
        // 0.62 起 reply 是 `ChannelOpenHandle`：必须显式 accept，否则丢 handle
        // 等同于拒绝，会话通道开不起来。
        async fn channel_open_session(
            &mut self,
            _channel: russh::Channel<russh::server::Msg>,
            reply: russh::server::ChannelOpenHandle,
            _session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }
        async fn shell_request(
            &mut self,
            channel: russh::ChannelId,
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            self.shell = Some(channel);
            session.channel_success(channel)?;
            session.data(channel, b"shell output".to_vec())?;
            Ok(())
        }
        async fn subsystem_request(
            &mut self,
            channel: russh::ChannelId,
            _: &str,
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.data(channel, b"binary sftp payload".to_vec())?;
            session.eof(channel)?;
            session.close(channel)?;
            if let Some(shell) = self.shell {
                session.eof(shell)?;
            }
            Ok(())
        }
        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.data(channel, data.to_vec())?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn ssh_loopback_isolates_subsystem_payload_and_closes_shell_receiver() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let key = ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let config = russh::server::Config {
                keys: vec![key],
                ..Default::default()
            };
            let running =
                russh::server::run_stream(Arc::new(config), stream, OutputTestServer::default())
                    .await
                    .unwrap();
            let _ = running.await;
        });
        let mut client = SshClient::new(
            SessionConfig {
                id: Uuid::new_v4(),
                name: "loopback".into(),
                folder_id: None,
                host: "127.0.0.1".into(),
                port,
                protocol: rshell_api::types::Protocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "test".into(),
                    has_password: true,
                },
                serial_config: None,
            },
            ResolvedAuthMethod::Password {
                username: "test".into(),
                password: "test".into(),
            },
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            client
                .connect_ssh(Some(Arc::new(AcceptTestHost)))
                .await
                .unwrap();
            assert!(
                client.auth.is_none(),
                "authenticated client retains credentials"
            );
            let mut output = client.take_data_receiver().unwrap();
            assert_eq!(output.recv().await.unwrap(), b"shell output");
            client.send_data(b"input echo").await.unwrap();
            assert_eq!(output.recv().await.unwrap(), b"input echo");
            client.resize_terminal(100, 40).await.unwrap();
            let _subsystem = client.open_sftp_channel().await.unwrap();
            assert_eq!(output.recv().await, None);
            client.disconnect_ssh().await.unwrap();
        })
        .await
        .unwrap();
        server.await.unwrap();
    }

    /// 空闲会话在双向静默中存活（PROB-07 回归测试）。
    ///
    /// 把 keepalive 间隔压到 100ms，静默 700ms 覆盖 `keepalive_max`
    /// （russh 默认 3）+1 个探测周期：若保活失效（探测无应答），russh
    /// 会在第 5 次探测时报 `KeepaliveTimeout` 断开，随后的回显断言失败。
    /// 静默期间无任何业务数据往来，会话必须保持可用。
    #[tokio::test]
    async fn idle_session_survives_bidirectional_silence_via_keepalive() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let key = ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let config = russh::server::Config {
                keys: vec![key],
                ..Default::default()
            };
            let running =
                russh::server::run_stream(Arc::new(config), stream, OutputTestServer::default())
                    .await
                    .unwrap();
            let _ = running.await;
        });
        let mut client = SshClient::new(
            SessionConfig {
                id: Uuid::new_v4(),
                name: "idle keepalive".into(),
                folder_id: None,
                host: "127.0.0.1".into(),
                port,
                protocol: rshell_api::types::Protocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "test".into(),
                    has_password: true,
                },
                serial_config: None,
            },
            ResolvedAuthMethod::Password {
                username: "test".into(),
                password: "test".into(),
            },
        );
        client.keepalive_interval = std::time::Duration::from_millis(100);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            client
                .connect_ssh(Some(Arc::new(AcceptTestHost)))
                .await
                .unwrap();
            let mut output = client.take_data_receiver().unwrap();
            assert_eq!(output.recv().await.unwrap(), b"shell output");
            // 双向静默：客户端不发送、服务器不输出任何业务数据。
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            // 静默结束后会话仍可用：输入被服务器回显。
            client.send_data(b"still alive").await.unwrap();
            assert_eq!(output.recv().await.unwrap(), b"still alive");
            client.disconnect_ssh().await.unwrap();
        })
        .await
        .unwrap();
        server.await.unwrap();
    }

    fn password_client(port: u16, password: &str) -> SshClient {
        SshClient::new(
            SessionConfig {
                id: Uuid::new_v4(),
                name: "credential boundary".into(),
                folder_id: None,
                host: "127.0.0.1".into(),
                port,
                protocol: rshell_api::types::Protocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "test".into(),
                    has_password: true,
                },
                serial_config: None,
            },
            ResolvedAuthMethod::Password {
                username: "test".into(),
                password: password.into(),
            },
        )
    }

    async fn auth_server(
        allowed_key: Option<ssh_key::PublicKey>,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let config = russh::server::Config {
                keys: vec![
                    ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519)
                        .unwrap(),
                ],
                auth_rejection_time: std::time::Duration::ZERO,
                ..Default::default()
            };
            if let Ok(running) = russh::server::run_stream(
                Arc::new(config),
                stream,
                OutputTestServer {
                    shell: None,
                    allowed_key,
                },
            )
            .await
            {
                let _ = running.await;
            }
        });
        (port, server)
    }

    #[tokio::test]
    async fn connection_recv_keeps_overflow_bytes_for_next_call() {
        // Connection::recv 契约：一条消息大于 buf 时余量必须留在客户端，
        // 多次 recv 的字节总和按序无损（与 serial 的 pending_bytes 同一契约）。
        let mut client = password_client(0, "unused");
        let (data_tx, data_rx) = mpsc::unbounded_channel();
        client.data_rx = Some(data_rx);

        // 256 字节的消息远大于 16 字节的 buf；第二条小消息验证
        // 余量耗尽后新消息走正常路径且不与前一条串流。
        let big: Vec<u8> = (0..=255u8).collect();
        let small = b"ssh-echo-ok".to_vec();
        data_tx.send(big.clone()).unwrap();
        data_tx.send(small.clone()).unwrap();
        drop(data_tx); // 消息消费完后 recv 应报 ConnectionClosed

        let mut expected = big;
        expected.extend_from_slice(&small);

        let mut buf = [0u8; 16];
        let mut received = Vec::new();
        loop {
            match Connection::recv(&mut client, &mut buf).await {
                Ok(n) => received.extend_from_slice(&buf[..n]),
                Err(ProtocolError::ConnectionClosed) => break,
                Err(e) => panic!("unexpected recv error: {e:?}"),
            }
        }
        assert_eq!(
            received, expected,
            "大于 buf 的消息经多次 recv 必须字节无损"
        );
    }

    #[tokio::test]
    async fn wrong_password_is_rejected_and_dropped() {
        let (port, server) = auth_server(None).await;
        let mut client = password_client(port, "wrong-password");
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client.connect_ssh(Some(Arc::new(AcceptTestHost))),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(ProtocolError::AuthFailed(_))));
        assert!(client.auth.is_none());
        client.disconnect_ssh().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn encrypted_private_key_authenticates_and_drops_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ed25519");
        let key = ssh_key::PrivateKey::random(&mut os_rng(), ssh_key::Algorithm::Ed25519).unwrap();
        key.encrypt(&mut os_rng(), "key-passphrase")
            .unwrap()
            .write_openssh_file(&path, ssh_key::LineEnding::LF)
            .unwrap();
        let (port, server) = auth_server(Some(key.public_key().clone())).await;
        let mut client = password_client(port, "unused");
        client.config.auth_method = AuthMethod::PublicKey {
            username: "test".into(),
            key_path: path.clone(),
            has_passphrase: true,
        };
        client.auth = Some(ResolvedAuthMethod::PublicKey {
            username: "test".into(),
            key_path: path,
            passphrase: Some("key-passphrase".into()),
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.connect_ssh(Some(Arc::new(AcceptTestHost))),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(client.auth.is_none());
        client.disconnect_ssh().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_and_failed_handshake_drop_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = password_client(port, "attempt-secret");
        // A connected TCP socket that never speaks SSH leaves authentication pending.
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(50),
            client.connect_ssh(None)
        )
        .await
        .is_err());
        assert!(client.auth.is_none());
        drop(listener);
        let mut failed = password_client(port, "attempt-secret");
        assert!(failed.connect_ssh(None).await.is_err());
        assert!(failed.auth.is_none());
    }

    #[tokio::test]
    async fn shell_output_excludes_other_channels_and_ends_on_shell_eof() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut output = ShellOutput::default();
        output.register(7, tx);
        output.data(9, b"sftp payload");
        output.data(7, b"shell");
        output.close(Some(9));
        assert_eq!(rx.recv().await.unwrap(), b"shell");
        assert!(rx.try_recv().is_err());
        output.close(Some(7));
        assert_eq!(rx.recv().await, None);
        output.data(7, b"late output");
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn transport_disconnect_ends_shell_output() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut output = ShellOutput::default();
        output.register(7, tx);
        output.close(None);
        assert_eq!(rx.recv().await, None);
    }

    #[test]
    fn known_hosts_accepts_saved_openssh_key_and_reports_change() {
        use ssh_key::public::{Ed25519PublicKey, KeyData};
        let key = ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([1; 32])), "");
        let changed_key = ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([2; 32])), "");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        std::fs::write(
            &path,
            format!("[example.test]:2222 {}\n", key.to_openssh().unwrap()),
        )
        .unwrap();
        // 输出目的地按 pty 登记（ShellOutput::register），这里无需预建通道
        let handler = SshHandler {
            shell_output: Arc::new(Mutex::new(ShellOutput::default())),
            host: "example.test".into(),
            port: 2222,
            known_hosts_paths: vec![path],
            host_key_sink: None,
        };
        assert_eq!(handler.verify_known_hosts(&key), (true, None));
        let (matched, previous) = handler.verify_known_hosts(&changed_key);
        assert!(!matched);
        assert_eq!(previous, Some(key.fingerprint(HashAlg::Sha256).to_string()));
    }

    #[test]
    fn default_known_hosts_paths_exclude_cwd_relative_file() {
        for path in build_known_hosts_paths(false) {
            assert!(
                path.is_absolute(),
                "默认搜索路径必须全为绝对路径，cwd 相对路径不得混入：{}",
                path.display()
            );
        }
        assert!(
            build_known_hosts_paths(true)
                .iter()
                .any(|p| !p.is_absolute()),
            "显式开启（RSHELL_ALLOW_CWD_KNOWN_HOSTS）后相对路径才允许出现"
        );
    }

    /// R2-06 验收：端口 22 的裸 IPv6 主机（OpenSSH 与本应用写侧均写裸地址）
    /// 保存过的密钥必须被匹配命中，重连时不再触发 UI 决策弹框
    #[test]
    fn known_hosts_matches_bare_ipv6_host_on_default_port() {
        use ssh_key::public::{Ed25519PublicKey, KeyData};
        let key = ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([9; 32])), "");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        std::fs::write(&path, format!("::1 {}\n", key.to_openssh().unwrap())).unwrap();
        // 输出目的地按 pty 登记（ShellOutput::register），这里无需预建通道
        let handler = SshHandler {
            shell_output: Arc::new(Mutex::new(ShellOutput::default())),
            host: "::1".into(),
            port: 22,
            known_hosts_paths: vec![path],
            host_key_sink: None,
        };
        // pattern_matches 直接断言：裸 IPv6 命中、其他 IPv6 不命中、方括号写法不变
        assert!(
            handler.pattern_matches("::1"),
            "pattern \"::1\" 与 host \"::1\"（端口 22）必须匹配命中"
        );
        assert!(!handler.pattern_matches("fe80::1"));
        assert!(handler.pattern_matches("[::1]:22"));
        // 命中即信任：verify 返回 (true, None)，不再弹确认框
        assert_eq!(handler.verify_known_hosts(&key), (true, None));
    }

    #[test]
    fn cwd_preset_known_hosts_file_is_not_trusted_by_default() {
        use ssh_key::public::{Ed25519PublicKey, KeyData};
        let key = ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([7; 32])), "");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("known_hosts"),
            format!("[example.test]:2222 {}\n", key.to_openssh().unwrap()),
        )
        .unwrap();

        fn handler_with(paths: Vec<PathBuf>) -> SshHandler {
            // 输出目的地按 pty 登记（ShellOutput::register），这里无需预建通道
            SshHandler {
                shell_output: Arc::new(Mutex::new(ShellOutput::default())),
                host: "example.test".into(),
                port: 2222,
                known_hosts_paths: paths,
                host_key_sink: None,
            }
        }

        // 修改进程 cwd 影响其他依赖 cwd 的测试，串行化并保证恢复
        static CWD_LOCK: Mutex<()> = Mutex::new(());
        let _guard = CWD_LOCK.lock().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let (known_default, _) =
            handler_with(build_known_hosts_paths(false)).verify_known_hosts(&key);
        let (known_opt_in, _) =
            handler_with(build_known_hosts_paths(true)).verify_known_hosts(&key);
        let _ = std::env::set_current_dir(original_dir);

        assert!(
            !known_default,
            "cwd 里预置的 known_hosts 不应被默认搜索路径命中并直接信任"
        );
        assert!(
            known_opt_in,
            "显式开启（RSHELL_ALLOW_CWD_KNOWN_HOSTS）后相对路径才参与匹配"
        );
    }

    #[test]
    fn test_ssh_client_creation() {
        let config = SessionConfig {
            id: uuid::Uuid::new_v4(),
            name: "test".to_string(),
            folder_id: None,
            host: "127.0.0.1".to_string(),
            port: 22,
            protocol: rshell_api::types::Protocol::SSH,
            auth_method: AuthMethod::Password {
                username: "root".to_string(),
                has_password: true,
            },
            serial_config: None,
        };

        let client = SshClient::new(
            config,
            ResolvedAuthMethod::Password {
                username: "root".into(),
                password: "test".into(),
            },
        );
        assert!(client.handle.is_none());
        assert!(client.channel.is_none());
    }

    /// 回归：主 pty 必须以**会话 id** 注册。
    ///
    /// 前端首标签把 `session_id` 当 terminal_id 传下来（壳层输出路由、
    /// 核心层 send_data/resize 都用这个键）。若协议层给主 pty 生成随机
    /// uuid，`terminals.get(&session_id)` 就查不到句柄 —— 表现为
    /// **输出正常但键盘完全无反应**（输入被静默丢弃）。
    #[test]
    fn primary_terminal_is_addressed_by_session_id() {
        let session_id = Uuid::new_v4();
        let client = SshClient::new(
            SessionConfig {
                id: session_id,
                name: "test".to_string(),
                folder_id: None,
                host: "127.0.0.1".to_string(),
                port: 22,
                protocol: rshell_api::types::Protocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "root".to_string(),
                    has_password: true,
                },
                serial_config: None,
            },
            ResolvedAuthMethod::Password {
                username: "root".into(),
                password: "test".into(),
            },
        );
        assert_eq!(
            client.primary_terminal_id(),
            session_id,
            "主 pty 的 terminal_id 必须等于 session_id，否则首标签输入会被丢弃",
        );
    }

    /// 造一个只配好配置、没有任何连接的 SshClient（供纯本地的锁/生命周期测试用）。
    fn bare_client() -> SshClient {
        let session_id = Uuid::new_v4();
        SshClient::new(
            SessionConfig {
                id: session_id,
                name: "test".to_string(),
                folder_id: None,
                host: "127.0.0.1".to_string(),
                port: 22,
                protocol: rshell_api::types::Protocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "root".to_string(),
                    has_password: true,
                },
                serial_config: None,
            },
            ResolvedAuthMethod::Password {
                username: "root".into(),
                password: "test".into(),
            },
        )
    }

    /// 回归 R3-01：`disconnect_ssh` 不得因某个 pty actor 卡住而挂起。
    ///
    /// 这里用「sender 的接收端永不 poll」精确模拟 actor 卡在 `channel.data()`
    /// （对端不读 stdin、发送窗口打满）的状态：actor 任务还活着，所以
    /// `send(Close)` 会成功；但它永远不会回 select!，`reply` 也就永远收不到。
    ///
    /// 旧实现逐个 `await close_terminal(id)` → `received.await` 永久挂起 →
    /// 调用方（`SessionService::disconnect`）持有的 `SshClient` 写锁永不释放 →
    /// 整个会话卡死、只能重启应用。本测试断言断开**立即**完成。
    #[tokio::test]
    async fn disconnect_completes_even_when_a_pty_actor_never_replies() {
        let mut client = bare_client();
        let stuck_terminal = Uuid::new_v4();

        // 保留 `_never_polled`：它就是那个「卡住不再处理请求」的 actor。
        let (_tx, _never_polled) = mpsc::channel::<ShellRequest>(32);
        client.terminals.insert(
            stuck_terminal,
            TerminalHandle {
                sender: _tx,
                channel: 0,
            },
        );
        assert_eq!(client.terminal_count(), 1);

        let disconnected =
            tokio::time::timeout(std::time::Duration::from_secs(2), client.disconnect_ssh()).await;

        assert!(
            disconnected.is_ok(),
            "disconnect_ssh 在 pty actor 不应答时挂住了（R3-01 回归）"
        );
        assert!(disconnected.expect("checked above").is_ok());
        assert_eq!(client.terminal_count(), 0, "断开后不应残留任何 pty 条目");
    }

    /// 回归 R3-01（配套）：等待 pty actor 应答必须有上限。
    ///
    /// 直接测 `await_actor_reply` 并传毫秒级超时，这样「有界」这一性质能被
    /// 快速覆盖，而不必让测试套件为生产值（10 秒）买单。
    #[tokio::test]
    async fn awaiting_a_stuck_actor_reply_is_bounded_not_infinite() {
        // 构造一个「actor 卡住」的等价形态：sender 存在但接收端永不 poll，
        // 因此 reply 永远收不到。两者都刻意保活到断言结束。
        let (_tx, _never_polled) = mpsc::channel::<ShellRequest>(32);
        let (_reply, received) = oneshot::channel();

        let started = std::time::Instant::now();
        let result =
            await_actor_reply(received, std::time::Duration::from_millis(150), "input").await;
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "actor 不应答时必须返回错误，不能伪装成发送成功"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "等待应答应按超时返回，实际耗时 {elapsed:?}"
        );

        // 对照：应答 sender 被丢弃时（actor 任务已结束）应立刻返回
        // ConnectionClosed，而不是等满超时。
        let (reply2, received2) = oneshot::channel::<Result<(), ProtocolError>>();
        drop(reply2);
        let started = std::time::Instant::now();
        let err = await_actor_reply(received2, std::time::Duration::from_secs(30), "input")
            .await
            .expect_err("sender dropped must not look like success");
        assert!(
            matches!(err, ProtocolError::ConnectionClosed),
            "sender 被丢弃应报 ConnectionClosed，实际 {err:?}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "sender 已丢弃时应立即返回，不该等满超时"
        );
    }
}
