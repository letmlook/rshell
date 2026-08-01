//! SSH 协议实现
//!
//! 提供 SSH 连接、SFTP 文件传输等功能。

pub mod client;
pub mod sftp;

pub use client::{HostKeyDecision, HostKeyDecisionRequest, HostKeyDecisionSink, SshClient};
