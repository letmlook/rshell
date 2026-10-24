//! 真机端到端验收（CommandDispatcher 级）
//!
//! 界面上的每个可见操作最终都落到 `CommandDispatcher::dispatch(AppCommand)`，
//! 前端与后端之间**没有第二条路径**。因此在 GUI 无法操作时（macOS 锁屏时
//! AX 层对所有应用都返回 `ax_window_unresolved`），直接驱动同一个入口就能
//! 验证界面对应的后端行为 —— 而且验的是**真机**：真实的 multipass Linux
//! 实例、真实的 sshd / telnetd / OpenSSH 端口转发。
//!
//! 覆盖范围（与界面功能一一对应）：
//! - SSH：连接、命令往返、远端目录浏览、主目录查询
//! - Telnet：完整登录握手 + 命令往返
//! - 主机密钥：未知主机的决策流程（拒绝 / 会话内信任 / 永久信任落盘）
//! - 触发器：四种动作（发送文本 / 通知 / 断开 / 写文件）真实执行
//! - 隧道：Local 正向转发 + Dynamic(SOCKS5) 真实转发
//! - 快速命令 / Rhai 脚本 / SSH 密钥全生命周期
//! - 会话多开（独立 pty）与「分组内名称唯一」
//! - 主密码：设置 → 锁定 → 解锁 → 改密 → 停用
//! - 文件传输：上传 / 下载 / 同名冲突
//!
//! 全部 `#[ignore]`：需要真实服务器与 `RSHELL_REAL_*` 环境变量，不进 CI。
//! 与 `crates/rshell-protocol/tests/sftp_bench.rs` 同一范式。
//!
//! 环境变量：
//! - `RSHELL_REAL_HOST`（必填）
//! - `RSHELL_REAL_SSH_PORT`（默认 22）
//! - `RSHELL_REAL_TELNET_PORT`（默认 2323）
//! - `RSHELL_REAL_USER`（默认 ubuntu）
//! - `RSHELL_REAL_PASS`（Telnet 登录口令，必填）
//! - `RSHELL_REAL_KEY`（SSH 私钥路径；默认 `$HOME/.ssh/id_ed25519`）
//! - `RSHELL_REAL_PATH`（远端工作目录；默认 `/home/ubuntu`）

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use rshell_api::credentials::{CredentialError, CredentialKey, CredentialStore};
use rshell_api::types::*;
use rshell_api::{AppCommand, AppEvent, CommandOutcome};
use rshell_core::command_dispatcher::{CommandDispatcher, Services};
use rshell_core::event_bus::EventBus;
use rshell_core::script::trigger_engine::TriggerEngine;
use rshell_core::security::host_key_decision::HostKeyDecisionRegistry;
use rshell_core::security::host_key_manager::HostKeyManager;
use rshell_core::security::key_manager::KeyManager;
use rshell_core::security::master_password::MasterPassword;
use rshell_core::security::tunnel_manager::TunnelManager;
use rshell_core::session::folder_service::FolderService;
use rshell_core::session::repository::SessionRepository;
use rshell_core::session::service::SessionService;
use rshell_core::terminal::service::TerminalService;
use rshell_core::theme::ThemeManager;
use rshell_core::transfer::service::TransferService;
use uuid::Uuid;

// ===== 真机环境 =====

struct RealEnv {
    host: String,
    ssh_port: u16,
    telnet_port: u16,
    user: String,
    pass: String,
    key: PathBuf,
    remote_dir: String,
}

fn real_env() -> RealEnv {
    let host = std::env::var("RSHELL_REAL_HOST").unwrap_or_default();
    assert!(!host.is_empty(), "缺少 RSHELL_REAL_HOST");
    let ssh_port = std::env::var("RSHELL_REAL_SSH_PORT")
        .unwrap_or_else(|_| "22".to_string())
        .parse()
        .expect("RSHELL_REAL_SSH_PORT 非法");
    let telnet_port = std::env::var("RSHELL_REAL_TELNET_PORT")
        .unwrap_or_else(|_| "2323".to_string())
        .parse()
        .expect("RSHELL_REAL_TELNET_PORT 非法");
    let user = std::env::var("RSHELL_REAL_USER").unwrap_or_else(|_| "ubuntu".to_string());
    let pass = std::env::var("RSHELL_REAL_PASS").unwrap_or_default();
    assert!(!pass.is_empty(), "缺少 RSHELL_REAL_PASS（Telnet 登录需要）");
    let key = match std::env::var("RSHELL_REAL_KEY") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".ssh/id_ed25519"),
    };
    let remote_dir =
        std::env::var("RSHELL_REAL_PATH").unwrap_or_else(|_| "/home/ubuntu".to_string());
    RealEnv {
        host,
        ssh_port,
        telnet_port,
        user,
        pass,
        key,
        remote_dir,
    }
}

// ===== 隔离：把 HOME 指向临时目录 =====
//
// `CommandDispatcher::new` 内部自己算数据目录（`dirs::data_local_dir()/rshell`），
// 快速命令服务与插件加载器都落在这里。真机用例不应该污染用户真实的
// `~/Library/Application Support/rshell/`（正在运行的 dev app 也在用那个目录）。
// `dirs` 在 macOS 上遵循 `$HOME`，所以改 HOME 就能整体挪走。
//
// 顺带解决第二个问题：SSH 客户端查找 known_hosts 的路径也是
// `dirs::data_local_dir()/rshell/known_hosts`（见 rshell-protocol 的
// `build_known_hosts_paths`），与 `HostKeyManager` 写的路径是同一个 ——
// 所以「永久信任」写进去的条目，客户端下一次握手真的读得到。

fn isolate_home() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let base =
            std::env::temp_dir().join(format!("rshell-real-machine-home-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("创建隔离 HOME 失败");
        std::env::set_var("HOME", &base);
    });
}

/// 应用自己的数据根目录（与 Tauri 壳层 `data_root()` 完全一致）。
fn app_data_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rshell")
}

/// 同一进程内的真机用例串行执行。
///
/// 这些用例共享应用的 known_hosts（必须与 SSH 客户端读的是同一个文件），
/// 并行跑会互相干扰（一个用例的「永久信任」会让另一个用例等不到决策）。
/// 用 `tokio::sync::Mutex` 而不是 `std::sync::Mutex`：后者把 guard 跨 `.await`
/// 持有会触发 clippy 的 `await_holding_lock`，且在异步上下文里并不安全。
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ===== 凭据存储（测试内的内存实现） =====
//
// `rshell-infra` 的内存后端是 `#[cfg(test)]` 私有实现，集成测试拿不到；
// 这里按 `rshell_api::credentials::CredentialStore` 契约自己实现一份，
// 不碰系统钥匙串（真机用例不该在用户钥匙串里留条目）。

#[derive(Default)]
struct MemoryCredentialStore {
    entries: Mutex<HashMap<String, String>>,
}

impl CredentialStore for MemoryCredentialStore {
    fn get(&self, key: &CredentialKey) -> Result<Option<String>, CredentialError> {
        Ok(self.entries.lock().unwrap().get(&key.account()).cloned())
    }
    fn set(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialError> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.account(), secret.to_string());
        Ok(())
    }
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        self.entries.lock().unwrap().remove(&key.account());
        Ok(())
    }
}

// ===== 事件记录 =====

#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<AppEvent>>>);

impl EventLog {
    fn record(&self, event: &AppEvent) {
        self.0.lock().unwrap().push(event.clone());
    }

    fn snapshot(&self) -> Vec<AppEvent> {
        self.0.lock().unwrap().clone()
    }

    /// 等到第一个满足条件的事件出现。
    async fn wait_for<F>(&self, predicate: F, timeout: Duration) -> Option<AppEvent>
    where
        F: Fn(&AppEvent) -> bool,
    {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(found) = self.snapshot().into_iter().find(&predicate) {
                return Some(found);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }
}

// ===== 终端输出收集 =====

#[derive(Clone, Default)]
struct OutputLog(Arc<Mutex<HashMap<Uuid, Vec<u8>>>>);

impl OutputLog {
    fn append(&self, terminal_id: Uuid, data: Vec<u8>) {
        self.0
            .lock()
            .unwrap()
            .entry(terminal_id)
            .or_default()
            .extend_from_slice(&data);
    }

