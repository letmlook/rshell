//! Telnet 协议实现
//!
//! 基于 RFC 854 的 Telnet 协议实现，使用 Tokio TCP 连接。

#![allow(dead_code)]

use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, debug};

use crate::{Connection, ProtocolError};

/// Telnet 命令字节
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelnetCommand {
    SE = 240,    // 子协商结束
    NOP = 241,   // 无操作
    DM = 242,    // 数据标记
    BRK = 243,   // 中断
    IP = 244,    // 中断进程
    AO = 245,    // 中止输出
    AYT = 246,   // 你在那里
    EC = 247,    // 擦除字符
    EL = 248,    // 擦除行
    GA = 249,    // 继续
    SB = 250,    // 子协商开始
    WILL = 251,  // 愿意
    WONT = 252,  // 不愿意
    DO = 253,    // 要求对方
    DONT = 254,  // 拒绝对方
    IAC = 255,   // 解释为命令
}

/// Telnet 选项
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelnetOption {
    Echo = 1,              // RFC 857
    SuppressGoAhead = 3,   // RFC 858
    TerminalType = 24,     // RFC 1091
    WindowSize = 31,       // RFC 1073
    LineMode = 34,         // RFC 1184
}

/// Telnet 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelnetState {
    Disconnected,
    Connecting,
    Connected,
    Negotiating,
}

/// Telnet 连接
pub struct TelnetConnection {
    host: String,
    port: u16,
    terminal_type: String,
    stream: Option<TcpStream>,
    state: TelnetState,
    /// 是否启用了 Suppress Go Ahead
    suppress_go_ahead: bool,
    /// 是否启用了 Echo（服务器端回显）
    server_echo: bool,
}

impl TelnetConnection {
    /// 创建新的 Telnet 连接
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            terminal_type: "xterm-256color".to_string(),
            stream: None,
            state: TelnetState::Disconnected,
            suppress_go_ahead: false,
            server_echo: false,
        }
    }

    /// 设置终端类型
    pub fn set_terminal_type(&mut self, terminal_type: &str) {
        self.terminal_type = terminal_type.to_string();
    }

    /// 发送 Telnet 命令
    async fn send_command(&mut self, cmd: TelnetCommand, option: Option<TelnetOption>) -> Result<(), ProtocolError> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        let mut buf = vec![TelnetCommand::IAC as u8, cmd as u8];
        if let Some(opt) = option {
            buf.push(opt as u8);
        }

        stream.write_all(&buf).await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send command: {}", e)))?;

        Ok(())
    }

    /// 处理接收到的 Telnet 命令
    async fn handle_command(&mut self, cmd: TelnetCommand, option: u8) -> Result<Vec<u8>, ProtocolError> {
        let mut response = Vec::new();

        match cmd {
            TelnetCommand::DO => {
                match option {
                    x if x == TelnetOption::SuppressGoAhead as u8 => {
                        self.suppress_go_ahead = true;
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::WILL as u8,
                            TelnetOption::SuppressGoAhead as u8,
                        ]);
                        debug!("Telnet: Agreed to Suppress Go Ahead");
                    }
                    x if x == TelnetOption::TerminalType as u8 => {
                        // 发送子协商
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::WILL as u8,
                            TelnetOption::TerminalType as u8,
                        ]);
                        debug!("Telnet: Agreed to Terminal Type");
                    }
                    _ => {
                        // 拒绝其他选项
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::WONT as u8,
                            option,
                        ]);
                    }
                }
            }
            TelnetCommand::WILL => {
                match option {
                    x if x == TelnetOption::Echo as u8 => {
                        self.server_echo = true;
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::DO as u8,
                            TelnetOption::Echo as u8,
                        ]);
                        debug!("Telnet: Accepted server echo");
                    }
                    x if x == TelnetOption::SuppressGoAhead as u8 => {
                        self.suppress_go_ahead = true;
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::DO as u8,
                            TelnetOption::SuppressGoAhead as u8,
                        ]);
                        debug!("Telnet: Accepted server Suppress Go Ahead");
                    }
                    _ => {
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::DONT as u8,
                            option,
                        ]);
                    }
                }
            }
            TelnetCommand::SB => {
                // 子协商 - 简化处理
                debug!("Telnet: Subnegotiation for option {}", option);
            }
            _ => {
                debug!("Telnet: Unhandled command {:?}", cmd);
            }
        }

        Ok(response)
    }

    /// 处理接收到的数据，过滤 Telnet 命令
    async fn process_data(&mut self, data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        let mut output = Vec::new();
        let mut response = Vec::new();
        let mut i = 0;

        while i < data.len() {
            if data[i] == TelnetCommand::IAC as u8 {
                if i + 1 >= data.len() {
                    break;
                }

                let cmd_byte = data[i + 1];
                i += 2;

                match cmd_byte {
                    x if x == TelnetCommand::DO as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::DO, opt).await?;
                            response.extend(resp);
                        }
                    }
                    x if x == TelnetCommand::DONT as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::DONT, opt).await?;
                            response.extend(resp);
                        }
                    }
                    x if x == TelnetCommand::WILL as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::WILL, opt).await?;
                            response.extend(resp);
                        }
                    }
                    x if x == TelnetCommand::WONT as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::WONT, opt).await?;
                            response.extend(resp);
                        }
                    }
                    x if x == TelnetCommand::SB as u8 => {
                        // 跳过子协商直到 SE
                        while i < data.len() {
                            if data[i] == TelnetCommand::IAC as u8 && i + 1 < data.len() && data[i + 1] == TelnetCommand::SE as u8 {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    x if x == TelnetCommand::IAC as u8 => {
                        // 255 255 = 数据字节 255
                        output.push(255);
                    }
                    _ => {
                        // 其他命令，跳过
                    }
                }
            } else {
                output.push(data[i]);
                i += 1;
            }
        }

        // 发送响应命令
        if !response.is_empty() {
            if let Some(stream) = self.stream.as_mut() {
                let _ = stream.write_all(&response).await;
            }
        }

        Ok(output)
    }
}

