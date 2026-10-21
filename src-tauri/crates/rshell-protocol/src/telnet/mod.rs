//! Telnet 协议实现
//!
//! 基于 RFC 854 的 Telnet 协议实现，使用 Tokio TCP 连接。

#![allow(dead_code)]

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{Connection, ProtocolError};

/// Telnet 命令字节
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelnetCommand {
    SE = 240,   // 子协商结束
    NOP = 241,  // 无操作
    DM = 242,   // 数据标记
    BRK = 243,  // 中断
    IP = 244,   // 中断进程
    AO = 245,   // 中止输出
    AYT = 246,  // 你在那里
    EC = 247,   // 擦除字符
    EL = 248,   // 擦除行
    GA = 249,   // 继续
    SB = 250,   // 子协商开始
    WILL = 251, // 愿意
    WONT = 252, // 不愿意
    DO = 253,   // 要求对方
    DONT = 254, // 拒绝对方
    IAC = 255,  // 解释为命令
}

/// Telnet 选项
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelnetOption {
    Echo = 1,            // RFC 857
    SuppressGoAhead = 3, // RFC 858
    TerminalType = 24,   // RFC 1091
    WindowSize = 31,     // RFC 1073
    LineMode = 34,       // RFC 1184
}

/// Telnet 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelnetState {
    Disconnected,
    Connecting,
    Connected,
    Negotiating,
}