    fn text(&self, terminal_id: Uuid) -> String {
        let bytes = self.0.lock().unwrap();
        bytes
            .get(&terminal_id)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default()
    }

    /// 等到某个 pty 的输出里出现 `needle`。
    async fn wait_for_text(
        &self,
        terminal_id: Uuid,
        needle: &str,
        timeout: Duration,
    ) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let text = self.text(terminal_id);
            if text.contains(needle) {
                return Some(text);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }
}

// ===== 测试台 =====

struct Harness {
    dispatcher: Arc<CommandDispatcher>,
    events: EventLog,
    output: OutputLog,
    root: PathBuf,
    known_hosts_path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// 默认测试台：未知主机自动按「始终信任」回应。
    ///
    /// 真实客户端遇到未知主机会弹窗等用户决策；命令行用例没人点，所以
    /// 挂一个后台应答任务，行为等价于用户点了「始终信任」。
    fn new() -> Self {
        Self::with_host_key_auto_accept(true)
    }

    /// 主机密钥决策流程专用：关掉自动应答，由用例自己决策。
    fn with_host_key_auto_accept(auto_accept_host_key: bool) -> Self {
        isolate_home();
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("keys")).expect("创建密钥目录失败");

        let event_bus = Arc::new(EventBus::new());
        let events = EventLog::default();
        let sink = events.clone();
        // 主机密钥不匹配单独走一条通道：自动应答任务据此立刻回应握手，
        // 否则 connect 会一直挂在 oneshot 上（服务端有超时，但用例等太久）。
        let (mismatch_tx, mut mismatch_rx) = tokio::sync::mpsc::unbounded_channel::<Uuid>();
        event_bus.subscribe(move |event| {
            sink.record(event);
            if let AppEvent::HostKeyMismatch { decision_id, .. } = event {
                let _ = mismatch_tx.send(*decision_id);
            }
        });

        let terminal_service = Arc::new(TerminalService::new(event_bus.clone()));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Uuid, Uuid, Vec<u8>)>();
        terminal_service.set_output_sender(tx);
        let output = OutputLog::default();
        let collector = output.clone();
        tokio::spawn(async move {
            while let Some((_session_id, terminal_id, data)) = rx.recv().await {
                collector.append(terminal_id, data);
            }
        });

        let trigger_engine = Arc::new(TriggerEngine::with_path(
            event_bus.clone(),
            root.join("triggers.json"),
        ));
        let host_key_registry = Arc::new(HostKeyDecisionRegistry::new(event_bus.clone()));
        let key_manager = Arc::new(KeyManager::new(root.join("keys"), event_bus.clone()));
        let repository = Arc::new(SessionRepository::new(
            root.join("sessions.json"),
            Arc::new(MemoryCredentialStore::default()),
        ));
        let session_service = Arc::new(SessionService::with_repository(
            event_bus.clone(),
            terminal_service.clone(),
            trigger_engine.clone(),
            host_key_registry.clone(),
            Some(repository),
            Some(key_manager.clone()),
        ));
        let transfer_service = Arc::new(TransferService::new(event_bus.clone()));
        let master_password = Arc::new(MasterPassword::with_path(
            event_bus.clone(),
            root.join("master_password.json"),
        ));
        let folder_service = Arc::new(FolderService::with_path(
            event_bus.clone(),
            root.join("folders.json"),
        ));
        let tunnel_manager = Arc::new(
            TunnelManager::new(event_bus.clone()).with_persistence(root.join("tunnels.toml")),
        );
        // known_hosts 必须与 SSH 客户端读的是同一个文件（应用数据根下的那一份），
        // 否则「永久信任」写进去的条目客户端根本看不到。
        let known_hosts_path = app_data_root().join("known_hosts");
        std::fs::create_dir_all(app_data_root()).expect("创建应用数据目录失败");
        let host_key_manager = Arc::new(HostKeyManager::new(known_hosts_path.clone()));
        let theme_manager = Arc::new(ThemeManager::with_persistence(
            event_bus.clone(),
            root.join("color_schemes.json"),
        ));

        let dispatcher = Arc::new(CommandDispatcher::new(Services {
            session_service,
            terminal_service,
            transfer_service,
            trigger_engine,
            key_manager,
            master_password,
            folder_service,
            tunnel_manager,
            host_key_manager,
            theme_manager,
            event_bus,
            host_key_registry,
        }));
        dispatcher.initialize();

        if auto_accept_host_key {
            // 模拟用户在弹窗里点「始终信任」：自动回应未知主机。
            let responder = dispatcher.clone();
            tokio::spawn(async move {
                while let Some(decision_id) = mismatch_rx.recv().await {
                    let _ = responder
                        .dispatch(AppCommand::DecideHostKey {
                            decision_id,
                            accept: true,
                            permanent: true,
                        })
                        .await;
                }
            });
        }

        Self {
            dispatcher,
            events,
            output,
            root,
            known_hosts_path,
            _dir: dir,
        }
    }

    async fn dispatch(
        &self,
        command: AppCommand,
    ) -> Result<CommandOutcome, rshell_core::CoreError> {
        self.dispatcher.dispatch(command).await
    }

    /// 建一条 SSH 会话（公钥认证，无口令短语）。
    async fn create_ssh_session(&self, name: &str) -> Uuid {
        let env = real_env();
        self.create_ssh_session_in(name, None, &env).await
    }

    async fn create_ssh_session_in(
        &self,
        name: &str,
        folder_id: Option<Uuid>,
        env: &RealEnv,
    ) -> Uuid {
        self.try_create_ssh_session_in(name, folder_id, env)
            .await
            .expect("创建 SSH 会话失败")
    }

    async fn try_create_ssh_session_in(
        &self,
        name: &str,
        folder_id: Option<Uuid>,
        env: &RealEnv,
    ) -> Result<Uuid, rshell_core::CoreError> {
        let config = SessionConfig {
            id: Uuid::new_v4(),
            name: name.to_string(),
            folder_id,
            host: env.host.clone(),
            port: env.ssh_port,
            protocol: Protocol::SSH,
            auth_method: AuthMethod::PublicKey {
                username: env.user.clone(),
                key_path: env.key.clone(),
                has_passphrase: false,
                managed_key_id: None,
            },
            serial_config: None,
        };
        match self
            .dispatch(AppCommand::CreateSession {
                config,
                credential: None,
            })
            .await?
        {
            CommandOutcome::SessionId(id) => Ok(id),
            other => panic!("CreateSession 返回了意外的结果: {other:?}"),
        }
    }

    async fn connect(&self, session_id: Uuid) {
        self.dispatch(AppCommand::ConnectSession { session_id })
            .await
            .expect("连接失败");
    }

    /// 已连接会话的主 pty（terminal_id == session_id，与壳层一致）。
    fn primary(&self, session_id: Uuid) -> Uuid {
        session_id
    }

    async fn send(&self, session_id: Uuid, terminal_id: Option<Uuid>, text: &str) {
        self.dispatch(AppCommand::SendInput {
            session_id,
            terminal_id,
            data: text.as_bytes().to_vec(),
        })
        .await
        .expect("发送输入失败");
    }

    async fn connected_event(&self, session_id: Uuid, timeout: Duration) {
        let found = self
            .events
            .wait_for(
                |event| {
                    matches!(
                        event,
                        AppEvent::ConnectionStateChanged { session_id: sid, state: ConnectionState::Connected, .. }
                            if *sid == session_id
                    )
                },
                timeout,
            )
            .await;
        assert!(
            found.is_some(),
            "没有收到 Connected 事件（session {session_id}）"
        );
    }

    /// 已经出现过的「主机密钥不匹配」决策 id（按发生顺序）。
    fn mismatch_ids(&self) -> Vec<Uuid> {
        self.events
            .snapshot()
            .into_iter()
            .filter_map(|event| match event {
                AppEvent::HostKeyMismatch { decision_id, .. } => Some(decision_id),
                _ => None,
            })
            .collect()
    }

    /// 已经完成过的传输任务数（按完成事件计数）。
    fn transfer_completed_count(&self) -> usize {
        self.events
            .snapshot()
            .into_iter()
            .filter(|event| matches!(event, AppEvent::TransferCompleted { .. }))
            .count()
    }

    /// 等到第 `seen + 1` 次传输完成。
    async fn wait_for_transfer_completion(&self, seen: usize, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if self.transfer_completed_count() > seen {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("等待第 {} 次传输完成超时", seen + 1);
    }

    /// 等到**下一个**新的不匹配决策（跳过已经出现过的）。
    async fn wait_for_new_mismatch(&self, seen: usize, timeout: Duration) -> Uuid {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let ids = self.mismatch_ids();
            if ids.len() > seen {
                return ids[seen];
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("等待第 {} 个主机密钥不匹配事件超时", seen + 1);
    }
}

// ===== 工具 =====

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定临时端口失败");
    let port = listener.local_addr().expect("读取本地地址失败").port();
    drop(listener);
    port
}

