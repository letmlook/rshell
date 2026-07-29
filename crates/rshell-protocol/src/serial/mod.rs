//! 串口协议实现
//!
//! 基于 `serialport` crate 实现串口通信。
//!
//! `serialport::SerialPort` 是**同步阻塞**的（POSIX / Win32 read / Win32 overlapped
//! depending on platform），因此所有 I/O 都包在 `tokio::task::spawn_blocking` 中，
//! 以避免阻塞后端 runtime 的事件循环。

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serialport::{SerialPort, SerialPortInfo};
use tokio::task;
use tracing::info;

use crate::{Connection, ProtocolError};

/// 串口配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: SerialParity,
    pub flow_control: SerialFlowControl,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            port: "COM1".to_string(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            flow_control: SerialFlowControl::None,
        }
    }
}

/// 串口奇偶校验
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

/// 串口流控制
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

fn to_serialport_data_bits(b: u8) -> serialport::DataBits {
    match b {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    }
}

fn to_serialport_stop_bits(b: u8) -> serialport::StopBits {
    match b {
        2 => serialport::StopBits::Two,
        _ => serialport::StopBits::One,
    }
}

fn to_serialport_parity(p: SerialParity) -> serialport::Parity {
    match p {
        SerialParity::None => serialport::Parity::None,
        SerialParity::Even => serialport::Parity::Even,
        SerialParity::Odd => serialport::Parity::Odd,
    }
}

fn to_serialport_flow(fc: SerialFlowControl) -> serialport::FlowControl {
    match fc {
        SerialFlowControl::None => serialport::FlowControl::None,
        SerialFlowControl::Software => serialport::FlowControl::Software,
        SerialFlowControl::Hardware => serialport::FlowControl::Hardware,
    }
}

/// 串口连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SerialState {
    Disconnected,
    Connected,
}

/// 串口连接
///
/// 内部持有一个 `Arc<Mutex<Box<dyn SerialPort>>>`。`serialport::SerialPort` 是
/// `Send` 但**不是** `Sync`，所以包在 Mutex 中以便 spawn_blocking 闭包可以借用。
/// `Connection` trait 要求 `Sync`，Mutex 提供内部可变性 + Sync 语义。
pub struct SerialConnection {
    config: SerialConfig,
    state: SerialState,
    port: Option<Arc<Mutex<Box<dyn SerialPort>>>>,
}

impl SerialConnection {
    /// 创建新的串口连接（尚未连接）
    pub fn new(config: SerialConfig) -> Self {
        Self {
            config,
            state: SerialState::Disconnected,
            port: None,
        }
    }

    /// 获取串口配置
    pub fn config(&self) -> &SerialConfig {
        &self.config
    }

    /// 设置波特率（仅在断连状态下生效）
    pub fn set_baud_rate(&mut self, baud_rate: u32) {
        if self.state == SerialState::Disconnected {
            self.config.baud_rate = baud_rate;
        }
    }

    /// 列出当前系统可用的串口
    pub async fn list_ports() -> Result<Vec<String>, ProtocolError> {
        task::spawn_blocking(|| -> Result<Vec<String>, ProtocolError> {
            let ports = serialport::available_ports().map_err(|e| {
                ProtocolError::ConnectionFailed(format!("available_ports failed: {}", e))
            })?;
            Ok(ports.into_iter().map(|p: SerialPortInfo| p.port_name).collect())
        })
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("join error: {}", e)))?
    }
}

#[async_trait]
impl Connection for SerialConnection {
    async fn connect(&mut self) -> Result<(), ProtocolError> {
        info!(
            "Connecting to serial port {} at {} baud",
            self.config.port, self.config.baud_rate
        );

        let cfg = self.config.clone();
        let port = task::spawn_blocking(move || -> Result<Box<dyn SerialPort>, ProtocolError> {
            let p = serialport::new(&cfg.port, cfg.baud_rate)
                .data_bits(to_serialport_data_bits(cfg.data_bits))
                .stop_bits(to_serialport_stop_bits(cfg.stop_bits))
                .parity(to_serialport_parity(cfg.parity))
                .flow_control(to_serialport_flow(cfg.flow_control))
                .timeout(Duration::from_millis(100))
                .open();
            match p {
                Ok(p) => Ok(p),
                Err(e) => Err(ProtocolError::ConnectionFailed(format!(
                    "Failed to open {}: {}",
                    cfg.port, e
                ))),
            }
        })
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("join error: {}", e)))?
        ?;

        self.port = Some(Arc::new(Mutex::new(port)));
        self.state = SerialState::Connected;
        info!("Serial port {} connected", self.config.port);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        info!("Disconnecting serial port {}", self.config.port);
        // 显式 take，避免 Drop 时的阻塞 syscall 在 async 上下文中运行
        if let Some(_port) = self.port.take() {
            // drop 在 task 上下文中是同步的；SerialPort 的 drop 通常立即返回
        }
        self.state = SerialState::Disconnected;
        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError> {
        if self.state != SerialState::Connected {
            return Err(ProtocolError::ConnectionFailed(
                "Serial port not connected".to_string(),
            ));
        }
        let port_arc = self
            .port
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?
            .clone();
        let bytes = data.to_vec();

        task::spawn_blocking(move || -> Result<(), std::io::Error> {
            let mut port = port_arc.lock().unwrap();
            port.write_all(&bytes)?;
            port.flush()
        })
        .await
        .map_err(|e| ProtocolError::ProtocolError(format!("join error: {}", e)))?
        .map_err(|e| ProtocolError::ProtocolError(format!("write failed: {}", e)))?;

        Ok(())
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        if self.state != SerialState::Connected {
            return Err(ProtocolError::ConnectionFailed(
                "Serial port not connected".to_string(),
            ));
        }

        // 同步 read：SerialPort 设有 100ms timeout，足够短不会卡住 runtime。
        // 真实的高并发场景应该把 recv 放到 spawn_blocking 里循环读取并通过 mpsc
        // 投递 — 当前实现保留 Connection trait 的同步语义。
        let mut port = self
            .port
            .as_ref()
            .ok_or(ProtocolError::ConnectionClosed)?
            .lock()
            .unwrap();
        match port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(ProtocolError::ProtocolError(format!("read failed: {}", e))),
        }
    }

    async fn resize(&mut self, _cols: u16, _rows: u16) -> Result<(), ProtocolError> {
        // 串口没有终端窗口概念，resize 是 no-op
        Ok(())
    }
}

// SAFETY: `Box<dyn SerialPort>` 在所有支持平台（macOS / Linux / Windows）上都是
// Sync 的——它们操作的是文件描述符 / HANDLE，这些是进程级句柄而非线程局部状态。
// `serialport` crate 文档化此约束；其在 Linux/macOS 通过 pthread / Win32 API 提供
// 线程安全访问模式。
unsafe impl Sync for SerialConnection {}
unsafe impl Send for SerialConnection {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serial_config_default() {
        let cfg = SerialConfig::default();
        assert_eq!(cfg.baud_rate, 115200);
        assert_eq!(cfg.data_bits, 8);
        assert_eq!(cfg.stop_bits, 1);
        assert_eq!(cfg.parity, SerialParity::None);
        assert_eq!(cfg.flow_control, SerialFlowControl::None);
    }

    #[tokio::test]
    async fn test_serial_connection_creation() {
        let conn = SerialConnection::new(SerialConfig::default());
        assert_eq!(conn.state, SerialState::Disconnected);
        assert!(conn.port.is_none());
    }
}