//! SSH 客户端实现
//!
//! 基于 russh 实现 SSH 连接、认证、数据收发和终端大小调整。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct ShellOutput {
    channel: Option<u32>,
    sender: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl ShellOutput {
    fn data(&self, channel: u32, data: &[u8]) {
        if self.channel == Some(channel) {
            if let Some(sender) = &self.sender {
                let _ = sender.send(data.to_vec());
            }
        }
    }

    fn close(&mut self, channel: Option<u32>) {
        if channel.is_none() || channel == self.channel {
            self.sender.take();
            self.channel = None;
        }
    }
}

use rshell_api::types::SessionConfig;
use ssh_key::HashAlg;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{Connection, ProtocolError};

/// SSH 客户端
pub struct SshClient {
    config: SessionConfig,
    auth: ResolvedAuthMethod,
    /// 连接句柄
    handle: Option<russh::client::Handle<SshHandler>>,
    /// 当前会话通道
    channel: Option<mpsc::Sender<ShellRequest>>,
    /// 接收数据的通道
    data_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    /// 发送数据的通道（供 Handler 使用）
    shell_output: Arc<Mutex<ShellOutput>>,
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
    /// 已知的 known_hosts 文件路径（依次尝试：~/.ssh/known_hosts、用户配置）
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
            // [1.2.3.4]:2222 或 [hostname]:2222
            let host = &pattern[..idx + 1]; // 含 ']'
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let port = &pattern[idx + 2..];
            (host.to_string(), Some(port.to_string()))
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

#[async_trait::async_trait]
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

impl SshClient {
    /// 创建新的 SSH 客户端
    pub fn new(config: SessionConfig, auth: ResolvedAuthMethod) -> Self {
        Self {
            config,
            auth,
            handle: None,
            channel: None,
            data_rx: None,
            shell_output: Arc::new(Mutex::new(ShellOutput::default())),
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
        info!(
            "Connecting to SSH server {}:{}",
            self.config.host, self.config.port
        );

        // 创建数据通道
        let (data_tx, data_rx) = mpsc::unbounded_channel();
        self.shell_output = Arc::new(Mutex::new(ShellOutput {
            channel: None,
            sender: Some(data_tx),
        }));

        // 创建 SSH 配置
        let ssh_config = Arc::new(russh::client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        });

        // 构建 known_hosts 搜索路径（按优先级）
        let mut known_hosts_paths: Vec<PathBuf> = Vec::new();
        if let Some(home) = dirs::home_dir() {
            known_hosts_paths.push(home.join(".ssh").join("known_hosts"));
        }
        // 也尝试 rshell 自有 known_hosts 文件（由 HostKeyManager 维护）
        if let Some(mut data_dir) = dirs::data_local_dir() {
            data_dir.push("rshell");
            data_dir.push("known_hosts");
            known_hosts_paths.push(data_dir);
        }
        // 最后尝试当前目录（开发环境）
        known_hosts_paths.push(PathBuf::from("known_hosts"));

        // 创建 Handler（带 host_key_sink）
        let handler = SshHandler {
            shell_output: self.shell_output.clone(),
            host: self.config.host.clone(),
            port: self.config.port,
            known_hosts_paths,
            host_key_sink,
        };

        // 连接到服务器
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let handle = russh::client::connect(ssh_config, &addr, handler)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

        self.handle = Some(handle);
        self.data_rx = Some(data_rx);

        info!("SSH TCP connection established");

        // 进行认证
        self.authenticate().await?;

        info!("SSH authentication successful");

        // 打开会话通道
        self.open_session().await?;

        info!("SSH session channel opened");

        Ok(())
    }