async fn read_ssh_banner(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(15), stream.read(&mut buf))
        .await
        .expect("读取 SSH banner 超时")
        .expect("读取 SSH banner 失败");
    String::from_utf8_lossy(&buf[..n]).to_string()
}

/// 等到本地文件出现且非空。
async fn wait_local_file(path: &std::path::Path, timeout: Duration) -> Option<String> {
    tokio::time::timeout(timeout, async {
        loop {
            if let Ok(content) = std::fs::read_to_string(path) {
                if !content.is_empty() {
                    return content;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .ok()
}

/// 清理远端测试目录。
///
/// 应用侧**刻意**只允许删除远端普通文件（`DeleteRemoteEntry` 明确拒绝目录，
/// 避免误删整棵树），所以目录的清理只能在应用之外做 —— 与仓库既有的真机
/// 用例同样用 `ssh` 收尾。
fn ssh_cleanup_dir(env: &RealEnv, path: &str) {
    let output = std::process::Command::new("ssh")
        .args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-i",
            &env.key.to_string_lossy(),
            &format!("{}@{}", env.user, env.host),
            &format!("rm -rf {path}"),
        ])
        .output();
    match output {
        Ok(result) => assert!(
            result.status.success(),
            "清理远端目录 {path} 失败: {}",
            String::from_utf8_lossy(&result.stderr)
        ),
        Err(error) => {
            // 没装 ssh 客户端时不该让用例失败，但必须留下痕迹。
            eprintln!("警告：无法调用 ssh 清理 {path}（{error}）");
        }
    }
}

// ===== 1. SSH：连接 / 命令往返 / 远端目录 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_ssh_session_runs_commands_and_browses_remote() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let session_id = h.create_ssh_session("ssh-roundtrip").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    // 命令往返：远端真的执行并回显
    h.send(session_id, None, "echo RSHELL_REAL_SSH_OK_42\n")
        .await;
    let text = h
        .output
        .wait_for_text(
            h.primary(session_id),
            "RSHELL_REAL_SSH_OK_42",
            Duration::from_secs(20),
        )
        .await
        .expect("没有收到远端命令回显");
    assert!(text.contains("RSHELL_REAL_SSH_OK_42"));

    // 远端主目录
    let home = match h
        .dispatch(AppCommand::GetRemoteHomeDir { session_id })
        .await
        .expect("查询远端主目录失败")
    {
        CommandOutcome::RemoteHomeDir(path) => path,
        other => panic!("GetRemoteHomeDir 返回了意外结果: {other:?}"),
    };
    assert!(home.starts_with('/'), "远端主目录不是绝对路径: {home}");

    // 浏览远端目录：必须真的列出了条目
    let (path, entries) = match h
        .dispatch(AppCommand::BrowseRemoteDir {
            session_id,
            path: home.clone(),
        })
        .await
        .expect("浏览远端目录失败")
    {
        CommandOutcome::RemoteDir { path, entries } => (path, entries),
        other => panic!("BrowseRemoteDir 返回了意外结果: {other:?}"),
    };
    assert_eq!(path, home);
    assert!(
        !entries.is_empty(),
        "远端主目录条目为空（{home}）——浏览其实没生效"
    );

    // 幂等 connect：已连接时再连一次必须成功且不报错
    h.dispatch(AppCommand::ConnectSession { session_id })
        .await
        .expect("对已连接会话重复 connect 必须幂等成功");
}

// ===== 2. Telnet：登录握手 + 命令往返 =====

#[tokio::test]
#[ignore = "需要真实 telnetd 与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_telnet_session_logs_in_and_runs_commands() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let env = real_env();

    let config = SessionConfig {
        id: Uuid::new_v4(),
        name: "telnet-login".to_string(),
        folder_id: None,
        host: env.host.clone(),
        port: env.telnet_port,
        protocol: Protocol::Telnet,
        // Telnet 的用户名/口令是在 pty 里交互输入的（telnetd 的登录提示），
        // 后端不参与认证，所以这里不带凭据。
        auth_method: AuthMethod::Password {
            username: env.user.clone(),
            has_password: false,
        },
        serial_config: None,
    };
    let session_id = match h
        .dispatch(AppCommand::CreateSession {
            config,
            credential: None,
        })
        .await
        .expect("创建 Telnet 会话失败")
    {
        CommandOutcome::SessionId(id) => id,
        other => panic!("CreateSession 返回了意外结果: {other:?}"),
    };

    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    let pty = h.primary(session_id);
    // 登录握手：用户名 → 口令 → shell 提示符
    h.output
        .wait_for_text(pty, "login:", Duration::from_secs(20))
        .await
        .expect("没有等到 telnetd 的 login 提示");
    h.send(session_id, None, &format!("{}\n", env.user)).await;
    h.output
        .wait_for_text(pty, "assword:", Duration::from_secs(20))
        .await
        .expect("没有等到 telnetd 的口令提示");
    h.send(session_id, None, &format!("{}\n", env.pass)).await;
    h.output
        .wait_for_text(pty, "$", Duration::from_secs(20))
        .await
        .expect("没有等到 telnet 登录成功的 shell 提示符");

    // 登录之后命令必须真的能跑
    h.send(session_id, None, "echo RSHELL_REAL_TELNET_OK_42\n")
        .await;
    let text = h
        .output
        .wait_for_text(pty, "RSHELL_REAL_TELNET_OK_42", Duration::from_secs(20))
        .await
        .expect("Telnet 会话没有回显命令输出");
    assert!(text.contains("RSHELL_REAL_TELNET_OK_42"));
}

// ===== 3. 主机密钥：未知主机的决策流程 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_unknown_host_key_requires_decision_and_permanent_trust_persists() {
    let _guard = TEST_LOCK.lock().await;
    // 关掉自动应答：这个用例要自己走完「拒绝 → 永久信任 → 不再询问」的全过程。
    let h = Harness::with_host_key_auto_accept(false);
    let env = real_env();
    let known_hosts = h.known_hosts_path.clone();

    // 先把之前用例可能留下的信任条目删掉，制造「未知主机」的前提。
    let _ = h
        .dispatch(AppCommand::DeleteHostKey {
            host: env.host.clone(),
            port: env.ssh_port,
        })
        .await;

    // ── 第一次连接：未知主机 → 握手挂起等决策 ──
    let session_id = h.create_ssh_session("hostkey-first").await;
    let dispatcher = h.dispatcher.clone();
    let connect = tokio::spawn(async move {
        dispatcher
            .dispatch(AppCommand::ConnectSession { session_id })
            .await
    });

    let decision_id = h.wait_for_new_mismatch(0, Duration::from_secs(30)).await;
    assert_ne!(decision_id, Uuid::nil(), "不匹配事件必须带决策 id");

    // ── 拒绝：连接必须失败，且 known_hosts 不得留下任何条目 ──
    h.dispatch(AppCommand::DecideHostKey {
        decision_id,
        accept: false,
        permanent: false,
    })
    .await
    .expect("拒绝决策本身不应报错");
    let rejected = connect
        .await
        .expect("连接任务异常")
        .expect_err("拒绝后连接必须失败");
    assert!(
        format!("{rejected}").to_lowercase().contains("host")
            || format!("{rejected}").to_lowercase().contains("key"),
        "拒绝后的错误信息应当指向主机密钥: {rejected}"
    );
    let known_after_reject = std::fs::read_to_string(&known_hosts).unwrap_or_default();
    assert!(
        !known_after_reject.contains(&env.host),
        "拒绝决策不得写入 known_hosts: {known_after_reject}"
    );

    // ── 第二次连接：永久信任 → 连接成功且落盘 ──
    let session_id = h.create_ssh_session("hostkey-second").await;
    let dispatcher = h.dispatcher.clone();
    let connect = tokio::spawn(async move {
        dispatcher
            .dispatch(AppCommand::ConnectSession { session_id })
            .await
    });
    let decision_id = h.wait_for_new_mismatch(1, Duration::from_secs(30)).await;
    h.dispatch(AppCommand::DecideHostKey {
        decision_id,
        accept: true,
        permanent: true,
    })
    .await
    .expect("永久信任决策失败");
    connect
        .await
        .expect("连接任务异常")
        .expect("永久信任后连接必须成功");

    let known = std::fs::read_to_string(&known_hosts).unwrap_or_default();
    assert!(
        known.contains(&env.host),
        "永久信任必须写入 known_hosts: {known}"
    );

    // ── 第三次连接：主机已信任 → 不再要求决策 ──
    let mismatches_before = h.mismatch_ids().len();
    let session_id = h.create_ssh_session("hostkey-third").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;
    assert_eq!(
        h.mismatch_ids().len(),
        mismatches_before,
        "已信任的主机不应再次要求决策"
    );

    // 界面上的「主机密钥列表」必须能看到这条
    let entries = match h
        .dispatch(AppCommand::ListHostKeys)
        .await
        .expect("列出主机密钥失败")
    {
        CommandOutcome::HostKeys(entries) => entries,
        other => panic!("ListHostKeys 返回了意外结果: {other:?}"),
    };
    assert!(
        entries.iter().any(|entry| entry.host == env.host),
        "主机密钥列表里没有刚信任的主机: {entries:?}"
    );

    // 删除条目
    h.dispatch(AppCommand::DeleteHostKey {
        host: env.host.clone(),
        port: env.ssh_port,
    })
    .await
    .expect("删除主机密钥失败");
    let entries = match h.dispatch(AppCommand::ListHostKeys).await.unwrap() {
        CommandOutcome::HostKeys(entries) => entries,
        other => panic!("ListHostKeys 返回了意外结果: {other:?}"),
    };
    assert!(
        !entries.iter().any(|entry| entry.host == env.host),
        "删除后列表里不应再有该主机"
    );
}

