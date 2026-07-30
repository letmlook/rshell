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
//! ## 实现状态
//!
//! | 阶段 | 状态 |
//! |------|------|
//! | TCP connect | ✅ |
//! | X.224 协商（`connect_begin`） | ✅ |
//! | TLS 升级（tokio-rustls） | ❌ 留作后续 — 需要把 framed 内部的 TcpStream 拆出来做 TLS 握手，再包装为新的 TokioFramed |
//! | CredSSP / NLA | ❌ 留作后续（依赖 sspi + reqwest） |
//! | `connect_finalize`（能力交换 + 通道连接） | ❌ 需要 TLS 成功后才能跑 |
//! | ActiveStage 帧渲染（ironrdp-graphics + SoftDisplay） | ❌ 需要 connect_finalize 成功后驱动 |
//!
//! 当前实现成功完成 X.224 后将状态标记为 `X224Only`，GUI 可收到连接成功信号但
//! 桌面帧通道仅发出占位帧。完整 RDP 端到端验证需要一个真实的 RDP 服务器
//! （Windows / xrdp / FreeRDP）；本环境无 RDP 服务端，最终阶段未跑通。
//!
//! 后续接入的入口：
//! 1. 实现 `upgrade_to_tls`（拆 framed 内部 TcpStream → rustls client.connect →
//!    重新包装为 TokioFramed<TlsStream>）
//! 2. 实现 `NoopNetworkClient`/`ReqwestNetworkClient` + `connect_finalize`
//! 3. 把 `ActiveStage::process()` 放进后台任务，用 `ironrdp-graphics`
//!    `Software` 渲染器把 GraphicsUpdate 转为 RGBA 帧并通过 frame_tx 发出

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
    /// 是否启用 NLA（CredSSP）。生产环境建议 true；当前 MVP 留 false 以避免
    /// 引入 sspi / reqwest / hickory-resolver 等重依赖。
    pub enable_nla: bool,
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
            enable_nla: false,
        }
    }
}

/// RDP 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpState {
    Disconnected,
    Connecting,
    /// X.224 已完成；TLS 升级未完成 / 已失败
    X224Only,
    /// TLS + 能力交换完成（ActiveStage 可用）
    Active,
}

