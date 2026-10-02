//! RShell Tauri 壳入口（切片 1.1）
//!
//! 接线后端壳四件套（设计 §1.1 / §3 / §4.1）：
//! - `state.rs` 中的 `AppState`：注入 `Arc<CommandDispatcher>` + `Arc<TerminalChannels>`
//! - `events.rs` 中的 `spawn_bridge`：EventBus.subscribe → `app.emit("rshell://event")` 桥
//! - `terminal.rs` 中的 `TerminalChannels`：每会话双态 sink
//! - `error.rs` 中的 `IpcError`：CoreError → IPC 错误映射
//!
//! 切片 1.2 在 `commands.rs` 中加首批 7 个 `#[tauri::command]` 薄壳 +
//! `cmd!` 宏（设计 §3.4）。

mod commands;
mod error;
mod events;
mod state;
mod terminal;

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
use rshell_core::theme::ThemeManager;
use rshell_core::transfer::service::TransferService;
use rshell_core::CommandDispatcher;
use state::AppState;
use tauri::Manager;
use terminal::TerminalChannels;
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
            let terminal_channels = Arc::new(TerminalChannels::new());
            let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel();
            terminal_service.set_output_sender(output_tx);
            let channels_for_output = terminal_channels.clone();
            tauri::async_runtime::spawn(async move {
                while let Some((session_id, data)) = output_rx.recv().await {
                    channels_for_output.push(session_id, &data).await;
                }
            });
            let trigger_engine = Arc::new(TriggerEngine::with_path(
                event_bus.clone(),
                data_root.join("triggers.json"),
            ));
            let host_key_registry = Arc::new(HostKeyDecisionRegistry::new(event_bus.clone()));

            let credentials = Arc::new(rshell_infra::credentials::SystemCredentialStore::new(
                "com.letmlook.rshell.credentials",
            ));
            let session_repository = Arc::new(SessionRepository::with_default_path(credentials));
            let session_service = Arc::new(SessionService::with_repository(
                event_bus.clone(),
                terminal_service.clone(),
                trigger_engine.clone(),
                host_key_registry.clone(),
                Some(session_repository),
            ));

            let transfer_service = Arc::new(TransferService::new(event_bus.clone()));
            let key_manager = Arc::new(KeyManager::new(keys_dir, event_bus.clone()));
            let master_password = Arc::new(MasterPassword::new(event_bus.clone()));
            let tunnel_manager = Arc::new(
                TunnelManager::new(event_bus.clone())
                    .with_persistence(data_root.join("tunnels.toml")),
            );
            let host_key_manager = Arc::new(HostKeyManager::new(known_hosts_path));
            let theme_manager = Arc::new(ThemeManager::new(event_bus.clone()));

            // SessionService restores saved sessions during construction.

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
            dispatcher.initialize();

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
            commands::list_session_load_issues,
            commands::retry_session_load,
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
            commands::create_remote_directory,
            commands::delete_remote_entry,
            commands::list_transfers,
            commands::list_tunnels,
            commands::list_pending_tunnels,
            commands::generate_ssh_key,
            commands::import_private_key,
            commands::delete_ssh_key,
            commands::setup_master_password,
            commands::change_master_password,
            commands::trust_host_key,
            commands::execute_quick_command,
            commands::list_quick_commands,
            commands::list_triggers,
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
            commands::list_plugins,
            commands::send_compose_text,
            commands::toggle_sync_input,
            commands::export_public_key,
            commands::delete_host_key,
            commands::import_color_scheme,
            commands::load_plugin,
            commands::unload_plugin,
            commands::push_one_mb,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
