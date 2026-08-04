//! Tauri `#[tauri::command]` 薄壳层（切片 1.2）。
//!
//! 规则（设计 §1.3 边界铁律 3）：薄壳仅做参数转换、`AppCommand` 构造、
//! `CommandOutcome` 解包、错误映射、Channel 路由。**不**含业务逻辑。
//!
//! 设计 §3.4 宏消除 56 个薄壳的样板。首批 7 个命令：
//! - `create_session` / `list_sessions` / `connect_session` / `disconnect_session`
//! - `update_session` / `delete_session`
//! - `send_input` / `resize_terminal` / `attach_terminal`

use rshell_api::types::SessionConfig;
use rshell_api::{AppCommand, CommandOutcome};
use tauri::ipc::Channel;
use tauri::State;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::IpcError;
use crate::state::AppState;

/// 设计 §3.4 宏消除薄壳样板。
///
/// 用法：
/// ```ignore
/// cmd!(create_session(config: SessionConfig) -> SessionId(SessionId) =
///     AppCommand::CreateSession { config });
/// ```
///
/// `cmd!` 把 `$name` 函数体构造为：
///   1. await `dispatch($variant)`
///   2. match outcome 期望变体 → 提取负载返回 Ok
///   3. 不匹配 → `Err(IpcError::outcome_mismatch(...))`
///
/// 命名约定：函数名严格遵循前端 `src/ipc/client.ts:39-45` 的
/// PascalCase → snake_case 转换（设计 §3.4）。
macro_rules! cmd {
    // 无参版本:用于 list_sessions 等
    ($name:ident() -> $out:ident() = $variant:expr) => {
        cmd!($name() -> $out(CommandOutcome) = $variant);
    };
    // 有参版本:第一个 arm 用于简单负载(如 SessionId),
    // 第二个 arm 用于具名负载(如 SessionConfig)。
    ($name:ident($($arg:ident: $ty:ty),*) -> $out:ident($ret:ty) = $variant:expr) => {
        // rename_all = "snake_case":Tauri 2 默认把形参从 snake_case 转
        // camelCase 暴露给前端,但本项目前端 `src/ipc/client.ts` 一律发
        // snake_case 字段名(与 rshell-api AppCommand 一致)。
        // 关闭默认重命名,使前后端共用 snake_case 契约。
        #[tauri::command(rename_all = "snake_case")]
        pub async fn $name(
            $($arg: $ty,)*
            state: State<'_, AppState>,
        ) -> Result<$ret, IpcError> {
            let outcome = state.dispatcher.dispatch($variant).await
                .map_err(IpcError::from)?;
            match outcome {
                CommandOutcome::$out(v) => Ok(v),
                other => {
                    warn!(
                        command = stringify!($name),
                        expected = stringify!($out),
                        actual = other.kind(),
                        "CommandOutcome mismatch (dispatcher branch bug)"
                    );
                    Err(IpcError::outcome_mismatch(stringify!($out), other.kind()))
                }
            }
        }
    };
}

// ─────────────────────────────────────────────────────────────────────
// 首批 7 个薄壳 —— 设计 §3.4 / 切片 1.2
// ─────────────────────────────────────────────────────────────────────

// 写命令:返回 None（薄壳统一返回 CommandOutcome::None,序列化后为空 JSON）
// 用宏不易表达"返回 ()",改写为直接函数体。
//
// 所有形参遵循 snake_case 命名 —— `rename_all = "snake_case"` 关闭 Tauri 2
// 默认的 camelCase 重命名,使前后端共用 snake_case 契约(与 rshell-api 一致)。
#[tauri::command(rename_all = "snake_case")]
pub async fn connect_session(session_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::ConnectSession { session_id }).await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn disconnect_session(session_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::DisconnectSession { session_id }).await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_session(
    id: Uuid,
    config: SessionConfig,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::UpdateSession { id, config }).await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_session(id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::DeleteSession { id }).await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn send_input(
    session_id: Uuid,
    data: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::SendInput { session_id, data }).await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resize_terminal(
    session_id: Uuid,
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state.dispatcher.dispatch(AppCommand::ResizeTerminal { session_id, cols, rows }).await
        .map_err(IpcError::from)?;
    Ok(())
}

// CreateSession:从 dispatch 返回的 SessionId(Uuid) 中取出 inner Uuid
#[tauri::command(rename_all = "snake_case")]
pub async fn create_session(
    config: SessionConfig,
    state: State<'_, AppState>,
) -> Result<Uuid, IpcError> {
    let outcome = state.dispatcher.dispatch(AppCommand::CreateSession { config }).await
        .map_err(IpcError::from)?;
    match outcome {
        CommandOutcome::SessionId(id) => Ok(id),
        other => Err(IpcError::outcome_mismatch("SessionId", other.kind())),
    }
}

// ListSessions:返回 Vec<SessionConfig>
cmd!(list_sessions() -> Sessions(Vec<SessionConfig>) = AppCommand::ListSessions);

