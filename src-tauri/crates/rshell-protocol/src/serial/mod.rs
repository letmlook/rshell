//! 串口协议实现
//!
//! 基于 `serialport` crate 实现串口通信。
//!
//! `serialport::SerialPort` 是**同步阻塞**的（POSIX / Win32 read / Win32 overlapped
//! depending on platform），因此所有 I/O 都包在 `tokio::task::spawn_blocking` 中，
//! 以避免阻塞后端 runtime 的事件循环。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serialport::{SerialPort, SerialPortInfo};
use tokio::task;
use tracing::{info, warn};

use crate::{Connection, ProtocolError};

/// 读超时：阻塞读最多挂这么久，超时按「暂无数据」处理并重启读。
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// 写侧抢锁的有界等待上限（R3-09）。
///
/// 读者在 `spawn_blocking` 里持锁做阻塞读，`READ_TIMEOUT` 到点返回
/// `TimedOut` 后调用方立刻重启读，锁几乎 100% 时间被占住。若写侧用
/// `lock()` 无限等待，每次按键都会被排在读后面随机延迟 0–100ms，
/// 粘贴突发更是完全串行化——违反「不要在等待远端输入时阻塞发送路径」。
///
/// 首选方案是给写侧独立句柄（见 `connect`），这条有界等待只是
/// `try_clone` 失败时的兜底：串口写本身只要几十微秒，读者每
/// `READ_TIMEOUT` 就会释放一次锁，因此给 2.5 倍读超时的窗口足够抢到锁；
/// 抢不到就返回可诊断错误，让这次发送失败可见，而不是无限期排队。
const WRITE_LOCK_TIMEOUT: Duration = Duration::from_millis(250);

/// 抢锁重试间隔
const WRITE_LOCK_RETRY: Duration = Duration::from_millis(1);

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
///
/// 读、写各持一个句柄（R3-09）：写句柄由读句柄 `try_clone` 得到（POSIX 上
/// `dup(fd)`、Windows 上 `DuplicateHandle`，是同一底层句柄的副本，不二次打开
/// 设备），因此写路径不与阻塞读竞争。
pub struct SerialConnection {
    config: SerialConfig,
    state: SerialState,
    /// 读侧句柄：阻塞读期间长期持锁
    port: Option<Arc<Mutex<Box<dyn SerialPort>>>>,
    /// 写侧句柄：`try_clone` 失败时为 `None`，此时退回读句柄 + 有界抢锁
    write_port: Option<Arc<Mutex<Box<dyn SerialPort>>>>,
    read_task: Option<task::JoinHandle<(std::io::Result<usize>, Vec<u8>)>>,
    pending_bytes: VecDeque<u8>,
}

impl SerialConnection {
    /// 创建新的串口连接（尚未连接）
    pub fn new(config: SerialConfig) -> Self {
        Self {
            config,
            state: SerialState::Disconnected,
            port: None,
            write_port: None,
            read_task: None,
            pending_bytes: VecDeque::new(),
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
            Ok(ports
                .into_iter()
                .map(|p: SerialPortInfo| p.port_name)
                .collect())
        })
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("join error: {}", e)))?
    }
}