#[async_trait::async_trait]
impl Connection for TelnetConnection {
    async fn connect(&mut self) -> Result<(), ProtocolError> {
        info!("Connecting to Telnet server {}:{}", self.host, self.port);
        self.state = TelnetState::Connecting;

        let stream = TcpStream::connect(format!("{}:{}", self.host, self.port))
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("TCP connect failed: {}", e)))?;

        self.stream = Some(stream);
        self.state = TelnetState::Connected;

        // 主动请求 Suppress Go Ahead
        self.send_command(TelnetCommand::DO, Some(TelnetOption::SuppressGoAhead)).await?;

        info!("Telnet connection established to {}:{}", self.host, self.port);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        info!("Disconnecting Telnet session");
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.shutdown().await;
        }
        self.state = TelnetState::Disconnected;
        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        // 转义 IAC 字节
        let mut escaped = Vec::with_capacity(data.len());
        for &byte in data {
            escaped.push(byte);
            if byte == TelnetCommand::IAC as u8 {
                escaped.push(TelnetCommand::IAC as u8);
            }
        }

        stream.write_all(&escaped).await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send: {}", e)))?;

        Ok(())
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        let mut raw_buf = vec![0u8; buf.len() * 2]; // 预留空间给命令过滤
        let n = stream.read(&mut raw_buf).await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to recv: {}", e)))?;

        if n == 0 {
            return Err(ProtocolError::ConnectionClosed);
        }

        // 处理 Telnet 命令
        let processed = self.process_data(&raw_buf[..n]).await?;
        let copy_len = processed.len().min(buf.len());
        buf[..copy_len].copy_from_slice(&processed[..copy_len]);

        Ok(copy_len)
    }

    async fn resize(&mut self, _cols: u16, _rows: u16) -> Result<(), ProtocolError> {
        // Telnet 协议通过 NAWS 选项协商窗口大小
        // 简化实现：暂不支持
        debug!("Telnet resize not yet implemented");
        Ok(())
    }
}