// ===== 4. 触发器：四种动作 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_trigger_executes_all_four_action_kinds() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();

    // ── 动作一：发送文本 ──
    let session_id = h.create_ssh_session("trigger-send").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    let trigger_id = Uuid::new_v4();
    h.dispatch(AppCommand::CreateTrigger {
        trigger: Trigger {
            id: trigger_id,
            name: "send-text".to_string(),
            enabled: true,
            condition: TriggerCondition::RegexAppear("TRIGGER_SEND_HIT".to_string()),
            action: TriggerAction::SendText("echo TRIGGER_SEND_REPLIED\n".to_string()),
        },
    })
    .await
    .expect("创建触发器失败");

    h.send(session_id, None, "echo TRIGGER_SEND_HIT\n").await;
    let fired = h
        .events
        .wait_for(
            |event| matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == trigger_id),
            Duration::from_secs(20),
        )
        .await
        .expect("发送文本触发器没有触发");
    match fired {
        AppEvent::TriggerFired { action_summary, .. } => {
            assert!(
                action_summary.starts_with("send_text("),
                "触发摘要应是 send_text: {action_summary}"
            );
        }
        _ => unreachable!(),
    }
    // 触发后送出的文本必须真的到了远端（远端回显它）
    let text = h
        .output
        .wait_for_text(
            h.primary(session_id),
            "TRIGGER_SEND_REPLIED",
            Duration::from_secs(20),
        )
        .await
        .expect("触发器发送的文本没有到达远端");
    assert!(text.contains("TRIGGER_SEND_REPLIED"));

    // ── 动作二：显示通知 ──
    let notify_trigger = Uuid::new_v4();
    h.dispatch(AppCommand::CreateTrigger {
        trigger: Trigger {
            id: notify_trigger,
            name: "notify".to_string(),
            enabled: true,
            condition: TriggerCondition::RegexAppear("TRIGGER_NOTIFY_HIT".to_string()),
            action: TriggerAction::ShowNotification("RSHELL_TRIGGER_NOTIFIED".to_string()),
        },
    })
    .await
    .expect("创建通知触发器失败");
    h.send(session_id, None, "echo TRIGGER_NOTIFY_HIT\n").await;
    let fired = h
        .events
        .wait_for(
            |event| matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == notify_trigger),
            Duration::from_secs(20),
        )
        .await
        .expect("通知触发器没有触发");
    match fired {
        AppEvent::TriggerFired { action_summary, .. } => assert_eq!(
            action_summary, "notify: RSHELL_TRIGGER_NOTIFIED",
            "通知摘要应带上通知内容"
        ),
        _ => unreachable!(),
    }

    // ── 动作三：写文件 ──
    let log_path = h.root.join("trigger.log");
    let log_trigger = Uuid::new_v4();
    h.dispatch(AppCommand::CreateTrigger {
        trigger: Trigger {
            id: log_trigger,
            name: "log-to-file".to_string(),
            enabled: true,
            condition: TriggerCondition::RegexAppear("TRIGGER_LOG_HIT".to_string()),
            action: TriggerAction::LogToFile(log_path.clone()),
        },
    })
    .await
    .expect("创建写文件触发器失败");
    h.send(session_id, None, "echo TRIGGER_LOG_HIT\n").await;
    h.events
        .wait_for(
            |event| matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == log_trigger),
            Duration::from_secs(20),
        )
        .await
        .expect("写文件触发器没有触发");
    let logged = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(content) = std::fs::read_to_string(&log_path) {
                if content.contains("TRIGGER_LOG_HIT") {
                    return content;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("触发日志文件没有写出内容");
    assert!(logged.contains("TRIGGER_LOG_HIT"));

    // ── 动作四：断开连接 ──
    let disconnect_trigger = Uuid::new_v4();
    h.dispatch(AppCommand::CreateTrigger {
        trigger: Trigger {
            id: disconnect_trigger,
            name: "disconnect".to_string(),
            enabled: true,
            condition: TriggerCondition::RegexAppear("TRIGGER_DISCONNECT_HIT".to_string()),
            action: TriggerAction::Disconnect,
        },
    })
    .await
    .expect("创建断开触发器失败");
    h.send(session_id, None, "echo TRIGGER_DISCONNECT_HIT\n")
        .await;
    h.events
        .wait_for(
            |event| matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == disconnect_trigger),
            Duration::from_secs(20),
        )
        .await
        .expect("断开触发器没有触发");
    h.events
        .wait_for(
            |event| {
                matches!(
                    event,
                    AppEvent::ConnectionStateChanged { session_id: sid, state: ConnectionState::Disconnected, .. }
                        if *sid == session_id
                )
            },
            Duration::from_secs(20),
        )
        .await
        .expect("断开动作之后会话没有变成 Disconnected");

    // 触发器列表 / 切换 / 删除
    let triggers = match h.dispatch(AppCommand::ListTriggers).await.unwrap() {
        CommandOutcome::Triggers(triggers) => triggers,
        other => panic!("ListTriggers 返回了意外结果: {other:?}"),
    };
    assert!(triggers.iter().any(|t| t.id == disconnect_trigger));
    h.dispatch(AppCommand::ToggleTrigger {
        trigger_id: disconnect_trigger,
    })
    .await
    .expect("切换触发器失败");
    let triggers = match h.dispatch(AppCommand::ListTriggers).await.unwrap() {
        CommandOutcome::Triggers(triggers) => triggers,
        other => panic!("ListTriggers 返回了意外结果: {other:?}"),
    };
    let toggled = triggers
        .iter()
        .find(|t| t.id == disconnect_trigger)
        .expect("切换后触发器仍在列表里");
    assert!(!toggled.enabled, "ToggleTrigger 之后应为禁用");
    h.dispatch(AppCommand::DeleteTrigger {
        trigger_id: disconnect_trigger,
    })
    .await
    .expect("删除触发器失败");
    let triggers = match h.dispatch(AppCommand::ListTriggers).await.unwrap() {
        CommandOutcome::Triggers(triggers) => triggers,
        other => panic!("ListTriggers 返回了意外结果: {other:?}"),
    };
    assert!(!triggers.iter().any(|t| t.id == disconnect_trigger));
}

