//! SSH 客户端实现
//!
//! 基于 russh 实现 SSH 连接、认证、数据收发和终端大小调整。

use std::path::PathBuf;
use std::sync::Arc;

use rshell_api::types::{AuthMethod, SessionConfig};
use ssh_key::HashAlg;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{Connection, ProtocolError};

/// SSH 客户端
pub struct SshClient {
    config: SessionConfig,
    /// 连接句柄
    handle: Option<russh::client::Handle<SshHandler>>,
    /// 当前会话通道
    channel: Option<russh::Channel<russh::client::Msg>>,
    /// 接收数据的通道
    data_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    /// 发送数据的通道（供 Handler 使用）
    data_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
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
/// `SshHandler::check_server_key` 是**同步** trait 方法，不能直接 `.await`。
/// 但 UI 端需要异步响应决策——所以 `SshClient` 接受一个 `Arc<dyn HostKeyDecisionSink>`：
/// 未知 key 时调用 `register_decision` 注册一个等待项、拿到一个 oneshot::Receiver，
/// 再 `publish_request` 让 UI 看到、最后在 `check_server_key` 的 async body 内 `.await` 等。
///
/// 协议层（`rshell-protocol`）不依赖 `rshell-core`,所以这里用 trait object 解耦;
/// `rshell-core::security::host_key_decision::HostKeyDecisionRegistry` 是 trait 的
/// 标准实现。
pub trait HostKeyDecisionSink: Send + Sync {
    /// 注册一个待决策项
    fn register_decision(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>);
    /// 向 UI 端发布"请决策"通知
    fn publish_request(&self, info: HostKeyDecisionRequest);
}

/// 待决策的主机密钥信息（用于发布给 UI 端）
#[derive(Debug, Clone)]
pub struct HostKeyDecisionRequest {
    pub decision_id: Uuid,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub public_key_blob: String,
}

/// SSH Handler 实现
///
/// 负责接收服务端数据并通过通道转发给上层；同时验证服务器主机密钥。
pub(crate) struct SshHandler {
    data_tx: mpsc::UnboundedSender<Vec<u8>>,
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
    fn verify_known_hosts(&self, server_key: &ssh_key::PublicKey) -> bool {
        let expected_fp = server_key.fingerprint(HashAlg::Sha256).to_string();

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

            if self.scan_known_hosts(&content, server_key, &expected_fp) {
                debug!("Host key matched entry in {}", path.display());
                return true;
            }
        }

        false
    }

    /// 扫描 known_hosts 内容，匹配 host 模式 + 密钥指纹
    fn scan_known_hosts(&self, content: &str, server_key: &ssh_key::PublicKey, expected_fp: &str) -> bool {
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
            let host_matches = host_field.split(',').any(|pattern| {
                self.pattern_matches(pattern)
            });
            if !host_matches {
                continue;
            }

            // 解析存储的公钥并比对指纹
            // OpenSSH 格式：key_blob 是 base64 编码的 wire-format 公钥
            let stored_key = ssh_key::PublicKey::from_openssh(key_blob)
                .or_else(|_| ssh_key::PublicKey::from_bytes(key_blob.as_bytes()));
            let stored_key = match stored_key {
                Ok(k) => k,
                Err(e) => {
                    debug!("Failed to parse known_hosts key (type={}): {}", keytype, e);
                    continue;
                }
            };

            if stored_key.fingerprint(HashAlg::Sha256).to_string() == expected_fp {
                // 额外确认 keytype 一致（防 base64 巧合匹配）
                if stored_key.algorithm().as_str() == server_key.algorithm().as_str() {
                    return true;
                }
            }
        }

        false
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
            None => true, // 未指定端口 → 默认 22，与 SSH 默认一致
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

        if self.verify_known_hosts(server_public_key) {
            return Ok(true);
        }