// 切片 4 新增：decide_host_key —— 把 UI 端的 host key 决策通过 dispatcher
// 派发到 HostKeyDecisionRegistry.resolve(decision_id, decision),唤醒阻塞在
// oneshot::Receiver 的 SshHandler::check_server_key 同步 trait 方法。
//
// 决策结构:rshell_protocol::ssh::HostKeyDecision { fingerprint, key_blob, accept, permanent }
// —— dispatcher 的 DecideHostKey 分支已处理(切片 1.2)。permanent=true 时
// known_hosts 写入留到切片 6 密钥管理域;本切片仅做一次性唤醒。
#[tauri::command(rename_all = "snake_case")]
pub async fn decide_host_key(
    decision_id: Uuid,
    accept: bool,
    permanent: bool,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::DecideHostKey {
            decision_id,
            accept,
            permanent,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// ===== 切片 5: SFTP 传输薄壳 =====
// 写命令:返回 ()（slice 5.1 切片 5.3 验证取消/暂停链路）
#[tauri::command(rename_all = "snake_case")]
pub async fn enqueue_upload(
    local: String,
    remote: String,
    session_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::EnqueueUpload {
            local: std::path::PathBuf::from(local),
            remote,
            session_id,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn enqueue_download(
    remote: String,
    local: String,
    session_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::EnqueueDownload {
            remote,
            local: std::path::PathBuf::from(local),
            session_id,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pause_transfer(task_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::PauseTransfer { task_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resume_transfer(task_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ResumeTransfer { task_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cancel_transfer(task_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::CancelTransfer { task_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// browse_remote_dir —— dispatcher 当前返回 None(切片 3 迁移,CommandOutcome::RemoteDir
// 完整接通留到切片 5+)。本切片先注册占位薄壳保证 IPC 路由可用。
#[tauri::command(rename_all = "snake_case")]
pub async fn browse_remote_dir(
    session_id: Uuid,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::BrowseRemoteDir { session_id, path })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// ===== 切片 6: 密钥管理 + 主密码薄壳 =====
use rshell_api::types::SshKeyType;

// generate_ssh_key
#[tauri::command(rename_all = "snake_case")]
pub async fn generate_ssh_key(
    name: String,
    key_type: SshKeyType,
    passphrase: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::GenerateSshKey { name, key_type, passphrase })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// import_private_key
#[tauri::command(rename_all = "snake_case")]
pub async fn import_private_key(
    path: String,
    passphrase: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ImportPrivateKey {
            path: std::path::PathBuf::from(path),
            passphrase,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_ssh_key(key_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::DeleteSshKey { key_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn setup_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::SetupMasterPassword { password })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn change_master_password(
    old_password: String,
    new_password: String,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ChangeMasterPassword { old_password, new_password })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// trust_host_key —— permanent=true 走 host_key_manager.trust_host_key 持久化
#[tauri::command(rename_all = "snake_case")]
pub async fn trust_host_key(
    host: String,
    port: u16,
    key_type: String,
    public_key_blob: String,
    decision: rshell_api::types::TrustHostKeyDecision,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::TrustHostKey {
            host,
            port,
            key_type,
            public_key_blob,
            decision,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// ===== 切片 7: 触发器/快速命令/脚本薄壳 =====
use rshell_api::types::{QuickCommand, Trigger};

// 写命令:返回 ()
#[tauri::command(rename_all = "snake_case")]
pub async fn execute_quick_command(
    command_id: Uuid,
    target_sessions: Vec<Uuid>,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ExecuteQuickCommand {
            command_id,
            target_sessions,
        })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_quick_command(
    command: QuickCommand,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::CreateQuickCommand { command })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_quick_command(
    command_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::DeleteQuickCommand { command_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_trigger(trigger: Trigger, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::CreateTrigger { trigger })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_trigger(trigger_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::DeleteTrigger { trigger_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn toggle_trigger(trigger_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ToggleTrigger { trigger_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn execute_script(
    code: String,
    session_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ExecuteScript { code, session_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// ===== 切片 8: 隧道 CRUD 薄壳 =====
// 注:ListTunnels / ListPendingTunnels 在切片 2.2 已迁移到 CommandOutcome,薄壳
// 也在切片 3.1 通过 cmd! 宏注册。本切片补齐剩下的 create_tunnel / close_tunnel
// / restore_tunnel / suspend_tunnel / resume_tunnel。
use rshell_api::types::PortForwardRule;

#[tauri::command(rename_all = "snake_case")]
pub async fn create_tunnel(
    session_id: Uuid,
    rule: PortForwardRule,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::CreateTunnel { session_id, rule })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn close_tunnel(tunnel_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::CloseTunnel { tunnel_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn restore_tunnel(
    session_id: Uuid,
    rule: PortForwardRule,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::RestoreTunnel { session_id, rule })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn suspend_tunnel(tunnel_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::SuspendTunnel { tunnel_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resume_tunnel(tunnel_id: Uuid, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ResumeTunnel { tunnel_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// ===== 切片 9: 插件 CRUD 薄壳 =====
// 注:ListPlugins 在切片 2.2 已迁移到 CommandOutcome。
// 本切片按用户校核（2026-07-31 决策）：仅做 IPC 接入，WasmSandbox 仍是 scaffold，
// 实际加载 / 执行返回 `IpcError { kind: "internal", message: "plugin sandbox not yet implemented" }`。
#[tauri::command(rename_all = "snake_case")]
pub async fn scan_plugins(state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::ScanPlugins)
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn load_plugin(plugin_id: String, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::LoadPlugin { plugin_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn unload_plugin(plugin_id: String, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::UnloadPlugin { plugin_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn enable_plugin(plugin_id: String, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::EnablePlugin { plugin_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn disable_plugin(plugin_id: String, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::DisablePlugin { plugin_id })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}
use rshell_api::types::{SshKeyInfo, ThemeInfo};

// ListKeys:返回 Vec<SshKeyInfo>
cmd!(list_keys() -> Keys(Vec<SshKeyInfo>) = AppCommand::ListKeys);

// ListThemes:返回 ThemesInfo
cmd!(list_themes() -> Themes(ThemeInfo) = AppCommand::ListThemes);

// VerifyMasterPassword:返回 bool
#[tauri::command(rename_all = "snake_case")]
pub async fn verify_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<bool, IpcError> {
    let outcome = state
        .dispatcher
        .dispatch(AppCommand::VerifyMasterPassword { password })
        .await
        .map_err(IpcError::from)?;
    match outcome {
        CommandOutcome::Verified(b) => Ok(b),
        other => Err(IpcError::outcome_mismatch("Verified", other.kind())),
    }
}

// SetAppTheme / SetTerminalColorScheme:写命令,返回 ()
#[tauri::command(rename_all = "snake_case")]
pub async fn set_app_theme(theme_name: String, state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::SetAppTheme { theme_name })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_terminal_color_scheme(
    scheme_name: String,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state
        .dispatcher
        .dispatch(AppCommand::SetTerminalColorScheme { scheme_name })
        .await
        .map_err(IpcError::from)?;
    Ok(())
}

// attach_terminal —— 设计 §4.1
// 把 Channel<Vec<u8>> 注册到 TerminalChannels,flush 积压后切换为 Attached 模式。
// 该命令**不走 dispatcher**(Channel 是壳层基础设施,不需要业务逻辑)。
#[tauri::command(rename_all = "snake_case")]
pub async fn attach_terminal(
    session_id: Uuid,
    on_data: Channel<Vec<u8>>,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    state.terminal_channels.attach(session_id, on_data).await;
    info!(session_id = %session_id, "Terminal attached; backend→frontend channel established");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
// 切片 0 雏形命令 —— 切片 0 完成后 list_sessions 已被正式版接管,
// push_one_mb 继续保留到切片 1.4 完成判据(吞吐基线入库)。
// ─────────────────────────────────────────────────────────────────────

/// 切片 0.3 临时命令 —— 通过 Channel 推 1 MiB 假字节。
///
/// 切片 1.4 完成判据走完后删除。
#[tauri::command(rename_all = "snake_case")]
pub async fn push_one_mb(channel: Channel<Vec<u8>>) -> Result<usize, String> {
    use tracing::info;
    const CHUNK: usize = 16 * 1024;
    const CHUNKS: usize = 1024 * 1024 / CHUNK;
    let buf: Vec<u8> = vec![b'.'; CHUNK];
    let start = std::time::Instant::now();
    for _i in 0..CHUNKS {
        channel.send(buf.clone()).map_err(|e| e.to_string())?;
    }
    let elapsed = start.elapsed();
    info!(
        bytes = CHUNK * CHUNKS,
        elapsed_ms = elapsed.as_millis() as u64,
        throughput_mibps = (CHUNK * CHUNKS) as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64(),
        "push_one_mb done"
    );
    Ok(CHUNK * CHUNKS)
}

#[cfg(test)]
mod tests {
    /// 14 个首批薄壳都被注册过编译期可见（切片 3.1 增 5 + 切片 4 增 1 = 14）。
    /// 本测试仅做存在性 + 命令名稳定性回归,真实 invoke 往返由前端 E2E 验证。
    #[test]
    fn command_names_match_client_ts_convention() {
        // 客户端 `src/ipc/client.ts` 把 PascalCase 转换为 snake_case 调用。
        // 这里只断言函数名遵循 Rust snake_case —— 集成测试 (cargo tauri dev)
        // 验证真实路由。
        let _names = [
            stringify!(create_session),
            stringify!(list_sessions),
            stringify!(connect_session),
            stringify!(disconnect_session),
            stringify!(update_session),
            stringify!(delete_session),
            stringify!(send_input),
            stringify!(resize_terminal),
            stringify!(attach_terminal),
            stringify!(decide_host_key),
            stringify!(list_keys),
            stringify!(list_themes),
            stringify!(verify_master_password),
            stringify!(set_app_theme),
            stringify!(set_terminal_color_scheme),
        ];
    }
}