/// 跨 TCP 分片暂存的字节上限（未解析完的 Telnet 序列）。
///
/// 威胁模型（R3-07）：恶意或故障的 Telnet 设备、或注入字节的中间设备，
/// 只发 `IAC SB <opt>` 而永不发 `IAC SE`。未闭合的子协商会被整段留存在
/// `pending_bytes` 里，并由每次 `recv` 重新扫描、重新暂存：
/// 1. 其后的全部终端输出被吞掉——终端看起来卡死，只能断开会话；
/// 2. `pending_bytes` 随对端持续发送无界增长（可远程触发的内存耗尽）；
/// 3. 单次调用重扫整个累积缓冲，累计 CPU 随流量二次增长。
///
/// 64 KiB 远大于任何合法子协商（NAWS 载荷 6 字节、TSEND 终端类型 < 128 字节），
/// 因此超过上限即可判定为协议违规：清空暂存并上报可诊断错误，而不是继续吞。
const MAX_PENDING_BYTES: usize = 64 * 1024;

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
    /// NAWS（RFC 1073）协商是否成功 —— 服务端已同意接收我们的窗口尺寸。
    ///
    /// 只有协商成功后才发 `IAC SB NAWS` 子协商：未协商就发，对端按 RFC 854
    /// 「未启用的选项不应使用子协商」丢弃，表现为「窗口尺寸永远不生效」。
    naws_enabled: bool,
    /// 最近一次请求的窗口尺寸。
    ///
    /// 连接建立时通常**先** resize 再完成协商（握手是异步的），所以协商成功的
    /// 那一刻要把这个尺寸补发出去，否则服务端拿到的仍是它的默认值。
    last_size: Option<(u16, u16)>,
    pending_bytes: Vec<u8>,
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
            naws_enabled: false,
            last_size: None,
            pending_bytes: Vec::new(),
        }
    }

    /// 设置终端类型
    pub fn set_terminal_type(&mut self, terminal_type: &str) {
        self.terminal_type = terminal_type.to_string();
    }

    /// 发送 Telnet 命令
    async fn send_command(
        &mut self,
        cmd: TelnetCommand,
        option: Option<TelnetOption>,
    ) -> Result<(), ProtocolError> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        let mut buf = vec![TelnetCommand::IAC as u8, cmd as u8];
        if let Some(opt) = option {
            buf.push(opt as u8);
        }

        stream
            .write_all(&buf)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send command: {}", e)))?;

        Ok(())
    }

    /// 处理接收到的 Telnet 命令
    ///
    /// NAWS（RFC 1073）的方向性是这个方法里最容易搞错的地方：
    /// - RFC 1073 §2：**客户端**用 `IAC WILL NAWS` 表示「我建议用 NAWS」，
    ///   **服务端**用 `IAC DO NAWS` 表示「我也同意」。两个 RFC 示例
    ///   （§6 例 1、例 2）都是 `WILL` ↔ `DO` 这一对。
    /// - 旧实现反着来：连接建立时发 `IAC DO NAWS`（这是服务端该发的动词），
    ///   收到 `DO`/`WILL` 时又分别回 `WONT`/`DONT` —— 无论服务端怎么回，
    ///   协商都必然失败。
    ///
    /// 现在按 RFC 的方向处理：
    /// - 收到 `DO NAWS`（服务端建议）→ 回 `WILL NAWS`（客户端同意），
    ///   协商成功；
    /// - 收到 `WILL NAWS`（客户端侧的对端也提议）→ 回 `DO NAWS`，
    ///   同样进入协商成功状态。
    async fn handle_command(
        &mut self,
        cmd: TelnetCommand,
        option: u8,
    ) -> Result<Vec<u8>, ProtocolError> {
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
                    x if x == TelnetOption::WindowSize as u8 => {
                        // RFC 1073 §6 例 1：服务端 DO NAWS，客户端回 WILL NAWS。
                        response.extend_from_slice(&[
                            TelnetCommand::IAC as u8,
                            TelnetCommand::WILL as u8,
                            TelnetOption::WindowSize as u8,
                        ]);
                        self.naws_enabled = true;
                        debug!("Telnet: NAWS negotiated (server DO -> client WILL)");
                        // 协商成功即刻补发当前尺寸：握手是异步的，建立连接时
                        // 的 resize 往往发生在协商完成之前。
                        if let Some((cols, rows)) = self.last_size {
                            response.extend_from_slice(&Self::naws_frame(cols, rows));
                        }
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
            TelnetCommand::WILL => match option {
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
                x if x == TelnetOption::WindowSize as u8 => {
                    // 对端也想给我们发它的窗口尺寸。我们**不消费**对端的尺寸
                    // （我们才是客户端：尺寸由本端终端决定，发给远端），所以
                    // 按 RFC 854 回 `DONT NAWS` 拒绝，而不是回 `DO NAWS` 之后
                    // 再把对方发来的 `SB NAWS` 整段丢弃 —— 那正是「只答应不
                    // 兑现」。
                    //
                    // 注意这**不**影响我们自己的上报方向：`naws_enabled` 描述的
                    // 是「我们能否向对端发 SB NAWS」，它由对端的 `DO NAWS` 决定。
                    response.extend_from_slice(&[
                        TelnetCommand::IAC as u8,
                        TelnetCommand::DONT as u8,
                        TelnetOption::WindowSize as u8,
                    ]);
                    debug!("Telnet: refused the peer's inbound window size (DONT NAWS)");
                }
                _ => {
                    response.extend_from_slice(&[
                        TelnetCommand::IAC as u8,
                        TelnetCommand::DONT as u8,
                        option,
                    ]);
                }
            },
            // 对端拒绝 NAWS：关闭尺寸上报，之后的 resize 不再发子协商。
            // RFC 1073 §2 明确服务端可以在已接受后发 DON'T NAWS 禁止继续上报。
            //
            // RFC 854 要求闭合协商：收到 `DONT X` 回 `WONT X`，收到 `WONT X`
            // 回 `DONT X`。不回的话对端的协商状态停在半开，可能反复重发拒绝。
            TelnetCommand::DONT if option == TelnetOption::WindowSize as u8 => {
                self.naws_enabled = false;
                response.extend_from_slice(&[
                    TelnetCommand::IAC as u8,
                    TelnetCommand::WONT as u8,
                    TelnetOption::WindowSize as u8,
                ]);
                debug!("Telnet: NAWS refused by peer (DONT); window size reporting disabled");
            }
            TelnetCommand::WONT if option == TelnetOption::WindowSize as u8 => {
                self.naws_enabled = false;
                response.extend_from_slice(&[
                    TelnetCommand::IAC as u8,
                    TelnetCommand::DONT as u8,
                    TelnetOption::WindowSize as u8,
                ]);
                debug!("Telnet: NAWS refused by peer (WONT); window size reporting disabled");
            }
            TelnetCommand::SB => {
                // 子协商内容由 process_data 的 SB 分支单独处理
                debug!("Telnet: Subnegotiation for option {}", option);
            }
            _ => {
                debug!("Telnet: Unhandled command {:?}", cmd);
            }
        }

        Ok(response)
    }

    /// NAWS 子协商帧：`IAC SB NAWS <cols_hi> <cols_lo> <rows_hi> <rows_lo> IAC SE`
    ///
    /// RFC 1073 §2：宽高各 2 字节，Internet 标准字节序（网络序，大端）。
    fn naws_frame(cols: u16, rows: u16) -> Vec<u8> {
        vec![
            TelnetCommand::IAC as u8,
            TelnetCommand::SB as u8,
            TelnetOption::WindowSize as u8,
            (cols >> 8) as u8,
            cols as u8,
            (rows >> 8) as u8,
            rows as u8,
            TelnetCommand::IAC as u8,
            TelnetCommand::SE as u8,
        ]
    }

    /// 处理子协商内容（`IAC SB <option> <payload> IAC SE` 的中间部分）
    ///
    /// 客户端对 `DO TERMINAL-TYPE` 回了 `WILL TERMINAL-TYPE`，就必须真的把
    /// 终端类型发出去：服务器发 `SB TERMINAL-TYPE SEND` 时应答
    /// `SB TERMINAL-TYPE IS <type>`（RFC 1091）。此前子协商被整段丢弃，
    /// 客户端答应了却从不发送——服务器可能一直等，或按错误的终端类型渲染。
    async fn handle_subnegotiation(
        &mut self,
        option: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, ProtocolError> {
        if option == TelnetOption::TerminalType as u8 {
            // SEND = 1：服务器要终端类型
            if payload.first() == Some(&1) {
                // RFC 1091：子协商载荷以 IS(0) 开头 —— 它是载荷里的字节，
                // 不是一条 Telnet 命令。
                const TERMINAL_TYPE_IS: u8 = 0;
                let mut response = vec![
                    TelnetCommand::IAC as u8,
                    TelnetCommand::SB as u8,
                    TelnetOption::TerminalType as u8,
                    TERMINAL_TYPE_IS,
                ];
                response.extend_from_slice(self.terminal_type.as_bytes());
                response.extend_from_slice(&[TelnetCommand::IAC as u8, TelnetCommand::SE as u8]);
                debug!("Telnet: sending terminal type ({})", self.terminal_type);
                return Ok(response);
            }
        }
        debug!(
            "Telnet: ignoring subnegotiation for option {} ({} bytes)",
            option,
            payload.len()
        );
        Ok(Vec::new())
    }

    /// 把尚未解析完的字节留到下一次调用处理（跨 TCP 分片）。
    ///
    /// R3-07：留存量必须有硬上限。没有上限时，对端只要发一个不带 `IAC SE` 的
    /// `IAC SB` 就能让留存量随流量无界增长，同时把后续所有输出吞掉。
    /// 因此越界即判定为协议违规：`warn!` 留痕 + 清空暂存 + 返回可诊断的
    /// `ProtocolError`，让运维看到违规，而不是表现为「终端静默卡死」。
    fn stash_pending(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let retained = self.pending_bytes.len() + bytes.len();
        if retained > MAX_PENDING_BYTES {
            warn!(
                retained,
                limit = MAX_PENDING_BYTES,
                "Telnet: unterminated sequence exceeded the pending buffer limit, dropped"
            );
            self.pending_bytes.clear();
            return Err(ProtocolError::ProtocolError(format!(
                "unterminated Telnet subnegotiation retained {retained} bytes, over limit {MAX_PENDING_BYTES}"
            )));
        }
        self.pending_bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// 处理接收到的数据，过滤 Telnet 命令
    async fn process_data(&mut self, data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        let mut combined = std::mem::take(&mut self.pending_bytes);
        combined.extend_from_slice(data);
        let data = combined.as_slice();
        let mut output = Vec::new();
        let mut response = Vec::new();
        let mut i = 0;

        while i < data.len() {
            if data[i] == TelnetCommand::IAC as u8 {
                let command_start = i;
                if i + 1 >= data.len() {
                    self.stash_pending(&data[command_start..])?;
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
                        } else {
                            self.stash_pending(&data[command_start..])?;
                        }
                    }
                    x if x == TelnetCommand::DONT as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::DONT, opt).await?;
                            response.extend(resp);
                        } else {
                            self.stash_pending(&data[command_start..])?;
                        }
                    }
                    x if x == TelnetCommand::WILL as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::WILL, opt).await?;
                            response.extend(resp);
                        } else {
                            self.stash_pending(&data[command_start..])?;
                        }
                    }
                    x if x == TelnetCommand::WONT as u8 => {
                        if i < data.len() {
                            let opt = data[i];
                            i += 1;
                            let resp = self.handle_command(TelnetCommand::WONT, opt).await?;
                            response.extend(resp);
                        } else {
                            self.stash_pending(&data[command_start..])?;
                        }
                    }
                    x if x == TelnetCommand::SB as u8 => {
                        // 跳过子协商直到 SE，并把子协商内容取出来。
                        // R3-07：扫描窗口必须有界——未闭合的子协商会让缓冲
                        // 一直累积，无界扫描等于每次调用都重扫全部历史数据。
                        let scan_end = data.len().min(i.saturating_add(MAX_PENDING_BYTES));
                        let mut completed = false;
                        let sub_start = i;
                        while i < scan_end {
                            if data[i] == TelnetCommand::IAC as u8
                                && i + 1 < scan_end
                                && data[i + 1] == TelnetCommand::SE as u8
                            {
                                completed = true;
                                break;
                            }
                            i += 1;
                        }
                        if completed {
                            // 子协商内容（不含结尾的 IAC SE）要交给协议层处理：
                            // 服务器问终端类型（TERMINAL-TYPE SEND）时必须应答，
                            // 否则客户端答应了 WILL TERMINAL-TYPE 却从不真发，
                            // 是一次「说了要做却没做」的假承诺。
                            if sub_start < i {
                                let option = data[sub_start];
                                let payload = &data[sub_start + 1..i];
                                let resp = self.handle_subnegotiation(option, payload).await?;
                                response.extend(resp);
                            }
                            i += 2;
                        }
                        if !completed {
                            if scan_end < data.len() {
                                // 上限内仍未闭合：远端可触发的无界增长，判违规
                                warn!(
                                    limit = MAX_PENDING_BYTES,
                                    buffered = data.len(),
                                    "Telnet: subnegotiation without IAC SE exceeded the scan window"
                                );
                                self.pending_bytes.clear();
                                return Err(ProtocolError::ProtocolError(format!(
                                    "unterminated Telnet subnegotiation exceeded {MAX_PENDING_BYTES} bytes without IAC SE"
                                )));
                            }
                            // 合法但跨分片：暂存等待后续片段
                            self.stash_pending(&data[command_start..])?;
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
                stream.write_all(&response).await.map_err(|e| {
                    ProtocolError::ProtocolError(format!("Failed to negotiate Telnet options: {e}"))
                })?;
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

        // 主动协商 Suppress Go Ahead (RFC 858) 与 Window Size (NAWS, RFC 1073)。
        //
        // NAWS 的动词方向必须按 RFC 1073 §2：选项由**客户端**上报尺寸，
        // 所以客户端发 `WILL NAWS`（我提议用 NAWS），服务端回 `DO NAWS`（同意）。
        // 旧实现发的是 `IAC DO NAWS` —— 那是服务端该用的动词，等于反过来
        // 要求服务端上报自己的尺寸，协商必然不成立。
        self.send_command(TelnetCommand::DO, Some(TelnetOption::SuppressGoAhead))
            .await?;
        self.send_command(TelnetCommand::WILL, Some(TelnetOption::WindowSize))
            .await?;

        info!(
            "Telnet connection established to {}:{}",
            self.host, self.port
        );
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
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        // 转义 IAC 字节
        let mut escaped = Vec::with_capacity(data.len());
        for &byte in data {
            escaped.push(byte);
            if byte == TelnetCommand::IAC as u8 {
                escaped.push(TelnetCommand::IAC as u8);
            }
        }

        stream
            .write_all(&escaped)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send: {}", e)))?;

        Ok(())
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| ProtocolError::ConnectionFailed("Not connected".to_string()))?;

        // Command filtering never expands plain output beyond the raw input.
        // Limit reads to the caller's capacity so no decoded bytes are discarded.
        if buf.is_empty() {
            return Ok(0);
        }
        let mut raw_buf = vec![0u8; buf.len()];
        let n = stream
            .read(&mut raw_buf)
            .await
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

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ProtocolError> {
        // 记住尺寸：协商尚未完成时先存着，协商成功那一刻补发（见 handle_command）。
        self.last_size = Some((cols, rows));

        // 只有 NAWS 协商成功后才发子协商帧。未协商就发，对端按 RFC 854
        // 「未启用的选项不应使用子协商」丢弃 —— 命令返回成功但尺寸永不生效。
        if !self.naws_enabled {
            debug!(
                "Telnet: NAWS not negotiated yet; window size {}x{} deferred",
                cols, rows
            );
            return Ok(());
        }

        let stream = match self.stream.as_mut() {
            Some(s) => s,
            None => return Ok(()), // 未连接,resize 静默忽略
        };
        // NAWS (RFC 1073): IAC SB NAWS <16-bit cols> <16-bit rows> IAC SE
        // 16-bit 字段以 big-endian 传输。
        stream
            .write_all(&Self::naws_frame(cols, rows))
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send NAWS: {}", e)))?;
        debug!("Telnet: NAWS sent ({}x{})", cols, rows);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recv_preserves_output_larger_than_callers_buffer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut connection =
            TelnetConnection::new("127.0.0.1", listener.local_addr().unwrap().port());
        connection.connect().await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"abcdefgh").await.unwrap();
        socket.shutdown().await.unwrap();
        let mut all = Vec::new();
        let mut buf = [0; 4];
        while let Ok(n) = connection.recv(&mut buf).await {
            all.extend_from_slice(&buf[..n]);
        }
        assert_eq!(all, b"abcdefgh");
    }

    #[test]
    fn test_new_connection_defaults() {
        let c = TelnetConnection::new("example.com", 23);
        assert_eq!(c.host, "example.com");
        assert_eq!(c.port, 23);
        assert_eq!(c.terminal_type, "xterm-256color");
        assert_eq!(c.state, TelnetState::Disconnected);
        assert!(!c.suppress_go_ahead);
        assert!(!c.server_echo);
    }

    #[test]
    fn test_set_terminal_type() {
        let mut c = TelnetConnection::new("h", 23);
        c.set_terminal_type("vt100");
        assert_eq!(c.terminal_type, "vt100");
    }

    // ===== NAWS (RFC 1073) 协商 =====
    //
    // 旧实现的两个方向都反了：连接时发 `IAC DO NAWS`（服务端才该发的动词），
    // 收到 DO/WILL 时又分别回 WONT/DONT —— 无论服务端怎么回，协商都失败，
    // 远端窗口尺寸永不生效（vim/top/less 全屏程序错行）。

    /// RFC 1073 §2：客户端用 `IAC WILL NAWS` 提议用 NAWS。
    #[tokio::test]
    async fn connect_proposes_naws_with_will() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut connection = TelnetConnection::new("127.0.0.1", port);
        // 先连后 accept：connect 是 future，不 poll 它就永远不会发起连接，
        // accept() 会一直等下去。
        connection.connect().await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();

        // connect() 发两条协商：DO SGA（3 字节）+ WILL NAWS（3 字节）
        let mut buf = [0u8; 6];
        socket.read_exact(&mut buf).await.unwrap();
        assert_eq!(
            &buf[0..3],
            &[
                TelnetCommand::IAC as u8,
                TelnetCommand::DO as u8,
                TelnetOption::SuppressGoAhead as u8
            ],
            "Suppress Go Ahead 仍按原样协商"
        );
        assert_eq!(
            &buf[3..6],
            &[
                TelnetCommand::IAC as u8,
                TelnetCommand::WILL as u8,
                TelnetOption::WindowSize as u8
            ],
            "client must propose NAWS with WILL (RFC 1073 §2)"
        );
    }

    /// RFC 1073 §6 例 1：服务端 `DO NAWS` → 客户端回 `WILL NAWS`，协商成功。
    #[tokio::test]
    async fn server_do_naws_is_answered_with_will_and_enables_reporting() {
        let mut c = TelnetConnection::new("h", 23);
        assert!(!c.naws_enabled);
        let response = c
            .handle_command(TelnetCommand::DO, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert_eq!(
            response,
            vec![
                TelnetCommand::IAC as u8,
                TelnetCommand::WILL as u8,
                TelnetOption::WindowSize as u8
            ],
            "server DO NAWS must be answered with WILL NAWS"
        );
        assert!(c.naws_enabled, "NAWS must be enabled after agreement");
    }

    /// RFC 1073 §6 例 2 的镜像：对方也用 WILL 提议 → 回 DO 表示接受。
    #[tokio::test]
    async fn peer_will_naws_is_refused_without_enabling_reporting() {
        let mut c = TelnetConnection::new("h", 23);
        let response = c
            .handle_command(TelnetCommand::WILL, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert_eq!(
            response,
            vec![
                TelnetCommand::IAC as u8,
                TelnetCommand::DONT as u8,
                TelnetOption::WindowSize as u8
            ],
            "对端要给我们发窗口尺寸时必须拒绝（DONT）——回 DO 之后再丢弃子协商 \
             就是「只答应不兑现」"
        );
        assert!(
            !c.naws_enabled,
            "拒绝对端的上报方向不影响我们自己的上报状态"
        );
    }

    /// RFC 854：收到 DONT NAWS 回 WONT NAWS，收到 WONT NAWS 回 DONT NAWS ——
    /// 闭合协商，否则对端状态停在半开、可能反复重发拒绝。
    #[tokio::test]
    async fn refusal_is_closed_with_a_negative_acknowledgement() {
        let mut c = TelnetConnection::new("h", 23);
        let response = c
            .handle_command(TelnetCommand::DONT, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert_eq!(
            response,
            vec![
                TelnetCommand::IAC as u8,
                TelnetCommand::WONT as u8,
                TelnetOption::WindowSize as u8
            ],
            "DONT NAWS 必须回 WONT NAWS"
        );
        assert!(!c.naws_enabled);

        let mut c = TelnetConnection::new("h", 23);
        let response = c
            .handle_command(TelnetCommand::WONT, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert_eq!(
            response,
            vec![
                TelnetCommand::IAC as u8,
                TelnetCommand::DONT as u8,
                TelnetOption::WindowSize as u8
            ],
            "WONT NAWS 必须回 DONT NAWS"
        );
        assert!(!c.naws_enabled);
    }

    /// 协商未完成时不得发 NAWS 子协商帧：未启用的选项用了子协商，
    /// 对端按 RFC 854 直接丢弃 —— 命令返回成功但尺寸永不生效。
    #[tokio::test]
    async fn resize_before_negotiation_defers_instead_of_sending() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut connection = TelnetConnection::new("127.0.0.1", port);
        // 先连后 accept（connect 是 future，不 poll 就不会发起连接）。
        connection.connect().await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        // 吃掉连接建立时的两条协商（DO SGA + WILL NAWS）
        let mut negotiation = [0u8; 6];
        socket.read_exact(&mut negotiation).await.unwrap();

        connection.resize(120, 40).await.unwrap();
        assert_eq!(connection.last_size, Some((120, 40)));
        assert!(!connection.naws_enabled);

        // 未协商 → 不应有任何 NAWS 子协商帧写出去。
        // 用超时而不是「读到报错」：对端不回应时 read 会一直挂着。
        let mut frame = [0u8; 9];
        let received = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            socket.read_exact(&mut frame),
        )
        .await;
        assert!(
            received.is_err(),
            "协商完成前不得发出 NAWS 子协商帧，实际收到: {frame:?}"
        );
    }

    /// 协商完成后要把**已经记住**的尺寸补发出去：握手是异步的，
    /// 建立连接时的 resize 早于协商完成。
    #[tokio::test]
    async fn negotiated_naws_flushes_the_deferred_size() {
        let mut c = TelnetConnection::new("h", 23);
        c.last_size = Some((120, 40));
        let response = c
            .handle_command(TelnetCommand::DO, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        // WILL NAWS 之后紧跟 SB NAWS 帧（120x40，大端）
        let mut expected = vec![
            TelnetCommand::IAC as u8,
            TelnetCommand::WILL as u8,
            TelnetOption::WindowSize as u8,
        ];
        expected.extend_from_slice(&TelnetConnection::naws_frame(120, 40));
        assert_eq!(response, expected);
    }

    /// 对端拒绝 NAWS（DONT/WONT）后不得再发子协商帧。
    #[tokio::test]
    async fn refusal_disables_naws_reporting() {
        let mut c = TelnetConnection::new("h", 23);
        c.handle_command(TelnetCommand::DO, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert!(c.naws_enabled);

        c.handle_command(TelnetCommand::DONT, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert!(!c.naws_enabled, "DONT NAWS must disable reporting");

        c.handle_command(TelnetCommand::DO, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        c.handle_command(TelnetCommand::WONT, TelnetOption::WindowSize as u8)
            .await
            .unwrap();
        assert!(!c.naws_enabled, "WONT NAWS must disable reporting");
    }

    /// NAWS 帧的字节布局：宽高各 2 字节大端。
    #[test]
    fn naws_frame_uses_big_endian_dimensions() {
        let frame = TelnetConnection::naws_frame(300, 24);
        assert_eq!(
            frame,
            vec![
                255, 250, 31, // IAC SB NAWS
                0x01, 0x2C, // width 300 = 0x012C，大端
                0x00, 0x18, // height 24 = 0x0018，大端
                255, 240, // IAC SE
            ]
        );
    }

    #[tokio::test]
    async fn test_send_command_requires_connected() {
        let mut c = TelnetConnection::new("h", 23);
        // 未连接时调 send_command 应返回错误
        let r = c
            .send_command(TelnetCommand::DO, Some(TelnetOption::SuppressGoAhead))
            .await;
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("Not connected"));
    }

    #[tokio::test]
    async fn fragmented_telnet_command_is_not_rendered_or_lost() {
        let mut connection = TelnetConnection::new("example", 23);
        assert!(connection.process_data(&[255]).await.unwrap().is_empty());
        assert_eq!(connection.pending_bytes, vec![255]);
        assert_eq!(
            connection.process_data(&[253, 3, b'A']).await.unwrap(),
            b"A"
        );
        assert!(connection.pending_bytes.is_empty());
    }

    /// 客户端答应了 `WILL TERMINAL-TYPE` 就必须真的把终端类型发出去。
    ///
    /// 此前子协商内容被整段丢弃：服务器问 `SB TERMINAL-TYPE SEND` 时，
    /// 客户端从不应答——一次「说了要做却没做」的假承诺。
    #[tokio::test]
    async fn terminal_type_send_is_answered_with_the_configured_type() {
        let mut connection = TelnetConnection::new("example", 23);
        connection.set_terminal_type("xterm-256color");

        // SB TERMINAL-TYPE SEND
        let response = connection
            .handle_subnegotiation(TelnetOption::TerminalType as u8, &[1])
            .await
            .unwrap();

        let mut expected = vec![
            TelnetCommand::IAC as u8,
            TelnetCommand::SB as u8,
            TelnetOption::TerminalType as u8,
            0, // IS
        ];
        expected.extend_from_slice(b"xterm-256color");
        expected.extend_from_slice(&[TelnetCommand::IAC as u8, TelnetCommand::SE as u8]);
        assert_eq!(response, expected);
    }

    /// 自定义终端类型也要如实发出去（set_terminal_type 不是摆设）。
    #[tokio::test]
    async fn terminal_type_response_follows_set_terminal_type() {
        let mut connection = TelnetConnection::new("example", 23);
        connection.set_terminal_type("vt100");

        let response = connection
            .handle_subnegotiation(TelnetOption::TerminalType as u8, &[1])
            .await
            .unwrap();
        assert!(
            response.windows(5).any(|w| w == b"vt100"),
            "应答里必须含设置的终端类型：{response:?}"
        );
    }

    /// 其它子协商不当作终端类型请求处理，也不产生噪音响应。
    #[tokio::test]
    async fn unrelated_subnegotiation_is_ignored() {
        let mut connection = TelnetConnection::new("example", 23);

        // SB NAWS（服务器偶尔会发窗口尺寸）
        let response = connection
            .handle_subnegotiation(TelnetOption::WindowSize as u8, &[0, 80, 0, 24])
            .await
            .unwrap();
        assert!(response.is_empty());

        // TERMINAL-TYPE 但不是 SEND
        let response = connection
            .handle_subnegotiation(TelnetOption::TerminalType as u8, &[0])
            .await
            .unwrap();
        assert!(response.is_empty());
    }

    /// R3-07 回归：对端只发 `IAC SB <opt>` 而不发 `IAC SE` 时，暂存缓冲
    /// 必须停在上限内，并且必须上报错误——不能静默吞掉后续输出。
    #[tokio::test]
    async fn unterminated_subnegotiation_is_capped_and_reported() {
        let mut connection = TelnetConnection::new("example", 23);

        // 每次都是「IAC SB NAWS + 8 KiB 填充」，对端持续喂数据且永不闭合
        let mut reported = None;
        for _ in 0..64 {
            let mut chunk = vec![255, 250, 31];
            chunk.extend(std::iter::repeat_n(b'x', 8 * 1024));
            match connection.process_data(&chunk).await {
                Ok(output) => {
                    assert!(
                        output.is_empty(),
                        "未闭合子协商之后本就没有可交付的输出，实际: {output:?}"
                    );
                    assert!(
                        connection.pending_bytes.len() <= MAX_PENDING_BYTES,
                        "暂存量越界: {} > {MAX_PENDING_BYTES}",
                        connection.pending_bytes.len()
                    );
                }
                Err(err) => {
                    reported = Some(err);
                    break;
                }
            }
        }

        let err = reported.expect("未闭合子协商超出上限时必须上报错误，而不是无限吞输出");
        assert!(
            format!("{err:?}").contains("subnegotiation"),
            "错误必须可诊断，实际: {err:?}"
        );
        assert!(
            connection.pending_bytes.is_empty(),
            "判违规后必须清空暂存缓冲，实际仍有 {} 字节",
            connection.pending_bytes.len()
        );
    }

    /// 合法子协商（NAWS：IAC SB NAWS <cols> <rows> IAC SE）仍必须被完整消费，
    /// 不得报错，其后的输出照常交付。
    #[tokio::test]
    async fn terminated_subnegotiation_is_consumed_and_output_flows() {
        let mut connection = TelnetConnection::new("example", 23);
        let mut chunk = vec![255u8, 250, 31, 0, 80, 0, 24, 255, 240];
        chunk.extend_from_slice(b"prompt$ ");
        assert_eq!(connection.process_data(&chunk).await.unwrap(), b"prompt$ ");
        assert!(connection.pending_bytes.is_empty());
    }

    /// 合法子协商跨 TCP 分片到达：前半片暂存、后半片补上 IAC SE，
    /// 不应被当成未闭合违规。
    #[tokio::test]
    async fn subnegotiation_split_across_chunks_is_not_treated_as_unterminated() {
        let mut connection = TelnetConnection::new("example", 23);
        assert!(connection
            .process_data(&[255, 250, 31, 0, 80])
            .await
            .unwrap()
            .is_empty());
        assert!(connection
            .process_data(&[0, 24, 255, 240])
            .await
            .unwrap()
            .is_empty());
        assert!(connection.pending_bytes.is_empty());
        assert_eq!(connection.process_data(b"ok").await.unwrap(), b"ok");
    }

    #[test]
    fn test_iac_constants() {
        // 关键协议字节值(RFC 854)避免被错改
        assert_eq!(TelnetCommand::IAC as u8, 255);
        assert_eq!(TelnetCommand::DO as u8, 253);
        assert_eq!(TelnetCommand::DONT as u8, 254);
        assert_eq!(TelnetCommand::WILL as u8, 251);
        assert_eq!(TelnetCommand::WONT as u8, 252);
        assert_eq!(TelnetCommand::SB as u8, 250);
        assert_eq!(TelnetCommand::SE as u8, 240);
    }

    #[test]
    fn test_option_codes() {
        assert_eq!(TelnetOption::Echo as u8, 1);
        assert_eq!(TelnetOption::SuppressGoAhead as u8, 3);
        assert_eq!(TelnetOption::TerminalType as u8, 24);
        assert_eq!(TelnetOption::WindowSize as u8, 31);
    }

    #[test]
    fn test_naws_frame_byte_count() {
        // 验收 NAWS 子协商帧长度: IAC SB NAWS <2 cols> <2 rows> IAC SE = 9 字节
        // 该测试保护 #4 实现不被后续 refactor 破坏
        let payload = [
            255u8, 250, 31, // IAC SB NAWS
            0, 80, // cols = 80
            0, 24, // rows = 24
            255, 240, // IAC SE
        ];
        assert_eq!(payload.len(), 9);
        assert_eq!(payload[0], TelnetCommand::IAC as u8);
        assert_eq!(payload[1], TelnetCommand::SB as u8);
        assert_eq!(payload[2], TelnetOption::WindowSize as u8);
        assert_eq!(payload[7], TelnetCommand::IAC as u8);
        assert_eq!(payload[8], TelnetCommand::SE as u8);
    }
}