/// 自触发防护（真机）：动作文本的回显若再次匹配条件，不得形成循环
///
/// 复审发现：自触发冷却只加在 SSH 的输出处理里，Telnet/Serial 的协议任务
/// 同样执行 `SendText` 动作却从不进入冷却 —— 那条自触发触发器在
/// Telnet/Serial 上照样无限刷屏。
///
/// 这里用一个**静态校验放行、运行期却会自匹配**的触发器复现这个闭环：
/// 条件 `SELF_LOOP\r` 匹配的是终端**回显**（带 `\r`），而动作文本
/// `echo SELF_LOOP` 本身不含 `\r` → 创建时不被拒；写出去的文本被回显 →
/// 回显再次匹配条件 → 没有冷却就是无限自触发。
#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_self_trigger_does_not_loop() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();

    let session_id = h.create_ssh_session("self-loop").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    let trigger_id = Uuid::new_v4();
    h.dispatch(AppCommand::CreateTrigger {
        trigger: Trigger {
            id: trigger_id,
            name: "self-loop".to_string(),
            enabled: true,
            condition: TriggerCondition::RegexAppear("SELF_LOOP\\r".to_string()),
            action: TriggerAction::SendText("echo SELF_LOOP\n".to_string()),
        },
    })
    .await
    .expect("创建触发器失败");

    h.send(session_id, None, "echo SELF_LOOP\n").await;

    // 至少触发一次（动作真的执行了）
    h.events
        .wait_for(
            |event| {
                matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == trigger_id)
            },
            Duration::from_secs(20),
        )
        .await
        .expect("触发器没有触发");

    // 给自触发闭环留出时间后统计触发次数：必须有限
    // （没有冷却时这里会是几百上千次）
    tokio::time::sleep(Duration::from_secs(5)).await;
    let fired = h
        .events
        .snapshot()
        .iter()
        .filter(|event| {
            matches!(event, AppEvent::TriggerFired { trigger_id: tid, .. } if *tid == trigger_id)
        })
        .count();
    assert!(fired <= 2, "触发器自触发成环了：5 秒内触发了 {fired} 次");

    h.dispatch(AppCommand::DeleteTrigger { trigger_id })
        .await
        .expect("删除触发器失败");
}

// ===== 5. 隧道：Local + Dynamic =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_local_and_dynamic_tunnels_forward_real_traffic() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let session_id = h.create_ssh_session("tunnel").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    // ── Local：本地端口 → 远端 sshd ──
    let local_port = free_port();
    h.dispatch(AppCommand::CreateTunnel {
        session_id,
        rule: PortForwardRule {
            bind_address: "127.0.0.1".to_string(),
            bind_port: local_port,
            remote_host: "localhost".to_string(),
            remote_port: 22,
            direction: ForwardDirection::Local,
            allow_non_loopback: false,
        },
    })
    .await
    .expect("创建 Local 隧道失败");

    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", local_port)),
    )
    .await
    .expect("连接 Local 隧道超时")
    .expect("连接 Local 隧道失败");
    let banner = read_ssh_banner(&mut stream).await;
    assert!(
        banner.starts_with("SSH-"),
        "Local 隧道没有转发到 sshd，收到的是: {banner:?}"
    );

    // ── Dynamic：SOCKS5 代理 ──
    let socks_port = free_port();
    h.dispatch(AppCommand::CreateTunnel {
        session_id,
        rule: PortForwardRule {
            bind_address: "127.0.0.1".to_string(),
            bind_port: socks_port,
            remote_host: String::new(),
            remote_port: 0,
            direction: ForwardDirection::Dynamic,
            allow_non_loopback: false,
        },
    })
    .await
    .expect("创建 Dynamic 隧道失败");

    let mut proxy = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", socks_port)),
    )
    .await
    .expect("连接 SOCKS5 代理超时")
    .expect("连接 SOCKS5 代理失败");
    {
        use tokio::io::AsyncWriteExt;
        // greeting：VER=5, 1 种方法, 无认证
        proxy
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .expect("写 greeting 失败");
        // CONNECT 127.0.0.1:22
        proxy
            .write_all(&[0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, 0x00, 0x16])
            .await
            .expect("写 CONNECT 失败");
    }
    {
        use tokio::io::AsyncReadExt;
        // 先读服务端选方法的两字节应答（05 00 = 无认证）
        let mut method = [0u8; 2];
        tokio::time::timeout(Duration::from_secs(15), proxy.read_exact(&mut method))
            .await
            .expect("SOCKS5 方法应答超时")
            .expect("SOCKS5 方法应答读取失败");
        assert_eq!(method[0], 0x05, "SOCKS5 版本字节不对: {method:?}");
        assert_eq!(method[1], 0x00, "SOCKS5 应选择无认证: {method:?}");
        // 再读 CONNECT 的十字节应答
        let mut reply = [0u8; 10];
        tokio::time::timeout(Duration::from_secs(15), proxy.read_exact(&mut reply))
            .await
            .expect("SOCKS5 应答超时")
            .expect("SOCKS5 应答读取失败");
        assert_eq!(reply[0], 0x05, "SOCKS5 版本字节不对: {reply:?}");
        assert_eq!(reply[1], 0x00, "SOCKS5 CONNECT 被拒绝: {reply:?}");
    }
    let banner = read_ssh_banner(&mut proxy).await;
    assert!(
        banner.starts_with("SSH-"),
        "Dynamic 隧道没有转发到 sshd，收到的是: {banner:?}"
    );

    // 隧道列表：两条都在，且状态是 Active
    let tunnels = match h.dispatch(AppCommand::ListTunnels).await.unwrap() {
        CommandOutcome::Tunnels(tunnels) => tunnels,
        other => panic!("ListTunnels 返回了意外结果: {other:?}"),
    };
    assert_eq!(tunnels.len(), 2, "隧道列表应有两条: {tunnels:?}");
    for tunnel in &tunnels {
        assert_eq!(tunnel.state, TunnelState::Active, "隧道状态应为 Active");
    }

    // 暂停 / 恢复（按 id 找，列表顺序不保证）
    let tunnel_id = tunnels
        .iter()
        .find(|t| t.rule.bind_port == local_port)
        .expect("列表里没有 Local 隧道")
        .id;
    h.dispatch(AppCommand::SuspendTunnel { tunnel_id })
        .await
        .expect("暂停隧道失败");
    let tunnels = match h.dispatch(AppCommand::ListTunnels).await.unwrap() {
        CommandOutcome::Tunnels(tunnels) => tunnels,
        other => panic!("ListTunnels 返回了意外结果: {other:?}"),
    };
    let suspended = tunnels
        .iter()
        .find(|t| t.id == tunnel_id)
        .expect("列表里没有刚暂停的隧道");
    assert_eq!(suspended.state, TunnelState::Suspended);
    h.dispatch(AppCommand::ResumeTunnel { tunnel_id })
        .await
        .expect("恢复隧道失败");
    let tunnels = match h.dispatch(AppCommand::ListTunnels).await.unwrap() {
        CommandOutcome::Tunnels(tunnels) => tunnels,
        other => panic!("ListTunnels 返回了意外结果: {other:?}"),
    };
    let resumed = tunnels
        .iter()
        .find(|t| t.id == tunnel_id)
        .expect("列表里没有刚恢复的隧道");
    assert_eq!(resumed.state, TunnelState::Active);

    // 恢复之后必须真的能再转发（暂停→恢复的端口必须真的被释放并重新绑定）
    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", local_port)),
    )
    .await
    .expect("恢复后连接隧道超时")
    .expect("恢复后连接隧道失败");
    let banner = read_ssh_banner(&mut stream).await;
    assert!(
        banner.starts_with("SSH-"),
        "恢复后的隧道没有转发到 sshd，收到的是: {banner:?}"
    );

    // 关闭
    h.dispatch(AppCommand::CloseTunnel { tunnel_id })
        .await
        .expect("关闭隧道失败");
    let tunnels = match h.dispatch(AppCommand::ListTunnels).await.unwrap() {
        CommandOutcome::Tunnels(tunnels) => tunnels,
        other => panic!("ListTunnels 返回了意外结果: {other:?}"),
    };
    assert_eq!(tunnels.len(), 1, "关闭后应只剩一条隧道");
}

