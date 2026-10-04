//! SSH 传输层吞吐探针：绕过 SFTP，直测 exec 通道的收发速率。
//!
//! 用途：把「SSH 传输层」和「SFTP 层」的瓶颈分开。
//! - 若 exec 通道能跑很高而 SFTP 只有 2.5 MB/s → 瓶颈在 SFTP/协议层；
//! - 若 exec 通道同样只有 2.5 MB/s → 瓶颈在 SSH 传输层或其之下。
//!
//! 读方向用 `dd if=/dev/zero`，不涉及服务端磁盘；
//! 写方向用 `cat > /dev/null`，也不落盘——两者都只测链路与 SSH 封装。
//!
//! 本文件自行建立原生 russh 连接，因此可以扫描传输参数（窗口、包长、密码套件）
//! 而不改动生产代码；测出有效组合后再回填到 `transport_config`。
//!
//! 用法：
//! ```text
//! RSHELL_BENCH_HOST=192.168.85.129 RSHELL_BENCH_USER=letmlook \
//! RSHELL_BENCH_PASS=<password> RSHELL_BENCH_MB=192 \
//! cargo test -p rshell-protocol --test transport_bench -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use russh::client::{connect_stream, Config, Handler};
use russh::keys::ssh_key;

struct Accept;

#[async_trait]
impl Handler for Accept {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

/// 计数读取器：统计底层 socket 的 read 次数与字节数。
///
/// 用途：判断吞吐瓶颈是「每包有界」还是「每字节有界」——
/// 若每包 32 KiB 而 2.7 MB/s，则每秒约 88 次 read；若每包只有 1 KiB，
/// 则每秒约 2700 次。两者对应的修复方向完全不同（放大包 vs 减少逐包开销）。
struct CountingReader<T> {
    inner: T,
    reads: Arc<std::sync::atomic::AtomicU64>,
    bytes: Arc<std::sync::atomic::AtomicU64>,
}

impl<T: tokio::io::AsyncRead + Unpin> CountingReader<T> {
    fn new(
        inner: T,
    ) -> (
        Self,
        Arc<std::sync::atomic::AtomicU64>,
        Arc<std::sync::atomic::AtomicU64>,
    ) {
        let reads = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        (
            Self {
                inner,
                reads: reads.clone(),
                bytes: bytes.clone(),
            },
            reads,
            bytes,
        )
    }
}

impl<T: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for CountingReader<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if r.is_ready() {
            let n = buf.filled().len() - before;
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.bytes
                .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        }
        r
    }
}

// russh 的 connect_stream 要求流同时实现 AsyncRead + AsyncWrite，
// 因此写侧透传即可（只统计读侧）。
impl<T: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for CountingReader<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn mbps(bytes: u64, secs: f64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}

