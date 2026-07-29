//! 串口协议实现
//!
//! 使用 serialport crate 实现串口通信。

#![allow(dead_code)]

use tracing::{info, debug};

use crate::{Connection, ProtocolError};

/// 串口配置
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

/// 串口流控制
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

/// 串口连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SerialState {
    Disconnected,
    Connected,
}

/// 串口连接
pub struct SerialConnection {
    config: SerialConfig,
    state: SerialState,
    /// 使用 Box<dyn serialport::SerialPort> 在支持 serialport crate 时使用
    /// 当前为结构体实现，实际串口操作需要 serialport crate
    buffer: Vec<u8>,
}

impl SerialConnection {
    /// 创建新的串口连接
    pub fn new(config: SerialConfig) -> Self {
        Self {
            config,
            state: SerialState::Disconnected,
            buffer: Vec::new(),
        }
    }

    /// 获取串口配置
    pub fn config(&self) -> &SerialConfig {
        &self.config
    }

    /// 设置波特率
    pub fn set_baud_rate(&mut self, baud_rate: u32) {
        self.config.baud_rate = baud_rate;
    }

    /// 列出可用串口
    pub fn list_ports() -> Result<Vec<String>, ProtocolError> {
        // 实际实现需要 serialport crate
        // serialport::available_ports() 返回 Vec<SerialPortInfo>
        debug!("Listing available serial ports (stub)");
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Connection for SerialConnection {
    async fn connect(&mut self) -> Result<(), ProtocolError> {
        info!(
            "Connecting to serial port {} at {} baud",
            self.config.port, self.config.baud_rate
        );

        // 实际实现需要 serialport crate:
        // let port = serialport::new(&self.config.port, self.config.baud_rate)
        //     .data_bits(serialport::DataBits::Eight)
        //     .stop_bits(serialport::StopBits::One)
        //     .parity(serialport::Parity::None)
        //     .flow_control(serialport::FlowControl::None)
        //     .timeout(Duration::from_millis(100))
        //     .open()?;

        // 当前为结构体实现，标记为已连接
        self.state = SerialState::Connected;
        info!("Serial port {} connected (stub)", self.config.port);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        info!("Disconnecting serial port {}", self.config.port);
        self.state = SerialState::Disconnected;
        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError> {
        if self.state != SerialState::Connected {
            return Err(ProtocolError::ConnectionFailed("Serial port not connected".to_string()));
        }

        // 实际实现需要写入 serialport:
        // self.port.write_all(data)?;

        debug!("Serial send {} bytes (stub)", data.len());
        Ok(())
    }

    async fn recv(&mut self, _buf: &mut [u8]) -> Result<usize, ProtocolError> {
        if self.state != SerialState::Connected {
            return Err(ProtocolError::ConnectionFailed("Serial port not connected".to_string()));
        }

        // 实际实现需要从 serialport 读取:
        // let n = self.port.read(buf)?;

        // 当前为 stub 实现
        debug!("Serial recv (stub)");
        Ok(0)
    }

    async fn resize(&mut self, _cols: u16, _rows: u16) -> Result<(), ProtocolError> {
        // 串口不支持窗口大小调整
        Ok(())
    }
}
