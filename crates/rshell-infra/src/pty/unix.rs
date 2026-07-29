//! Unix PTY 实现
//!
//! 使用 POSIX pty API 实现伪终端。
//! 仅在 Unix 平台编译实际实现。

use super::{Pty, PtyError};
use std::io;

#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use tracing::{info, debug};

/// Unix PTY
pub struct UnixPty {
    #[cfg(unix)]
    master: Option<File>,
    #[cfg(unix)]
    slave: Option<File>,
    #[cfg(unix)]
    slave_path: Option<String>,
}

impl UnixPty {
    /// 创建新的 Unix PTY
    pub fn new(rows: u16, cols: u16) -> Result<Self, PtyError> {
        info!("Creating Unix PTY ({}x{})", rows, cols);

        #[cfg(unix)]
        {
            let _ = (rows, cols);
            match Self::open_ptmx() {
                Ok(pty) => {
                    debug!("Unix PTY created via /dev/ptmx");
                    Ok(pty)
                }
                Err(e) => {
                    Err(PtyError::CreationFailed(format!("Failed to create PTY: {}", e)))
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = (rows, cols);
            Err(PtyError::CreationFailed("Unix PTY not supported on this platform".to_string()))
        }
    }

    #[cfg(unix)]
    fn open_ptmx() -> Result<Self, io::Error> {
        let master = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")?;

        let master_fd = master.as_raw_fd();

        unsafe {
            if libc::grantpt(master_fd) != 0 {
                return Err(io::Error::last_os_error());
            }

            if libc::unlockpt(master_fd) != 0 {
                return Err(io::Error::last_os_error());
            }

            let slave_name = libc::ptsname(master_fd);
            if slave_name.is_null() {
                return Err(io::Error::last_os_error());
            }

            let slave_path = std::ffi::CStr::from_ptr(slave_name)
                .to_string_lossy()
                .into_owned();

            let slave = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&slave_path)?;

            debug!("PTY created: master={}, slave={}", master_fd, slave_path);

            Ok(Self {
                master: Some(master),
                slave: Some(slave),
                slave_path: Some(slave_path),
            })
        }
    }

    #[cfg(unix)]
    fn set_window_size(&self, rows: u16, cols: u16) -> io::Result<()> {
        if let Some(ref master) = self.master {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };

            let ret = unsafe {
                libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws)
            };

            if ret != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

impl Pty for UnixPty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            if let Some(ref mut master) = self.master {
                return master.read(buf);
            }
        }
        let _ = buf;
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            if let Some(ref mut master) = self.master {
                master.write(buf)?;
                master.flush()?;
                return Ok(buf.len());
            }
        }
        let _ = buf;
        Ok(0)
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.set_window_size(rows, cols)
        }
        #[cfg(not(unix))]
        {
            let _ = (rows, cols);
            Ok(())
        }
    }
}

impl Drop for UnixPty {
    fn drop(&mut self) {
        debug!("Unix PTY dropped");
    }
}