    /// 执行 SSH 认证
    async fn authenticate(&mut self) -> Result<(), ProtocolError> {
        let handle = self
            .handle
            .as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        let username = get_username(&self.auth);

        match &self.auth {
            ResolvedAuthMethod::Password { password, .. } => {
                let success = handle
                    .authenticate_password(username, password)
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                if !success {
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
                // 加载私钥
                let key = russh_keys::load_secret_key(key_path, passphrase.as_deref())
                    .map_err(|e| ProtocolError::AuthFailed(format!("Failed to load key: {}", e)))?;

                let key = Arc::new(key);
                let success = handle
                    .authenticate_publickey(username, key)
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                if !success {
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
                    russh::client::KeyboardInteractiveAuthResponse::Failure => {
                        return Err(ProtocolError::AuthFailed(
                            "Keyboard-interactive authentication failed".to_string(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// 打开会话通道并请求 PTY
    async fn open_session(&mut self) -> Result<(), ProtocolError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        // 打开会话通道
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

        self.shell_output.lock().unwrap().channel = Some(channel.id().into());

        // 请求 PTY（默认 80x24）
        channel
            .request_pty(
                false,            // want_reply
                "xterm-256color", // term
                80,               // col_width
                24,               // row_height
                0,                // pix_width
                0,                // pix_height
                &[],              // terminal_modes
            )
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("PTY request failed: {}", e)))?;

        // 请求 shell
        channel
            .request_shell(false)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Shell request failed: {}", e)))?;

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
            output.lock().unwrap().close(None);
        });
        self.channel = Some(tx);

        Ok(())
    }

    /// 断开连接
    pub async fn disconnect_ssh(&mut self) -> Result<(), ProtocolError> {
        if let Some(channel) = self.channel.take() {
            let (reply, received) = oneshot::channel();
            if channel.send(ShellRequest::Close(reply)).await.is_ok() {
                let _ = received.await;
            }
        }

        if let Some(handle) = self.handle.take() {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "User disconnect", "en")
                .await;
        }

        self.shell_output.lock().unwrap().close(None);
        self.data_rx = None;

        info!("SSH disconnected");
        Ok(())
    }

    /// 发送数据到远程 shell
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
    pub async fn open_direct_tcpip(
        &self,
        host: &str,
        port: u32,
    ) -> Result<russh::Channel<russh::client::Msg>, ProtocolError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;
        handle
            .channel_open_direct_tcpip(host, port, "127.0.0.1", 0)
            .await
            .map_err(|e| {
                ProtocolError::ConnectionFailed(format!("direct-tcpip open failed: {}", e))
            })
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
        // 从通道读取数据到缓冲区
        match self.recv_data().await? {
            Some(data) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
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
            matches!(client.auth, ResolvedAuthMethod::Password { password, .. } if password == "sample-secret-password")
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
            matches!(client.auth, ResolvedAuthMethod::PublicKey { passphrase: Some(value), .. } if value == "sample-secret-passphrase")
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
    }

    #[async_trait::async_trait]
    impl russh::server::Handler for OutputTestServer {
        type Error = russh::Error;
        async fn auth_password(
            &mut self,
            _: &str,
            _: &str,
        ) -> Result<russh::server::Auth, Self::Error> {
            Ok(russh::server::Auth::Accept)
        }
        async fn channel_open_session(
            &mut self,
            _: russh::Channel<russh::server::Msg>,
            _: &mut russh::server::Session,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
        async fn shell_request(
            &mut self,
            channel: russh::ChannelId,
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            self.shell = Some(channel);
            session.data(channel, russh::CryptoVec::from_slice(b"shell output"))?;
            Ok(())
        }
        async fn subsystem_request(
            &mut self,
            channel: russh::ChannelId,
            _: &str,
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.data(
                channel,
                russh::CryptoVec::from_slice(b"binary sftp payload"),
            )?;
            session.eof(channel)?;
            session.close(channel)?;
            session.eof(self.shell.unwrap())?;
            Ok(())
        }
        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.data(channel, russh::CryptoVec::from_slice(data))?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn ssh_loopback_isolates_subsystem_payload_and_closes_shell_receiver() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let key = ssh_key::PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )
        .unwrap();
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

    #[tokio::test]
    async fn shell_output_excludes_other_channels_and_ends_on_shell_eof() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut output = ShellOutput {
            channel: Some(7),
            sender: Some(tx),
        };
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
        let mut output = ShellOutput {
            channel: Some(7),
            sender: Some(tx),
        };
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
        let (data_tx, _) = mpsc::unbounded_channel();
        let handler = SshHandler {
            shell_output: Arc::new(Mutex::new(ShellOutput {
                channel: None,
                sender: Some(data_tx),
            })),
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
}
