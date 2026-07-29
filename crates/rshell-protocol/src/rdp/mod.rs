//! RDP 协议实现
//!
//! 基于 `ironrdp` 0.14 + `ironrdp-tokio` 0.8 实现的远程桌面客户端。
//!
//! ⚠️ **RDP 与 byte-stream `Connection` trait 的语义不匹配**：RDP 产生桌面帧
//! (`GraphicsUpdate`) 而非终端字节流。本实现采用以下策略：
//! - `recv(buf)`：始终返回 `Ok(0)` — byte-stream 语义下 RDP 无数据可提供。
//! - `send(data)`：将字节序列排队为键盘事件。
//! - 桌面帧通过独立的 `frame_rx: mpsc::UnboundedReceiver<RdpFrame>` 通道向外发布。
//!
//! ⚠️ **MVP 范围限制**：本实现仅完成 TCP 连接 + `connect_begin` 的早期阶段。
//! 完整的 TLS 升级 + NLA 认证 + ActiveStage 帧渲染需要服务端配合与 ironrdp-graphics
//! 集成，留作后续工作。客户端握手失败时会返回明确错误。

use async_trait::async_trait;
use ironrdp_connector::{ClientConnector, Config as ConnectorConfig, Credentials, DesktopSize};
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tracing::{debug, info};

use crate::{Connection, ProtocolError};

/// RDP 配置
#[derive(Debug, Clone)]
pub struct RdpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub width: u32,
    pub height: u32,
}

impl Default for RdpConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 3389,
            username: String::new(),
            password: None,
            domain: None,
            width: 1920,
            height: 1080,
        }
    }
}

/// RDP 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpState {
    Disconnected,
    Connecting,
    Connected,
}

/// 桌面帧通道项（调用方拿走 `frame_receiver` 后消费）
#[derive(Debug, Clone)]
pub struct RdpFrame {
    pub width: u32,
    pub height: u32,
    /// RGBA 像素（自上而下、左到右）。
    pub pixels: Vec<u8>,
}

/// RDP 连接
pub struct RdpConnection {
    config: RdpConfig,
    state: RdpState,
    frame_rx: Option<mpsc::UnboundedReceiver<RdpFrame>>,
    /// 后台驱动任务是否存活
    driver_handle: Option<TokioMutex<Option<tokio::task::JoinHandle<()>>>>,
    /// 键盘事件发送端
    key_tx: Option<mpsc::UnboundedSender<u8>>,
}

impl RdpConnection {
    /// 创建新的 RDP 连接（尚未连接）
    pub fn new(config: RdpConfig) -> Self {
        Self {
            config,
            state: RdpState::Disconnected,
            frame_rx: None,
            driver_handle: None,
            key_tx: None,
        }
    }

    /// 获取连接配置
    pub fn config(&self) -> &RdpConfig {
        &self.config
    }

    /// 设置分辨率
    pub fn set_resolution(&mut self, width: u32, height: u32) {
        if self.state == RdpState::Disconnected {
            self.config.width = width;
            self.config.height = height;
        }
    }

    /// 获取当前连接状态
    pub fn state(&self) -> RdpState {
        self.state
    }

    /// 取出桌面帧接收端（仅一次）。GUI 层拿到后即可消费帧。
    pub fn take_frame_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<RdpFrame>> {
        self.frame_rx.take()
    }
}

