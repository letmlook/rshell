//! Windows PTY 实现
//!
//! 使用管道创建子进程进行 I/O。
//! 完整的 ConPTY 实现需要 windows crate。

use super::{Pty, PtyError};
use std::io::{self, Read, Write};
use tracing::{info, debug};

/// Windows PTY
pub struct WindowsPty {
    /// 进程句柄
    process: Option<std::process::Child>,
    /// 读取流（从子进程 stdout）
    reader: Option<Box<dyn Read + Send + Sync>>,
    /// 写入流（到子进程 stdin）
    writer: Option<Box<dyn Write + Send + Sync>>,
}

impl WindowsPty {
    /// 创建新的 Windows PTY
    pub fn new(rows: u16, cols: u16) -> Result<Self, PtyError> {
        info!("Creating Windows PTY ({}x{})", rows, cols);

        let mut child = std::process::Command::new("cmd.exe")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| PtyError::CreationFailed(format!("Failed to spawn cmd.exe: {}", e)))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| PtyError::CreationFailed("Failed to take stdout".to_string()))?;
        let stdin = child.stdin.take()
            .ok_or_else(|| PtyError::CreationFailed("Failed to take stdin".to_string()))?;

        let reader: Box<dyn Read + Send + Sync> = Box::new(stdout);
        let writer: Box<dyn Write + Send + Sync> = Box::new(stdin);

        debug!("Windows PTY created successfully");

        Ok(Self {
            process: Some(child),
            reader: Some(reader),
            writer: Some(writer),
        })
    }
}

impl Pty for WindowsPty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(ref mut reader) = self.reader {
            reader.read(buf)
        } else {
            Ok(0)
        }
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(ref mut writer) = self.writer {
            writer.write(buf)?;
            writer.flush()?;
            Ok(buf.len())
        } else {
            Ok(0)
        }
    }

    fn resize(&mut self, _rows: u16, _cols: u16) -> io::Result<()> {
        debug!("PTY resize requested (stub - needs ConPTY API)");
        Ok(())
    }
}

impl Drop for WindowsPty {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
        }
    }
}