/// 打开一个已 request_exec 的通道，返回收发流。
async fn exec_channel(
    session: &russh::client::Handle<Accept>,
    cmd: &str,
) -> russh::Channel<russh::client::Msg> {
    let ch = session.channel_open_session().await.expect("打开会话通道");
    ch.request_subsystem(false, cmd).await.ok();
    // request_subsystem 只对 sftp 有意义；exec 用 exec_request。
    let _ = ch.exec(false, cmd).await;
    ch
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要真实 SSH 服务器与 RSHELL_BENCH_* 环境变量，不进 CI"]
async fn probe_raw_ssh_transport_throughput() {
    let host = env_or("RSHELL_BENCH_HOST", "");
    let user = env_or("RSHELL_BENCH_USER", "");
    let pass = env_or("RSHELL_BENCH_PASS", "");
    let port: u16 = env_or("RSHELL_BENCH_PORT", "22").parse().expect("端口非法");
    let mb: usize = env_or("RSHELL_BENCH_MB", "192").parse().expect("MB 非法");
    assert!(
        !host.is_empty() && !user.is_empty(),
        "缺少 RSHELL_BENCH_HOST/USER"
    );
    let total = (mb * 1024 * 1024) as u64;

    let window: u32 = env_or("RSHELL_BENCH_WINDOW", "2097152")
        .parse()
        .expect("窗口非法");
    let packet: u32 = env_or("RSHELL_BENCH_PACKET", "32768")
        .parse()
        .expect("包长非法");
    // 密码套件扫描：若吞吐随套件变化，则瓶颈是逐字节加密开销而非协议往返。
    let cipher_pick = env_or("RSHELL_BENCH_CIPHER", "default");
    use russh::cipher;
    use std::borrow::Cow;
    let preferred = russh::Preferred {
        cipher: match cipher_pick.as_str() {
            "aes256ctr" => Cow::Borrowed(&[cipher::AES_256_CTR][..]),
            "aes128ctr" => Cow::Borrowed(&[cipher::AES_128_CTR][..]),
            "aes256gcm" => Cow::Borrowed(&[cipher::AES_256_GCM][..]),
            _ => Cow::Borrowed(&[cipher::CHACHA20_POLY1305][..]),
        },
        ..Default::default()
    };
    let config = Config {
        window_size: window,
        maximum_packet_size: packet,
        preferred,
        ..Default::default()
    };
    println!(
        "host={host}:{port} size={mb}MiB window={window} packet={packet} cipher={cipher_pick}"
    );

    // 与生产代码一致：关闭 Nagle。
    let socket = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("TCP 连接失败");
    socket.set_nodelay(true).expect("set_nodelay");
    let (counting, read_count, read_bytes) = CountingReader::new(socket);
    let mut session = connect_stream(Arc::new(config), counting, Accept)
        .await
        .expect("SSH 握手失败");
    // connect_stream 只做密钥交换；开通道前必须先认证，否则 channel_open_session
    // 会以 Disconnect 失败。
    session
        .authenticate_password(&user, &pass)
        .await
        .expect("SSH 密码认证失败");
    println!("connected+authenticated (nodelay on)");

    // ── 读方向：服务端产数据，不落盘 ──
    let ch = exec_channel(
        &session,
        &format!("dd if=/dev/zero bs=1M count={mb} 2>/dev/null"),
    )
    .await;
    let mut stream = ch.into_stream();
    let start = Instant::now();
    let mut got = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    use tokio::io::AsyncReadExt;
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n as u64,
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "transport-read (dd->stdout): {:.2} MB/s ({:.1} MiB in {:.2}s)",
        mbps(got, secs),
        got as f64 / (1024.0 * 1024.0),
        secs
    );
    let reads = read_count.load(std::sync::atomic::Ordering::Relaxed);
    let rbytes = read_bytes.load(std::sync::atomic::Ordering::Relaxed);
    if reads > 0 {
        println!(
            "socket reads: {reads} ({:.1} reads/s, avg {:.0} bytes/read)",
            reads as f64 / secs,
            rbytes as f64 / reads as f64
        );
    }

    // ── 写方向：cat 丢弃，不落盘 ──
    let ch2 = exec_channel(&session, "cat > /dev/null").await;
    let mut sink = ch2.into_stream();
    let start = Instant::now();
    let payload = vec![7u8; 256 * 1024];
    use tokio::io::AsyncWriteExt;
    let mut written = 0u64;
    let mut err = None;
    while written < total {
        if let Err(e) = sink.write_all(&payload).await {
            err = Some(e);
            break;
        }
        written += payload.len() as u64;
    }
    let secs = start.elapsed().as_secs_f64();
    if let Some(e) = err {
        println!(
            "transport-write aborted after {:.1} MiB: {e}",
            written as f64 / 1048576.0
        );
    } else {
        println!(
            "transport-write (->cat /dev/null): {:.2} MB/s ({:.1} MiB in {:.2}s)",
            mbps(written, secs),
            written as f64 / (1024.0 * 1024.0),
            secs
        );
    }
    let _ = sink.shutdown().await;
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    println!("probe done");
}

/// 通用远程命令执行器：把 `RSHELL_BENCH_CMD` 的输出打到 stdout。
/// 用途：查服务端环境（CPU/磁盘/网络栈），以及给 scp 装公钥做 OpenSSH 对照测量。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要真实 SSH 服务器与 RSHELL_BENCH_* 环境变量，不进 CI"]
async fn run_remote_command() {
    let cmd = env_or("RSHELL_BENCH_CMD", "echo no-command-given");
    let host = env_or("RSHELL_BENCH_HOST", "");
    let user = env_or("RSHELL_BENCH_USER", "");
    let pass = env_or("RSHELL_BENCH_PASS", "");
    let port: u16 = env_or("RSHELL_BENCH_PORT", "22").parse().expect("端口非法");
    assert!(!host.is_empty() && !user.is_empty());

    let socket = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("TCP 连接失败");
    socket.set_nodelay(true).expect("set_nodelay");
    let mut session = connect_stream(Arc::new(Config::default()), socket, Accept)
        .await
        .expect("SSH 握手失败");
    session
        .authenticate_password(&user, &pass)
        .await
        .expect("SSH 密码认证失败");

    let ch = session.channel_open_session().await.expect("开通道");
    ch.exec(true, cmd.as_str()).await.expect("exec");
    let mut stream = ch.into_stream();
    let mut out = Vec::new();
    use tokio::io::AsyncReadExt;
    let _ = stream.read_to_end(&mut out).await;
    print!("{}", String::from_utf8_lossy(&out));
    println!("--- exec done ---");
}
