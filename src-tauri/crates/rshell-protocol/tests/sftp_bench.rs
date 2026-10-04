//! SFTP 吞吐基准：对着**真实** SSH 服务器测上传/下载速率。
//!
//! 为什么不写在单元测试里：吞吐瓶颈（TCP Nagle、SSH 通道窗口、SFTP 请求并发度、
//! 单次读写分块）只有在真实网络栈和真实 SFTP 服务端上才暴露。仓库内既有测试全部是
//! 内存/回环桩，测不出这些。
//!
//! 用法（凭据只从环境变量读，绝不写进仓库）：
//! ```text
//! RSHELL_BENCH_HOST=192.168.85.129 \
//! RSHELL_BENCH_USER=letmlook \
//! RSHELL_BENCH_PASS=<password> \
//! RSHELL_BENCH_MB=64 \
//! cargo test -p rshell-protocol --test sftp_bench -- --ignored --nocapture
//! ```
//! 可选 `RSHELL_BENCH_PATH` 指定远端落盘目录（默认 `/tmp`）。
//!
//! `#[ignore]` 保证 `cargo test --workspace` 不会因缺少凭据而失败。
//!
//! 输出形如 `upload: 12.34 MB/s (64.0 MiB in 5.19s)`，用于改一处优化前后对比。

use std::sync::Arc;
use std::time::Instant;

use rshell_protocol::ssh::client::{ResolvedAuthMethod, SshClient};
use rshell_protocol::ssh::sftp::{SftpClient, TransferControl};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

/// 基准文件大小（MiB）。默认 64：足够让稳态吞吐压过握手与首包开销，
/// 又不至于让每次迭代等太久。
const DEFAULT_MB: usize = 64;

struct AcceptHost;

impl rshell_protocol::ssh::client::HostKeyDecisionSink for AcceptHost {
    fn register_decision(
        &self,
    ) -> (
        Uuid,
        oneshot::Receiver<rshell_protocol::ssh::client::HostKeyDecision>,
    ) {
        let (tx, rx) = oneshot::channel();
        tx.send(rshell_protocol::ssh::client::HostKeyDecision {
            fingerprint: String::new(),
            key_blob: String::new(),
            // 基准只连用户自己指定的测试机；不落盘 known_hosts，
            // 免得把一次临时授权混进真实信任库。
            accept: true,
            permanent: false,
        })
        .unwrap();
        (Uuid::new_v4(), rx)
    }
    fn publish_request(&self, _: rshell_protocol::ssh::client::HostKeyDecisionRequest) {}
    fn cancel_decision(&self, _: Uuid) {}
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

fn mb_per_sec(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        return f64::INFINITY;
    }
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要真实 SSH 服务器与 RSHELL_BENCH_* 环境变量，不进 CI"]
async fn bench_sftp_upload_and_download() {
    let host = env_or("RSHELL_BENCH_HOST", "");
    let user = env_or("RSHELL_BENCH_USER", "");
    let pass = env_or("RSHELL_BENCH_PASS", "");
    assert!(!host.is_empty(), "缺少 RSHELL_BENCH_HOST");
    assert!(!user.is_empty(), "缺少 RSHELL_BENCH_USER");

    let mb: usize = env_or("RSHELL_BENCH_MB", &DEFAULT_MB.to_string())
        .parse()
        .expect("RSHELL_BENCH_MB 必须是整数");
    let remote_dir = env_or("RSHELL_BENCH_PATH", "/tmp");
    let port: u16 = env_or("RSHELL_BENCH_PORT", "22").parse().expect("端口非法");

    let total = mb * 1024 * 1024;
    // 内容非全零：全零数据可能被服务端或链路层压缩路径特殊对待。
    let payload: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();

    let dir = tempfile::tempdir().expect("临时目录");
    let local_src = dir.path().join("bench-src.bin");
    let local_dst = dir.path().join("bench-dst.bin");
    std::fs::write(&local_src, &payload).expect("写本地源文件");
    let remote_path = format!(
        "{}/rshell-bench-{}.bin",
        remote_dir.trim_end_matches('/'),
        Uuid::new_v4()
    );

    println!("host={host}:{port} user={user} size={mb}MiB remote={remote_path}");

    let mut client = SshClient::new(
        rshell_api::types::SessionConfig {
            id: Uuid::new_v4(),
            name: "bench".into(),
            folder_id: None,
            host: host.clone(),
            port,
            protocol: rshell_api::types::Protocol::SSH,
            auth_method: rshell_api::types::AuthMethod::Password {
                username: user.clone(),
                has_password: true,
            },
            serial_config: None,
        },
        ResolvedAuthMethod::Password {
            username: user.clone(),
            password: pass,
        },
    );
    client
        .connect_ssh(Some(Arc::new(AcceptHost)))
        .await
        .expect("SSH 连接失败");
    println!("connected");

    let channel = client
        .open_sftp_channel()
        .await
        .expect("打开 SFTP 通道失败");
    let sftp = SftpClient::new(channel).await.expect("SFTP 初始化失败");

    // ---- 上传 ----
    let (_ctl_tx, mut ctl_rx) = watch::channel(TransferControl::Run);
    let up_start = Instant::now();
    let uploaded = sftp
        .upload(&local_src, &remote_path, &mut ctl_rx, &mut |_, _| {})
        .await
        .expect("上传失败");
    let up_secs = up_start.elapsed().as_secs_f64();
    println!(
        "upload: {:.2} MB/s ({:.1} MiB in {:.2}s)",
        mb_per_sec(uploaded, up_secs),
        uploaded as f64 / (1024.0 * 1024.0),
        up_secs
    );

    // ---- 下载 ----
    let (_ctl_tx2, mut ctl_rx2) = watch::channel(TransferControl::Run);
    let down_start = Instant::now();
    let downloaded = sftp
        .download(&remote_path, &local_dst, &mut ctl_rx2, &mut |_, _| {})
        .await
        .expect("下载失败");
    let down_secs = down_start.elapsed().as_secs_f64();
    println!(
        "download: {:.2} MB/s ({:.1} MiB in {:.2}s)",
        mb_per_sec(downloaded, down_secs),
        downloaded as f64 / (1024.0 * 1024.0),
        down_secs
    );

    // 落盘内容必须与源逐字节一致：吞吐优化改了读写分块/并发，
    // 错位会写坏文件，这里是最便宜的护栏。
    let back = std::fs::read(&local_dst).expect("读回文件");
    assert_eq!(back.len(), payload.len(), "下载长度与源不一致");
    assert!(back == payload, "下载内容与源不一致（区间错位或丢数据）");

    // 清理远端；失败不影响基准结论，只提示。
    if let Err(e) = sftp.remove_file(&remote_path).await {
        println!("warn: 远端清理失败 {remote_path}: {e}");
    }
    let _ = client.disconnect_ssh().await;
    println!("bench done (content verified identical)");
}