// ===== 6. 快速命令 / 脚本 / 密钥 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_quick_command_script_and_key_lifecycle() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let session_id = h.create_ssh_session("tools").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    // ── 快速命令：创建 → 执行（真的把文本送到远端） ──
    let command_id = Uuid::new_v4();
    h.dispatch(AppCommand::CreateQuickCommand {
        command: QuickCommand {
            id: command_id,
            name: "real-quick".to_string(),
            command: "echo RSHELL_QUICK_COMMAND_OK".to_string(),
            send_enter: true,
            description: "真机验收用".to_string(),
            scope: QuickCommandScope::AllSessions,
            hotkey: None,
            group: Some("验收".to_string()),
        },
    })
    .await
    .expect("创建快速命令失败");

    let commands = match h.dispatch(AppCommand::ListQuickCommands).await.unwrap() {
        CommandOutcome::QuickCommands(commands) => commands,
        other => panic!("ListQuickCommands 返回了意外结果: {other:?}"),
    };
    let created = commands
        .iter()
        .find(|c| c.id == command_id)
        .expect("新建的快速命令不在列表里");
    assert_eq!(created.description, "真机验收用");
    assert_eq!(created.group.as_deref(), Some("验收"));

    h.dispatch(AppCommand::ExecuteQuickCommand {
        command_id,
        target_sessions: vec![session_id],
    })
    .await
    .expect("执行快速命令失败");
    let text = h
        .output
        .wait_for_text(
            h.primary(session_id),
            "RSHELL_QUICK_COMMAND_OK",
            Duration::from_secs(20),
        )
        .await
        .expect("快速命令的文本没有到达远端");
    assert!(text.contains("RSHELL_QUICK_COMMAND_OK"));

    h.dispatch(AppCommand::DeleteQuickCommand { command_id })
        .await
        .expect("删除快速命令失败");
    let commands = match h.dispatch(AppCommand::ListQuickCommands).await.unwrap() {
        CommandOutcome::QuickCommands(commands) => commands,
        other => panic!("ListQuickCommands 返回了意外结果: {other:?}"),
    };
    assert!(!commands.iter().any(|c| c.id == command_id));

    // ── Rhai 脚本：真的执行并回传结果 ──
    h.dispatch(AppCommand::ExecuteScript {
        code: "let n = rshell_list_sessions().len; \"会话数: \" + n.to_string()".to_string(),
        session_id,
    })
    .await
    .expect("执行脚本失败");
    let finished = h
        .events
        .wait_for(
            |event| matches!(event, AppEvent::ScriptFinished { session_id: sid, .. } if *sid == session_id),
            Duration::from_secs(20),
        )
        .await
        .expect("脚本没有回传结果");
    match finished {
        AppEvent::ScriptFinished { result, .. } => {
            assert!(result.success, "脚本执行失败: {:?}", result.error);
            assert!(
                result.output.contains("会话数"),
                "脚本输出不对: {:?}",
                result.output
            );
        }
        _ => unreachable!(),
    }

    // ── 密钥：生成 → 列表 → 导出公钥 → 删除 ──
    let key_name = format!("real-ed25519-{}", &Uuid::new_v4().to_string()[..8]);
    h.dispatch(AppCommand::GenerateSshKey {
        name: key_name.clone(),
        key_type: SshKeyType::ED25519,
        passphrase: None,
    })
    .await
    .expect("生成密钥失败");
    let keys = match h.dispatch(AppCommand::ListKeys).await.unwrap() {
        CommandOutcome::Keys(keys) => keys,
        other => panic!("ListKeys 返回了意外结果: {other:?}"),
    };
    let generated = keys
        .iter()
        .find(|k| k.name == key_name)
        .expect("新生成的密钥不在列表里");
    assert!(generated.fingerprint.starts_with("SHA256:"));
    assert!(generated.public_key_blob.starts_with("ssh-ed25519 "));

    h.dispatch(AppCommand::ExportPublicKey {
        key_id: generated.id,
    })
    .await
    .expect("导出公钥失败");
    let exported = h
        .events
        .wait_for(
            |event| matches!(event, AppEvent::PublicKeyExported { key_id, .. } if *key_id == generated.id),
            Duration::from_secs(10),
        )
        .await
        .expect("导出公钥没有回传事件");
    match exported {
        AppEvent::PublicKeyExported { public_key, .. } => {
            assert!(public_key.starts_with("ssh-ed25519 "));
        }
        _ => unreachable!(),
    }

    h.dispatch(AppCommand::DeleteSshKey {
        key_id: generated.id,
    })
    .await
    .expect("删除密钥失败");
    let keys = match h.dispatch(AppCommand::ListKeys).await.unwrap() {
        CommandOutcome::Keys(keys) => keys,
        other => panic!("ListKeys 返回了意外结果: {other:?}"),
    };
    assert!(!keys.iter().any(|k| k.id == generated.id));
}

// ===== 7. 会话多开 + 分组内名称唯一 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_multi_tab_ptys_are_independent_and_names_unique_per_folder() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let env = real_env();

    // ── 名称规则：同组不得重名，跨组可以 ──
    let folder_a = match h
        .dispatch(AppCommand::CreateFolder {
            name: "real-folder-a".to_string(),
        })
        .await
        .expect("创建分组失败")
    {
        CommandOutcome::FolderId(id) => id,
        other => panic!("CreateFolder 返回了意外结果: {other:?}"),
    };
    let folder_b = match h
        .dispatch(AppCommand::CreateFolder {
            name: "real-folder-b".to_string(),
        })
        .await
        .expect("创建分组失败")
    {
        CommandOutcome::FolderId(id) => id,
        other => panic!("CreateFolder 返回了意外结果: {other:?}"),
    };

    h.create_ssh_session_in("dup-name", Some(folder_a), &env)
        .await;
    let clash = h
        .try_create_ssh_session_in("dup-name", Some(folder_a), &env)
        .await
        .is_err();
    assert!(clash, "同一分组内的同名会话必须被拒绝");
    // 跨组同名必须放行
    h.create_ssh_session_in("dup-name", Some(folder_b), &env)
        .await;

    // ── 会话多开：两个独立 pty，输出互不串台 ──
    let session_id = h
        .create_ssh_session_in("multi-tab", Some(folder_a), &env)
        .await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    let tab_one = Uuid::new_v4();
    let tab_two = Uuid::new_v4();
    h.dispatch(AppCommand::OpenTerminal {
        session_id,
        terminal_id: tab_one,
        cols: 120,
        rows: 40,
    })
    .await
    .expect("开第一个标签失败");
    h.dispatch(AppCommand::OpenTerminal {
        session_id,
        terminal_id: tab_two,
        cols: 120,
        rows: 40,
    })
    .await
    .expect("开第二个标签失败");

    h.send(session_id, Some(tab_one), "echo RSHELL_TAB_ONE\n")
        .await;
    h.send(session_id, Some(tab_two), "echo RSHELL_TAB_TWO\n")
        .await;

    let one = h
        .output
        .wait_for_text(tab_one, "RSHELL_TAB_ONE", Duration::from_secs(20))
        .await
        .expect("标签一没有收到自己的输出");
    let two = h
        .output
        .wait_for_text(tab_two, "RSHELL_TAB_TWO", Duration::from_secs(20))
        .await
        .expect("标签二没有收到自己的输出");
    assert!(one.contains("RSHELL_TAB_ONE"));
    assert!(two.contains("RSHELL_TAB_TWO"));
    assert!(
        !one.contains("RSHELL_TAB_TWO"),
        "标签一收到了标签二的输出（pty 没有隔离）"
    );
    assert!(
        !two.contains("RSHELL_TAB_ONE"),
        "标签二收到了标签一的输出（pty 没有隔离）"
    );

    // 关闭其中一个标签不影响另一个
    h.dispatch(AppCommand::CloseTerminal {
        session_id,
        terminal_id: tab_one,
    })
    .await
    .expect("关闭标签失败");
    h.send(
        session_id,
        Some(tab_two),
        "echo RSHELL_TAB_TWO_STILL_ALIVE\n",
    )
    .await;
    h.output
        .wait_for_text(
            tab_two,
            "RSHELL_TAB_TWO_STILL_ALIVE",
            Duration::from_secs(20),
        )
        .await
        .expect("关闭一个标签后另一个标签死了");
}