        // ===== 未在 known_hosts 中找到匹配条目 → 等待 UI 决策 =====
        // `check_server_key` 是 async trait 方法,rx 是 tokio oneshot,直接 .await;
        // 30s 超时避免 UI 无响应时永久挂住 russh 连接。
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
                self.host, self.port, fp
            ));
        };

        // 1. 注册等待项,拿到 decision_id + oneshot receiver
        let (decision_id, rx) = sink.register_decision();

        // 2. 通知 UI 端"请决策"
        let key_blob = server_public_key.to_string();
        sink.publish_request(HostKeyDecisionRequest {
            decision_id,
            host: self.host.clone(),
            port: self.port,
            key_type: format!("{:?}", server_public_key.algorithm()),
            fingerprint: fp.clone(),
            public_key_blob: key_blob,
        });

        // 3. 等待 UI 端通过 AppCommand::DecideHostKey 唤醒。
        // `check_server_key` 已经是 async,rx 是 tokio oneshot,直接 .await。
        // 30s 超时,避免用户在 UI 上无响应时永久挂住 russh 连接。
        let user_decided = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            rx,
        )
        .await;
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
                Err(anyhow::anyhow!("User rejected host key for {}:{}", self.host, self.port))
            }
            Ok(Err(_)) => {
                warn!(
                    host = %self.host,
                    port = self.port,
                    fingerprint = %fp,
                    "Host key decision channel closed — rejecting"
                );
                Err(anyhow::anyhow!(
                    "Host key decision channel closed for {}:{}",
                    self.host, self.port
                ))
            }
            Err(_) => {
                warn!(
                    host = %self.host,
                    port = self.port,
                    fingerprint = %fp,
                    "Host key decision timed out after 30s — rejecting"
                );
                Err(anyhow::anyhow!(
                    "Host key decision timed out for {}:{}",
                    self.host, self.port
                ))
            }
        }
    }

    async fn data(
        &mut self,
        _channel: russh::ChannelId,
        data: &[u8],
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        // 将收到的数据通过通道转发给上层
        let _ = self.data_tx.send(data.to_vec());
        Ok(())
    }

    async fn disconnected(
        &mut self,
        reason: russh::client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        info!("SSH disconnected: {:?}", reason);
        Ok(())
    }
}

/// 从 AuthMethod 提取用户名
fn get_username(auth: &AuthMethod) -> &str {
    match auth {
        AuthMethod::Password { username, .. } => username,
        AuthMethod::PublicKey { username, .. } => username,
        AuthMethod::KeyboardInteractive { username, .. } => username,
    }
}

impl SshClient {
    /// 创建新的 SSH 客户端
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            handle: None,
            channel: None,
            data_rx: None,
            data_tx: None,
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
            data_tx: data_tx.clone(),
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
        self.data_tx = Some(data_tx);
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

        let username = get_username(&self.config.auth_method);

        match &self.config.auth_method {
            AuthMethod::Password { password, .. } => {
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
            AuthMethod::PublicKey {
                key_path, passphrase, ..
            } => {
                // 加载私钥
                let key = russh_keys::load_secret_key(key_path, passphrase.as_deref()).map_err(
                    |e| ProtocolError::AuthFailed(format!("Failed to load key: {}", e)),
                )?;

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
            AuthMethod::KeyboardInteractive { password, .. } => {
                // 键盘交互认证
                let response = handle
                    .authenticate_keyboard_interactive_start(username, None::<String>)
                    .await
                    .map_err(|e| ProtocolError::AuthFailed(e.to_string()))?;

                match response {
                    russh::client::KeyboardInteractiveAuthResponse::Success => {}
                    russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                        prompts,
                        ..
                    } => {
                        // 使用配置中的密码（如果有），否则用空字符串
                        let pwd = password.as_deref().unwrap_or("");
                        let responses: Vec<String> = prompts.iter().map(|_| pwd.to_string()).collect();

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
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

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

        self.channel = Some(channel);

        Ok(())
    }

    /// 断开连接
    pub async fn disconnect_ssh(&mut self) -> Result<(), ProtocolError> {
        if let Some(channel) = self.channel.take() {
            let _ = channel.close().await;
        }

        if let Some(handle) = self.handle.take() {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "User disconnect", "en")
                .await;
        }

        self.data_tx = None;
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

        channel
            .data(std::io::Cursor::new(data.to_vec()))
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Send data failed: {}", e)))?;

        Ok(())
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

    /// 调整终端大小
    pub async fn resize_terminal(&self, cols: u32, rows: u32) -> Result<(), ProtocolError> {
        let channel = self
            .channel
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?;

        channel
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Window change failed: {}", e)))?;

        Ok(())
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

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("SFTP channel open failed: {}", e)))?;

        // 请求 sftp 子系统
        channel
            .request_subsystem(false, "sftp")
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("SFTP subsystem request failed: {}", e)))?;

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
            .map_err(|e| ProtocolError::ConnectionFailed(format!("direct-tcpip open failed: {}", e)))
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
                password: "test".to_string(),
            },
        };

        let client = SshClient::new(config);
        assert!(client.handle.is_none());
        assert!(client.channel.is_none());
    }
}
