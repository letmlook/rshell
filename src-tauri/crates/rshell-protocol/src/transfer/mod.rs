//! R2-T2：传输抽象。
//!
//! 把「远端 vs 本地」的文件 I/O 拼成同一个 [`TransferSink`] 接口，让
//! core 层的传输状态机不必区分方向、只管 staged lifecycle。
//!
//! - [`super::ssh::sftp::RemoteSink`]：远端实现（包 `SftpClient`）
//! - [`local_sink::LocalSink`]：本地实现（包 `tokio::fs`）

pub mod local_sink;

pub use crate::ssh::sftp::{
    CommitOutcome, CommitStrategy, RemoteSink, SftpCapabilities, TransferSink,
};
pub use local_sink::LocalSink;