/// 写侧在 `WRITE_LOCK_TIMEOUT` 内抢到串口锁。
///
/// 只用 `try_lock` + 有界重试，绝不无限等待：调用方跑在 `spawn_blocking`
/// 里，一次无限等待会把按键无限期排在读者的阻塞读后面（R3-09）。
/// 有独立写句柄时这里第一次就成功；兜底路径下读者每 `READ_TIMEOUT`
/// 释放一次锁，重试足以抢到窗口。
fn lock_for_write(
    port: &Arc<Mutex<Box<dyn SerialPort>>>,
) -> Result<std::sync::MutexGuard<'_, Box<dyn SerialPort>>, std::io::Error> {
    let deadline = std::time::Instant::now() + WRITE_LOCK_TIMEOUT;
    loop {
        match port.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(std::io::Error::other("serial port lock poisoned"));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "serial port still locked by a read after {}ms",
                            WRITE_LOCK_TIMEOUT.as_millis()
                        ),
                    ));
                }
                std::thread::sleep(WRITE_LOCK_RETRY);
            }
        }
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
                .timeout(READ_TIMEOUT)
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
        .map_err(|e| ProtocolError::ConnectionFailed(format!("join error: {}", e)))??;

        // R3-09：写侧要独立句柄。`SerialPort::try_clone` 是 serialport 为
        // 「同时读写同一连接」提供的正式 API：POSIX 上 dup(fd)、Windows 上
        // DuplicateHandle，同一底层句柄的副本，兼容性等同原句柄。
        // 克隆失败不阻断连接：退回读句柄 + 有界抢锁，并 warn 留痕。
        let write_port = match port.try_clone() {
            Ok(clone) => Some(Arc::new(Mutex::new(clone))),
            Err(e) => {
                warn!(
                    port = %self.config.port,
                    error = %e,
                    "Serial: write handle clone failed, falling back to bounded lock on the read handle"
                );
                None
            }
        };

        self.port = Some(Arc::new(Mutex::new(port)));
        self.write_port = write_port;
        self.state = SerialState::Connected;
        info!("Serial port {} connected", self.config.port);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ProtocolError> {
        info!("Disconnecting serial port {}", self.config.port);
        if let Some(read) = self.read_task.take() {
            // A blocking read cannot be cancelled. Join the bounded (100ms) read
            // before releasing the port, so no detached reader survives disconnect.
            let _ = read.await;
        }
        self.pending_bytes.clear();
        // 写句柄是读句柄的副本，先放它，再归还底层句柄
        if let Some(_write) = self.write_port.take() {
            // 显式 take：drop 时的同步 syscall 不留在 async 上下文里
        }
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
        // R3-09：优先用写句柄，它与读侧阻塞读互不竞争。
        // 没有写句柄（`try_clone` 失败）才退回读句柄，由有界抢锁兜底。
        let port_arc = self
            .write_port
            .as_ref()
            .or(self.port.as_ref())
            .ok_or(ProtocolError::ConnectionClosed)?
            .clone();
        let bytes = data.to_vec();

        task::spawn_blocking(move || -> Result<(), std::io::Error> {
            let mut port = lock_for_write(&port_arc)?;
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

        if buf.is_empty() {
            return Ok(0);
        }
        if !self.pending_bytes.is_empty() {
            let count = buf.len().min(self.pending_bytes.len());
            for byte in &mut buf[..count] {
                *byte = self.pending_bytes.pop_front().unwrap();
            }
            return Ok(count);
        }
        if self.read_task.is_none() {
            let port = self
                .port
                .as_ref()
                .ok_or(ProtocolError::ConnectionClosed)?
                .clone();
            let capacity = buf.len();
            self.read_task = Some(task::spawn_blocking(move || {
                let mut bytes = vec![0u8; capacity];
                let result = port.lock().unwrap().read(&mut bytes);
                (result, bytes)
            }));
        }
        // Await by reference: cancelling recv leaves the task owned by this
        // connection, and the next recv consumes the same result exactly once.
        let completed = self
            .read_task
            .as_mut()
            .expect("read task initialized")
            .await;
        self.read_task = None;
        let (result, bytes) =
            completed.map_err(|e| ProtocolError::ProtocolError(format!("join error: {e}")))?;
        match result {
            Ok(n) => {
                let count = n.min(buf.len());
                buf[..count].copy_from_slice(&bytes[..count]);
                self.pending_bytes.extend(&bytes[count..n]);
                Ok(count)
            }
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
    async fn cancelled_recv_preserves_inflight_read_and_bytes() {
        let mut connection = SerialConnection::new(SerialConfig::default());
        connection.state = SerialState::Connected;
        let (ready, wait) = tokio::sync::oneshot::channel();
        connection.read_task = Some(tokio::spawn(async move {
            wait.await.unwrap();
            (Ok(3), b"abc".to_vec())
        }));
        let mut buffer = [0u8; 2];
        assert!(
            tokio::time::timeout(Duration::from_millis(5), connection.recv(&mut buffer))
                .await
                .is_err()
        );
        ready.send(()).unwrap();
        assert_eq!(connection.recv(&mut buffer).await.unwrap(), 2);
        assert_eq!(&buffer, b"ab");
        assert_eq!(connection.recv(&mut buffer).await.unwrap(), 1);
        assert_eq!(buffer[0], b'c');
        assert!(connection.read_task.is_none());
    }

    #[tokio::test]
    async fn test_serial_config_default() {
        let cfg = SerialConfig::default();
        assert_eq!(cfg.baud_rate, 115200);
        assert_eq!(cfg.data_bits, 8);
        assert_eq!(cfg.stop_bits, 1);
        assert_eq!(cfg.parity, SerialParity::None);
        assert_eq!(cfg.flow_control, SerialFlowControl::None);
    }

    #[test]
    fn test_to_serialport_data_bits() {
        assert!(matches!(
            to_serialport_data_bits(5),
            serialport::DataBits::Five
        ));
        assert!(matches!(
            to_serialport_data_bits(6),
            serialport::DataBits::Six
        ));
        assert!(matches!(
            to_serialport_data_bits(7),
            serialport::DataBits::Seven
        ));
        // 任何其他值 (含 0、8、9) 都 fallback 到 8
        assert!(matches!(
            to_serialport_data_bits(0),
            serialport::DataBits::Eight
        ));
        assert!(matches!(
            to_serialport_data_bits(8),
            serialport::DataBits::Eight
        ));
        assert!(matches!(
            to_serialport_data_bits(99),
            serialport::DataBits::Eight
        ));
    }

    #[test]
    fn test_to_serialport_stop_bits() {
        assert!(matches!(
            to_serialport_stop_bits(1),
            serialport::StopBits::One
        ));
        assert!(matches!(
            to_serialport_stop_bits(2),
            serialport::StopBits::Two
        ));
        // 其他值 fallback 到 1
        assert!(matches!(
            to_serialport_stop_bits(0),
            serialport::StopBits::One
        ));
        assert!(matches!(
            to_serialport_stop_bits(3),
            serialport::StopBits::One
        ));
    }

    #[test]
    fn test_to_serialport_parity() {
        assert!(matches!(
            to_serialport_parity(SerialParity::None),
            serialport::Parity::None
        ));
        assert!(matches!(
            to_serialport_parity(SerialParity::Even),
            serialport::Parity::Even
        ));
        assert!(matches!(
            to_serialport_parity(SerialParity::Odd),
            serialport::Parity::Odd
        ));
    }

    #[test]
    fn test_to_serialport_flow() {
        assert!(matches!(
            to_serialport_flow(SerialFlowControl::None),
            serialport::FlowControl::None
        ));
        assert!(matches!(
            to_serialport_flow(SerialFlowControl::Software),
            serialport::FlowControl::Software
        ));
        assert!(matches!(
            to_serialport_flow(SerialFlowControl::Hardware),
            serialport::FlowControl::Hardware
        ));
    }

    #[test]
    fn test_serial_config_clone_preserves_all_fields() {
        let cfg = SerialConfig {
            port: "COM3".to_string(),
            baud_rate: 9600,
            data_bits: 7,
            stop_bits: 2,
            parity: SerialParity::Even,
            flow_control: SerialFlowControl::Hardware,
        };
        let cfg2 = cfg.clone();
        assert_eq!(cfg.port, cfg2.port);
        assert_eq!(cfg.baud_rate, cfg2.baud_rate);
        assert_eq!(cfg.data_bits, cfg2.data_bits);
        assert_eq!(cfg.stop_bits, cfg2.stop_bits);
        assert_eq!(cfg.parity, cfg2.parity);
        assert_eq!(cfg.flow_control, cfg2.flow_control);
    }

    #[tokio::test]
    async fn test_serial_connection_creation() {
        let conn = SerialConnection::new(SerialConfig::default());
        assert_eq!(conn.state, SerialState::Disconnected);
        assert!(conn.port.is_none());
    }

    // ── R3-09：写路径不得排在阻塞读后面 ──

    /// 测试用假串口：记录写入字节，读一律按「超时」返回。
    /// 有了它，「读侧持锁时写侧仍能完成」就能在无硬件的情况下验证——
    /// 持锁不放由 `LockHolder` 负责模拟。
    struct FakeSerialPort {
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl FakeSerialPort {
        fn new(written: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { written }
        }
    }

    impl Read for FakeSerialPort {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            // 与真实串口读超时同款：交回控制权，不当作致命错误
            Err(std::io::ErrorKind::TimedOut.into())
        }
    }

    impl Write for FakeSerialPort {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl SerialPort for FakeSerialPort {
        fn name(&self) -> Option<String> {
            Some("FAKE0".to_string())
        }
        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(115_200)
        }
        fn data_bits(&self) -> serialport::Result<serialport::DataBits> {
            Ok(serialport::DataBits::Eight)
        }
        fn flow_control(&self) -> serialport::Result<serialport::FlowControl> {
            Ok(serialport::FlowControl::None)
        }
        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }
        fn stop_bits(&self) -> serialport::Result<serialport::StopBits> {
            Ok(serialport::StopBits::One)
        }
        fn timeout(&self) -> Duration {
            READ_TIMEOUT
        }
        fn set_baud_rate(&mut self, _baud_rate: u32) -> serialport::Result<()> {
            Ok(())
        }
        fn set_data_bits(&mut self, _data_bits: serialport::DataBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_flow_control(
            &mut self,
            _flow_control: serialport::FlowControl,
        ) -> serialport::Result<()> {
            Ok(())
        }
        fn set_parity(&mut self, _parity: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }
        fn set_stop_bits(&mut self, _stop_bits: serialport::StopBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_timeout(&mut self, _timeout: Duration) -> serialport::Result<()> {
            Ok(())
        }
        fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn clear(&self, _buffer_to_clear: serialport::ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }
        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            Ok(Box::new(FakeSerialPort {
                written: self.written.clone(),
            }))
        }
        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }
        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    /// 持锁直到被显式释放，模拟读者长时间持锁做阻塞读。
    /// 释放走 Condvar 而不是固定 sleep，测试不靠时序碰运气、也不会拖慢。
    struct LockHolder {
        release: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        handle: task::JoinHandle<()>,
    }

    impl LockHolder {
        /// 阻塞到目标锁真的被持有为止，再返回控制器
        fn acquire(port: Arc<Mutex<Box<dyn SerialPort>>>) -> Self {
            let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
            let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let flag = release.clone();
            let handle = task::spawn_blocking(move || {
                let _guard = port.lock().unwrap();
                let _ = acquired_tx.send(());
                let (lock, cvar) = &*flag;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = cvar.wait(released).unwrap();
                }
            });
            acquired_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("持锁线程未能取得锁");
            Self { release, handle }
        }
    }

    impl Drop for LockHolder {
        fn drop(&mut self) {
            {
                let (lock, cvar) = &*self.release;
                *lock.lock().unwrap() = true;
                cvar.notify_all();
            }
            self.handle.abort();
        }
    }

    /// 把假串口装进连接内部使用的 `Arc<Mutex<Box<dyn SerialPort>>>`
    fn fake_port(written: Arc<Mutex<Vec<u8>>>) -> Arc<Mutex<Box<dyn SerialPort>>> {
        Arc::new(Mutex::new(Box::new(FakeSerialPort::new(written))))
    }

    /// R3-09 回归：读侧持锁做阻塞读时，`send` 不得被排在读后面。
    /// 写句柄是读句柄 `try_clone` 出来的同一底层句柄副本，两者互不竞争。
    #[tokio::test]
    async fn send_is_not_queued_behind_a_blocked_read() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let read_side = fake_port(Arc::new(Mutex::new(Vec::new())));

        let mut connection = SerialConnection::new(SerialConfig::default());
        connection.state = SerialState::Connected;
        connection.port = Some(read_side.clone());
        connection.write_port = Some(fake_port(written.clone()));

        // 读者持锁不放，等价于现实里 100ms 读超时后立刻重启读的常态
        let holder = LockHolder::acquire(read_side);

        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(5), connection.send(b"ls\r")).await;
        let elapsed = started.elapsed();

        assert!(outcome.is_ok(), "send 不得挂死在阻塞读后面");
        outcome
            .expect("已超时")
            .expect("有独立写句柄时按键必须发送成功");
        assert!(
            elapsed < WRITE_LOCK_TIMEOUT,
            "按键被阻塞读拖慢：耗时 {elapsed:?}，不得接近上限 {WRITE_LOCK_TIMEOUT:?}"
        );
        assert_eq!(&*written.lock().unwrap(), b"ls\r");
        drop(holder);
    }

    /// R3-09 兜底：没有写句柄时（`try_clone` 失败），`send` 只能有界等待锁，
    /// 到点必须返回可诊断错误，而不是无限期排在阻塞读后面。
    #[tokio::test]
    async fn send_without_write_handle_gives_up_within_the_bound() {
        let read_side = fake_port(Arc::new(Mutex::new(Vec::new())));
        let mut connection = SerialConnection::new(SerialConfig::default());
        connection.state = SerialState::Connected;
        connection.port = Some(read_side.clone());
        connection.write_port = None;

        let holder = LockHolder::acquire(read_side);

        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(5), connection.send(b"x"))
            .await
            .expect("send 必须在有界时间内返回，不得挂在锁上");
        let elapsed = started.elapsed();

        let err = outcome.expect_err("锁一直被读侧占用时不得报告发送成功");
        assert!(
            format!("{err:?}").contains("still locked by a read"),
            "错误必须可诊断，实际: {err:?}"
        );
        assert!(
            elapsed < WRITE_LOCK_TIMEOUT * 2,
            "有界等待应停在 {WRITE_LOCK_TIMEOUT:?} 附近，实际 {elapsed:?}"
        );
        drop(holder);
    }

    /// 写句柄接管后字节语义不变：原样写出，不转义、不改写。
    #[tokio::test]
    async fn send_writes_exact_bytes_to_the_write_handle() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut connection = SerialConnection::new(SerialConfig::default());
        connection.state = SerialState::Connected;
        connection.port = Some(fake_port(Arc::new(Mutex::new(Vec::new()))));
        connection.write_port = Some(fake_port(written.clone()));

        connection.send(b"\x1b[A\r").await.unwrap();
        assert_eq!(&*written.lock().unwrap(), b"\x1b[A\r");
    }
}