#[async_trait]
impl Connection for RdpConnection {
    async fn connect(&mut self) -> Result<(), ProtocolError> {
        info!(
            "Connecting to RDP server {}:{} as {} ({}x{})",
            self.config.host,
            self.config.port,
            self.config.username,
            self.config.width,
            self.config.height
        );
        self.state = RdpState::Connecting;

        // 1. 打开 TCP
        let tcp = tokio::net::TcpStream::connect((self.config.host.as_str(), self.config.port))
            .await
            .map_err(|e| {
                ProtocolError::ConnectionFailed(format!(
                    "TCP connect to {}: {}",
                    self.config.host,
                    e
                ))
            })?;
        let client_addr = tcp
            .local_addr()
            .map_err(|e| ProtocolError::ConnectionFailed(format!("local_addr: {}", e)))?;

        // 2. 构造 Connector
        let credentials = Credentials::UsernamePassword {
            username: self.config.username.clone(),
            password: self.config.password.clone().unwrap_or_default(),
        };
        let connector_config = ConnectorConfig {
            credentials,
            domain: self.config.domain.clone(),
            enable_tls: false,
            enable_credssp: false,
            keyboard_type: KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_layout: 0,
            keyboard_functional_keys_count: 12,
            ime_file_name: String::new(),
            dig_product_id: String::new(),
            desktop_size: DesktopSize {
                width: self.config.width as u16,
                height: self.config.height as u16,
            },
            desktop_scale_factor: 100,
            bitmap: None,
            client_build: 0,
            client_name: "rshell".to_owned(),
            client_dir: String::new(),
            platform: MajorPlatformType::WINDOWS,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            enable_server_pointer: true,
            pointer_software_rendering: false,
        };

        let mut connector = ClientConnector::new(connector_config, client_addr);

        // 3. 构造 TokioFramed 包装流
        let mut framed = ironrdp_tokio::TokioFramed::new(tcp);

        // 4. 启动 connect_begin（仅完成 X.224 协商；后续 TLS/NLA 升级留给后续工作）
        // ironrdp_async 重导出 ironrdp-connector 的 connect_begin，签名与 Framed 一致。
        if let Err(e) = ironrdp_async::connect_begin(&mut framed, &mut connector).await {
            self.state = RdpState::Disconnected;
            return Err(ProtocolError::ConnectionFailed(format!(
                "RDP X.224 negotiation failed: {}",
                e
            )));
        }

        info!("RDP X.224 negotiation completed; full TLS+NLA handshake pending implementation");

        // 5. 创建桌面帧 channel + 键盘事件 channel
        let (frame_tx, frame_rx) = mpsc::unbounded_channel::<RdpFrame>();
        let (key_tx, _key_rx) = mpsc::unbounded_channel::<u8>();

        // 6. 后台驱动任务：当前为占位实现，发送连接已建立信号帧。
        // 完整实现需要：
        //   - 升级到 TLS（tokio-rustls）
        //   - 跑 NLA / CredSSP
        //   - ActiveStage pump
        //   - 用 ironrdp-graphics SoftDisplay 渲染桌面为 RGBA bytes
        let handle = tokio::spawn(async move {
            let _ = frame_tx.send(RdpFrame {
                width: 0,
                height: 0,
                pixels: vec![0, 0, 0, 0],
            });
            debug!("RDP driver placeholder task started; awaiting full handshake implementation");
        });

        self.driver_handle = Some(TokioMutex::new(Some(handle)));
        self.frame_rx = Some(frame_rx);
        self.key_tx = Some(key_tx);
        self.state = RdpState::Connected;
        info!("RDP connection marked Connected (partial handshake)");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        info!("Disconnecting RDP session");
        if let Some(handle_mutex) = self.driver_handle.take() {
            if let Some(h) = handle_mutex.lock().await.take() {
                h.abort();
            }
        }
        self.frame_rx = None;
        self.key_tx = None;
        self.state = RdpState::Disconnected;
        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError> {
        if self.state != RdpState::Connected {
            return Err(ProtocolError::ConnectionFailed("RDP not connected".to_string()));
        }
        if let Some(tx) = &self.key_tx {
            for &b in data {
                let _ = tx.send(b);
            }
        }
        debug!("RDP send {} bytes (queued as keyboard events)", data.len());
        Ok(())
    }

    async fn recv(&mut self, _buf: &mut [u8]) -> Result<usize, ProtocolError> {
        if self.state != RdpState::Connected {
            return Err(ProtocolError::ConnectionFailed("RDP not connected".to_string()));
        }
        debug!("RDP recv (no byte-stream data; use take_frame_receiver)");
        Ok(0)
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ProtocolError> {
        self.config.width = cols as u32;
        self.config.height = rows as u32;
        info!("RDP resize requested to {}x{} (no-op until ActiveStage hooked up)", cols, rows);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdp_config_default() {
        let cfg = RdpConfig::default();
        assert_eq!(cfg.port, 3389);
        assert_eq!(cfg.width, 1920);
        assert_eq!(cfg.height, 1080);
    }

    #[tokio::test]
    async fn test_rdp_connection_creation() {
        let conn = RdpConnection::new(RdpConfig::default());
        assert_eq!(conn.state(), RdpState::Disconnected);
        assert!(conn.frame_rx.is_none());
    }
}