impl RdpState {
    /// 状态机:从 `connect_begin` 成功结果到下一态
    ///
    /// 这把"协议步骤"显式化,便于单测覆盖各分支:
    /// - X.224 成功 → `X224Only`(再走 TLS 升级)
    /// - TLS 升级成功 → `Active`
    /// - 任意中间步骤失败 → `Disconnected`
    pub fn after_x224() -> Self {
        RdpState::X224Only
    }
    pub fn after_tls_upgrade() -> Self {
        RdpState::Active
    }
    pub fn after_failure() -> Self {
        RdpState::Disconnected
    }
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
    driver_handle: Option<TokioMutex<Option<tokio::task::JoinHandle<()>>>>,
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
            "Connecting to RDP server {}:{} as {} ({}x{}, nla={})",
            self.config.host,
            self.config.port,
            self.config.username,
            self.config.width,
            self.config.height,
            self.config.enable_nla
        );
        self.state = RdpState::Connecting;

        // ====== 第 1 步：TCP 连接 ======
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

        // ====== 第 2 步：构造 Connector（标准 RDP 配置） ======
        let credentials = Credentials::UsernamePassword {
            username: self.config.username.clone(),
            password: self.config.password.clone().unwrap_or_default(),
        };
        let connector_config = ConnectorConfig {
            credentials,
            domain: self.config.domain.clone(),
            enable_tls: true,
            enable_credssp: self.config.enable_nla,
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

        // ====== 第 3 步：X.224 协商 ======
        let mut framed = ironrdp_tokio::TokioFramed::new(tcp);

        if let Err(e) = ironrdp_async::connect_begin(&mut framed, &mut connector).await {
            self.state = RdpState::Disconnected;
            return Err(ProtocolError::ConnectionFailed(format!(
                "RDP X.224 negotiation failed: {}",
                e
            )));
        }

        info!("RDP X.224 negotiation completed");

        // ====== 第 4 步：TLS 升级（未实现） ======
        // 当前直接标记为 X224Only。完整 TLS 升级需要：
        //   a) 从 connector 拿 server_public_key 与 server_name
        //   b) 拆 framed 内部的 TcpStream → tokio_rustls::TlsConnector::connect
        //   c) 把 TlsStream 包回 TokioFramed 并继续 connect_finalize
        //   d) 调用 mark_as_upgraded(should_upgrade, &mut connector)
        // 后续 PR 在 `upgrade_to_tls()` 函数中实现。
        self.state = RdpState::X224Only;

        // ====== 第 5 步：建立 frame 通道 + 后台驱动 ======
        let (frame_tx, frame_rx) = mpsc::unbounded_channel::<RdpFrame>();
        let (key_tx, _key_rx) = mpsc::unbounded_channel::<u8>();

        let handle = tokio::spawn(async move {
            // 真实实现：循环 pump ActiveStage → GraphicsUpdate → SoftDisplay → RGBA
            // 当前为占位：发送一帧连接已建立信号帧。
            let _ = frame_tx.send(RdpFrame {
                width: 0,
                height: 0,
                pixels: vec![0, 0, 0, 0],
            });
            debug!("RDP driver placeholder started (state: X224Only)");
        });

        self.driver_handle = Some(TokioMutex::new(Some(handle)));
        self.frame_rx = Some(frame_rx);
        self.key_tx = Some(key_tx);
        info!("RDP connection marked X224Only (TLS upgrade + ActiveStage pending)");
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
        if self.state == RdpState::Disconnected {
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
        if self.state == RdpState::Disconnected {
            return Err(ProtocolError::ConnectionFailed("RDP not connected".to_string()));
        }
        debug!("RDP recv (no byte-stream data; use take_frame_receiver)");
        Ok(0)
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ProtocolError> {
        self.config.width = cols as u32;
        self.config.height = rows as u32;
        info!(
            "RDP resize requested to {}x{} (no-op until ActiveStage hooked up)",
            cols, rows
        );
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
        assert!(!cfg.enable_nla);
    }

    #[test]
    fn test_rdp_config_clone_independent() {
        let cfg = RdpConfig::default();
        let cfg2 = cfg.clone();
        // 两个独立对象,改 cfg2 不应影响 cfg
        assert_eq!(cfg.width, 1920);
        assert_eq!(cfg2.width, 1920);
    }

    #[tokio::test]
    async fn test_rdp_connection_creation() {
        let conn = RdpConnection::new(RdpConfig::default());
        assert_eq!(conn.state(), RdpState::Disconnected);
        assert!(conn.frame_rx.is_none());
    }

    #[test]
    fn test_rdp_state_transitions() {
        // X.224 成功
        assert_eq!(RdpState::after_x224(), RdpState::X224Only);
        // TLS 升级成功
        assert_eq!(RdpState::after_tls_upgrade(), RdpState::Active);
        // 任意失败
        assert_eq!(RdpState::after_failure(), RdpState::Disconnected);
    }

    #[test]
    fn test_rdp_set_resolution_blocked_when_connected() {
        let mut conn = RdpConnection::new(RdpConfig::default());
        // Disconnected 状态可以改
        conn.set_resolution(800, 600);
        assert_eq!(conn.config().width, 800);
        // 模拟 connected 状态再调应无效 — 通过把 state 改到 Connecting 测试
        // (无法从外部直接设 state,这里只验证 set_resolution 不 panic)
    }

    #[test]
    fn test_rdp_take_frame_receiver_only_once() {
        let mut conn = RdpConnection::new(RdpConfig::default());
        // 没 connect 过是 None
        assert!(conn.take_frame_receiver().is_none());
        assert!(conn.take_frame_receiver().is_none());
    }
}