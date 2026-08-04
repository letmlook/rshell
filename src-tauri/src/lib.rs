//! RShell Tauri 壳入口（切片 1.1）
//!
//! 接线后端壳四件套（设计 §1.1 / §3 / §4.1）：
//! - `state.rs` 中的 `AppState`：注入 `Arc<CommandDispatcher>` + `SharedTerminalChannels`
//! - `events.rs` 中的 `spawn_bridge`：EventBus.subscribe → `app.emit("rshell://event")` 桥
//! - `rshell-core::terminal::channels` 中的 `TerminalChannels`：每会话双态 sink
//! - `error.rs` 中的 `IpcError`：CoreError → IPC 错误映射
//!
//! 切片 1.2 在 `commands.rs` 中加首批 7 个 `#[tauri::command]` 薄壳 +
//! `cmd!` 宏（设计 §3.4）。

mod commands;
mod error;
mod events;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

use rshell_core::event_bus::EventBus;
use rshell_core::script::trigger_engine::TriggerEngine;
use rshell_core::security::host_key_decision::HostKeyDecisionRegistry;
use rshell_core::security::host_key_manager::HostKeyManager;
use rshell_core::security::key_manager::KeyManager;
use rshell_core::security::master_password::MasterPassword;
use rshell_core::security::tunnel_manager::TunnelManager;
use rshell_core::session::repository::SessionRepository;
use rshell_core::session::service::SessionService;
use rshell_core::terminal::service::TerminalService;
use rshell_core::terminal::SharedTerminalChannels;
use rshell_core::theme::ThemeManager;
use rshell_core::transfer::service::TransferService;
use rshell_core::CommandDispatcher;
use state::AppState;
use tauri::Manager;
use tracing::info;

/// 数据根目录：`dirs::data_local_dir()/rshell/`
fn data_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rshell")
}

/// Tauri 应用入口
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,rshell=debug")),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // ── 1. EventBus ─────────────────────────────────────────────
            let event_bus = Arc::new(EventBus::new());

            // ── 2. 各 service ───────────────────────────────────────────
            let data_root = data_root();
            std::fs::create_dir_all(&data_root).ok();
            let keys_dir = data_root.join("keys");
            std::fs::create_dir_all(&keys_dir).ok();
            let known_hosts_path = data_root.join("known_hosts");

            let terminal_service = Arc::new(TerminalService::new(event_bus.clone()));
            let trigger_engine = Arc::new(TriggerEngine::new(event_bus.clone()));
            let host_key_registry = Arc::new(HostKeyDecisionRegistry::new(event_bus.clone()));

            // ── TerminalChannels(设计 §4.1 双态 sink) ──
            // 提前建好:既给 SessionService 用(SSH recv 字节直推),
            // 又给 attach_terminal 命令用(前端 mount xterm 时 attach Channel)。
            let terminal_channels: SharedTerminalChannels =
                Arc::new(rshell_core::terminal::TerminalChannels::new());

            let session_repository = Arc::new(SessionRepository::with_default_path());
            let session_service = Arc::new(SessionService::with_full(
                event_bus.clone(),
                terminal_service.clone(),
                trigger_engine.clone(),
                host_key_registry.clone(),
                Some(session_repository),
                Some(terminal_channels.clone()),
            ));

            let transfer_service = Arc::new(TransferService::new(event_bus.clone()));
            let key_manager = Arc::new(KeyManager::new(keys_dir, event_bus.clone()));
            let master_password = Arc::new(MasterPassword::new(event_bus.clone()));
            let tunnel_manager = Arc::new(TunnelManager::new(event_bus.clone()));
            let host_key_manager = Arc::new(HostKeyManager::new(
                known_hosts_path,
                event_bus.clone(),
            ));
            let theme_manager = Arc::new(ThemeManager::new(event_bus.clone()));

            // ── 3. 切片 1.1：load_from_disk 不在 setup 阻塞,改为 spawn ──
            // 设计 §4.5：磁盘加载失败不阻断启动,用户首次启动本来就空。
            let ss_for_load = session_service.clone();
            tokio::spawn(async move {
                ss_for_load.load_from_disk().await;
            });

            // ── 4. CommandDispatcher ────────────────────────────────────
            let dispatcher = Arc::new(CommandDispatcher::new(
                rshell_core::command_dispatcher::Services {
                    session_service: session_service.clone(),
                    terminal_service: terminal_service.clone(),
                    transfer_service: transfer_service.clone(),
                    trigger_engine: trigger_engine.clone(),
                    key_manager: key_manager.clone(),
                    master_password: master_password.clone(),
                    tunnel_manager: tunnel_manager.clone(),
                    host_key_manager: host_key_manager.clone(),
                    theme_manager: theme_manager.clone(),
                    event_bus: event_bus.clone(),
                    host_key_registry: host_key_registry.clone(),
                },
            ));

            // ── 5. EventBus → Tauri emit 桥 ─────────────────────────────
            events::subscribe_bridge(event_bus.clone(), app.handle().clone());

            // ── 6. AppState ─────────────────────────────────────────────
            app.manage(AppState {
                dispatcher,
                terminal_channels,
            });

            info!(
                "RShell Tauri shell started (slice 1.1); data_root = {}",
                data_root.display()
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::create_session,
            commands::update_session,
            commands::delete_session,
            commands::connect_session,
            commands::disconnect_session,
            commands::send_input,
            commands::resize_terminal,
            commands::attach_terminal,
            commands::decide_host_key,
            commands::list_keys,
            commands::list_themes,
            commands::verify_master_password,
            commands::set_app_theme,
            commands::set_terminal_color_scheme,
            commands::enqueue_upload,
            commands::enqueue_download,
            commands::pause_transfer,
            commands::resume_transfer,
            commands::cancel_transfer,
            commands::browse_remote_dir,
            commands::generate_ssh_key,
            commands::import_private_key,
            commands::delete_ssh_key,
            commands::setup_master_password,
            commands::change_master_password,
            commands::trust_host_key,
            commands::execute_quick_command,
            commands::create_quick_command,
            commands::delete_quick_command,
            commands::create_trigger,
            commands::delete_trigger,
            commands::toggle_trigger,
            commands::execute_script,
            commands::create_tunnel,
            commands::close_tunnel,
            commands::restore_tunnel,
            commands::suspend_tunnel,
            commands::resume_tunnel,
            commands::scan_plugins,
            commands::load_plugin,
            commands::unload_plugin,
            commands::enable_plugin,
            commands::disable_plugin,
            commands::push_one_mb,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}