// ===== 8. 主密码：设置 → 锁定 → 解锁 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_master_password_setup_lock_unlock_and_rotate() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();

    let state = |outcome: CommandOutcome| match outcome {
        CommandOutcome::MasterPasswordState(state) => state,
        other => panic!("GetMasterPasswordState 返回了意外结果: {other:?}"),
    };

    let initial = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(initial, "NotSet", "初始状态应为 NotSet");

    h.dispatch(AppCommand::SetupMasterPassword {
        password: Secret::new("rshell-master-pw-1"),
    })
    .await
    .expect("设置主密码失败");
    let after_setup = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(after_setup, "Unlocked", "设置后应为 Unlocked");

    // 锁定
    h.dispatch(AppCommand::LockMasterPassword)
        .await
        .expect("锁定失败");
    let after_lock = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(after_lock, "Locked", "锁定后应为 Locked");

    // 错误口令 → 验证失败，且不解锁
    let verified = match h
        .dispatch(AppCommand::VerifyMasterPassword {
            password: Secret::new("wrong-password"),
        })
        .await
        .expect("验证主密码失败")
    {
        CommandOutcome::Verified(ok) => ok,
        other => panic!("VerifyMasterPassword 返回了意外结果: {other:?}"),
    };
    assert!(!verified, "错误口令不得通过验证");
    let still_locked = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(still_locked, "Locked", "验证失败后必须仍是 Locked");

    // 正确口令 → 解锁
    let verified = match h
        .dispatch(AppCommand::VerifyMasterPassword {
            password: Secret::new("rshell-master-pw-1"),
        })
        .await
        .expect("验证主密码失败")
    {
        CommandOutcome::Verified(ok) => ok,
        other => panic!("VerifyMasterPassword 返回了意外结果: {other:?}"),
    };
    assert!(verified, "正确口令必须通过验证");
    let after_unlock = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(after_unlock, "Unlocked", "验证后应为 Unlocked");

    // 改密：旧口令错误 → 失败；正确 → 成功且新口令可用
    let wrong_change = h
        .dispatch(AppCommand::ChangeMasterPassword {
            old_password: Secret::new("wrong-password"),
            new_password: Secret::new("rshell-master-pw-2"),
        })
        .await;
    assert!(wrong_change.is_err(), "改密时旧口令错误必须失败");

    h.dispatch(AppCommand::ChangeMasterPassword {
        old_password: Secret::new("rshell-master-pw-1"),
        new_password: Secret::new("rshell-master-pw-2"),
    })
    .await
    .expect("改密失败");
    h.dispatch(AppCommand::LockMasterPassword)
        .await
        .expect("锁定失败");
    let verified = match h
        .dispatch(AppCommand::VerifyMasterPassword {
            password: Secret::new("rshell-master-pw-2"),
        })
        .await
        .expect("验证主密码失败")
    {
        CommandOutcome::Verified(ok) => ok,
        other => panic!("VerifyMasterPassword 返回了意外结果: {other:?}"),
    };
    assert!(verified, "改密后新口令必须可用");

    // 停用
    h.dispatch(AppCommand::DisableMasterPassword {
        password: Secret::new("rshell-master-pw-2"),
    })
    .await
    .expect("停用主密码失败");
    let after_disable = state(
        h.dispatch(AppCommand::GetMasterPasswordState)
            .await
            .unwrap(),
    );
    assert_eq!(after_disable, "NotSet", "停用后应为 NotSet");
}

// ===== 9. 文件传输：上传 / 下载 / 同名冲突 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_transfer_uploads_downloads_and_reports_conflict() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let env = real_env();
    let session_id = h.create_ssh_session("transfer").await;
    h.connect(session_id).await;
    h.connected_event(session_id, Duration::from_secs(20)).await;

    // 远端目录带本次运行的后缀：上一轮残留（目录非空时删不掉）不会撞车，
    // 结束时也会被清理干净。
    let remote_dir = format!(
        "{}/rshell-real-transfer-{}",
        env.remote_dir.trim_end_matches('/'),
        &Uuid::new_v4().to_string()[..8]
    );
    h.dispatch(AppCommand::CreateRemoteDirectory {
        session_id,
        path: remote_dir.clone(),
    })
    .await
    .expect("创建远端目录失败");

    // 上传（远端参数是**目标文件路径**，不是目录）
    let remote_file = format!("{remote_dir}/payload.txt");
    let local_file = h.root.join("payload.txt");
    std::fs::write(&local_file, b"RSHELL_REAL_TRANSFER_PAYLOAD").expect("写本地文件失败");
    h.dispatch(AppCommand::EnqueueUpload {
        local: local_file.clone(),
        remote: remote_file.clone(),
        session_id,
        conflict: ConflictPolicy::Fail,
    })
    .await
    .expect("入队上传失败");

    h.events
        .wait_for(
            |event| matches!(event, AppEvent::TransferCompleted { .. }),
            Duration::from_secs(60),
        )
        .await
        .expect("上传没有完成");

    // 远端文件确实存在
    let entries = match h
        .dispatch(AppCommand::BrowseRemoteDir {
            session_id,
            path: remote_dir.clone(),
        })
        .await
        .expect("浏览远端目录失败")
    {
        CommandOutcome::RemoteDir { entries, .. } => entries,
        other => panic!("BrowseRemoteDir 返回了意外结果: {other:?}"),
    };
    assert!(
        entries.iter().any(|e| e.name == "payload.txt"),
        "远端没有 payload.txt: {entries:?}"
    );

    // ── 同名再传一次（Fail 策略）：必须被拒绝，且不得静默覆盖 ──
    std::fs::write(&local_file, b"RSHELL_REAL_TRANSFER_OVERWRITE_ATTEMPT").expect("写本地文件失败");
    let conflict = h
        .dispatch(AppCommand::EnqueueUpload {
            local: local_file.clone(),
            remote: remote_file.clone(),
            session_id,
            conflict: ConflictPolicy::Fail,
        })
        .await;
    assert!(
        conflict.is_err(),
        "同名目标在 Fail 策略下必须被拒绝（不得静默覆盖）"
    );
    let conflict_error = conflict.expect_err("同名上传应当失败");
    assert!(
        format!("{conflict_error}").contains("already exists"),
        "冲突错误信息应当说明目标已存在: {conflict_error}"
    );

    // 远端内容没有被改动
    let probe = h.root.join("probe.txt");
    h.dispatch(AppCommand::EnqueueDownload {
        remote: remote_file.clone(),
        local: probe.clone(),
        session_id,
        conflict: ConflictPolicy::Overwrite,
    })
    .await
    .expect("入队下载失败");
    let content = wait_local_file(&probe, Duration::from_secs(60))
        .await
        .expect("下载没有产生文件");
    assert_eq!(
        content, "RSHELL_REAL_TRANSFER_PAYLOAD",
        "被拒绝的同名上传不得改动远端已有文件"
    );

    // ── Overwrite 策略：允许覆盖 ──
    let completed = h.transfer_completed_count();
    h.dispatch(AppCommand::EnqueueUpload {
        local: local_file.clone(),
        remote: remote_file.clone(),
        session_id,
        conflict: ConflictPolicy::Overwrite,
    })
    .await
    .expect("Overwrite 策略的上传应被接受");
    h.wait_for_transfer_completion(completed, Duration::from_secs(60))
        .await;

    // ── Rename 策略：另存为新文件，原文件保留 ──
    let completed = h.transfer_completed_count();
    h.dispatch(AppCommand::EnqueueUpload {
        local: local_file.clone(),
        remote: remote_file.clone(),
        session_id,
        conflict: ConflictPolicy::Rename("renamed.txt".to_string()),
    })
    .await
    .expect("Rename 策略的上传应被接受");
    h.wait_for_transfer_completion(completed, Duration::from_secs(60))
        .await;
    let entries = match h
        .dispatch(AppCommand::BrowseRemoteDir {
            session_id,
            path: remote_dir.clone(),
        })
        .await
        .expect("浏览远端目录失败")
    {
        CommandOutcome::RemoteDir { entries, .. } => entries,
        other => panic!("BrowseRemoteDir 返回了意外结果: {other:?}"),
    };
    assert!(
        entries.iter().any(|e| e.name == "renamed.txt"),
        "Rename 策略没有生成新文件: {entries:?}"
    );
    assert!(
        entries.iter().any(|e| e.name == "payload.txt"),
        "Rename 策略不应删掉原文件: {entries:?}"
    );

    // 下载
    let download_target = h.root.join("downloaded.txt");
    h.dispatch(AppCommand::EnqueueDownload {
        remote: remote_file.clone(),
        local: download_target.clone(),
        session_id,
        conflict: ConflictPolicy::Overwrite,
    })
    .await
    .expect("入队下载失败");
    let content = wait_local_file(&download_target, Duration::from_secs(60))
        .await
        .expect("下载没有产生文件");
    assert_eq!(content, "RSHELL_REAL_TRANSFER_OVERWRITE_ATTEMPT");

    // 传输队列：两条任务都在，且都是终态
    let tasks = match h.dispatch(AppCommand::ListTransfers).await.unwrap() {
        CommandOutcome::Transfers(tasks) => tasks,
        other => panic!("ListTransfers 返回了意外结果: {other:?}"),
    };
    assert!(tasks.len() >= 3, "传输队列应有至少三条任务: {tasks:?}");
    assert!(tasks
        .iter()
        .any(|t| t.direction == TransferDirection::Upload));
    assert!(tasks
        .iter()
        .any(|t| t.direction == TransferDirection::Download));

    // 清理远端目录：先删文件（应用只允许删普通文件），再由 ssh 收掉空目录
    for name in ["payload.txt", "renamed.txt"] {
        h.dispatch(AppCommand::DeleteRemoteEntry {
            session_id,
            path: format!("{remote_dir}/{name}"),
        })
        .await
        .unwrap_or_else(|error| panic!("删除远端文件 {name} 失败: {error}"));
    }
    ssh_cleanup_dir(&env, &remote_dir);
}

// ===== 10. 分组 / 同步输入 / 撰写窗格 / 主题 =====

#[tokio::test]
#[ignore = "需要真实 SSH 服务器与 RSHELL_REAL_* 环境变量，不进 CI"]
async fn real_folders_sync_input_compose_and_themes() {
    let _guard = TEST_LOCK.lock().await;
    let h = Harness::new();
    let env = real_env();

    // ── 分组：建 → 列 → 改名 → 删（组内连接回默认组，连接不丢） ──
    let folder_id = match h
        .dispatch(AppCommand::CreateFolder {
            name: "real-folder".to_string(),
        })
        .await
        .expect("创建分组失败")
    {
        CommandOutcome::FolderId(id) => id,
        other => panic!("CreateFolder 返回了意外结果: {other:?}"),
    };
    let session_id = h
        .create_ssh_session_in("in-folder", Some(folder_id), &env)
        .await;

    let folders = match h.dispatch(AppCommand::ListFolders).await.unwrap() {
        CommandOutcome::Folders(folders) => folders,
        other => panic!("ListFolders 返回了意外结果: {other:?}"),
    };
    assert!(folders.iter().any(|f| f.id == folder_id));

    h.dispatch(AppCommand::RenameFolder {
        folder_id,
        name: "real-folder-renamed".to_string(),
    })
    .await
    .expect("重命名分组失败");
    let folders = match h.dispatch(AppCommand::ListFolders).await.unwrap() {
        CommandOutcome::Folders(folders) => folders,
        other => panic!("ListFolders 返回了意外结果: {other:?}"),
    };
    assert!(folders
        .iter()
        .any(|f| f.id == folder_id && f.name == "real-folder-renamed"));

    h.dispatch(AppCommand::DeleteFolder { folder_id })
        .await
        .expect("删除分组失败");
    let sessions = match h.dispatch(AppCommand::ListSessions).await.unwrap() {
        CommandOutcome::Sessions(sessions) => sessions,
        other => panic!("ListSessions 返回了意外结果: {other:?}"),
    };
    let kept = sessions
        .iter()
        .find(|s| s.id == session_id)
        .expect("删除分组后连接丢了");
    assert_eq!(kept.folder_id, None, "组内连接应回到默认组");

    // ── 同步输入：开启 → 另一会话收到同样的字节 ──
    let source = h.create_ssh_session("sync-source").await;
    let target = h.create_ssh_session("sync-target").await;
    h.connect(source).await;
    h.connect(target).await;
    h.connected_event(source, Duration::from_secs(20)).await;
    h.connected_event(target, Duration::from_secs(20)).await;

    h.dispatch(AppCommand::ToggleSyncInput {
        session_ids: vec![source, target],
    })
    .await
    .expect("开启同步输入失败");
    let synced = match h.dispatch(AppCommand::GetSyncInputSessions).await.unwrap() {
        CommandOutcome::SyncSessions(ids) => ids,
        other => panic!("GetSyncInputSessions 返回了意外结果: {other:?}"),
    };
    assert_eq!(synced.len(), 2);
    assert!(synced.contains(&source) && synced.contains(&target));

    h.send(source, None, "echo RSHELL_SYNC_OK\n").await;
    let text = h
        .output
        .wait_for_text(h.primary(target), "RSHELL_SYNC_OK", Duration::from_secs(20))
        .await
        .expect("同步输入没有把字节送到另一个会话");
    assert!(text.contains("RSHELL_SYNC_OK"));

    // 关闭同步输入（接口语义：传空数组表示关闭，不是再传一次同一组）
    h.dispatch(AppCommand::ToggleSyncInput {
        session_ids: vec![],
    })
    .await
    .expect("关闭同步输入失败");
    let synced = match h.dispatch(AppCommand::GetSyncInputSessions).await.unwrap() {
        CommandOutcome::SyncSessions(ids) => ids,
        other => panic!("GetSyncInputSessions 返回了意外结果: {other:?}"),
    };
    assert!(synced.is_empty(), "关闭后同步组应为空");

    // ── 撰写窗格：文本真的送到会话 ──
    h.dispatch(AppCommand::SendComposeText {
        content: "echo RSHELL_COMPOSE_OK".to_string(),
        target: ComposeTarget::CurrentSession,
        active_session_id: Some(source),
    })
    .await
    .expect("撰写窗格发送失败");
    let text = h
        .output
        .wait_for_text(
            h.primary(source),
            "RSHELL_COMPOSE_OK",
            Duration::from_secs(20),
        )
        .await
        .expect("撰写窗格的文本没有到达会话");
    assert!(text.contains("RSHELL_COMPOSE_OK"));

    // ── 主题 / 配色 ──
    h.dispatch(AppCommand::SetAppTheme {
        theme_name: "Light".to_string(),
    })
    .await
    .expect("切换主题失败");
    h.dispatch(AppCommand::SetAppTheme {
        theme_name: "Dark".to_string(),
    })
    .await
    .expect("切回主题失败");
    h.dispatch(AppCommand::SetTerminalColorScheme {
        scheme_name: "Monokai".to_string(),
    })
    .await
    .expect("切换配色失败");

    // 导入自定义配色方案：必须落盘（重启后还在）
    let scheme = TerminalColorScheme {
        name: "RealImported".to_string(),
        ansi_colors: [0x112233; 16],
        default_fg: 0xffffff,
        default_bg: 0x000000,
        cursor_fg: 0x000000,
        cursor_bg: 0xffffff,
        selection_fg: 0x000000,
        selection_bg: 0x444444,
    };
    h.dispatch(AppCommand::ImportColorScheme { scheme })
        .await
        .expect("导入配色方案失败");
    let store = h.root.join("color_schemes.json");
    assert!(store.exists(), "导入的配色方案必须落盘");
    let content = std::fs::read_to_string(&store).expect("读取配色方案文件失败");
    assert!(content.contains("RealImported"));
}
