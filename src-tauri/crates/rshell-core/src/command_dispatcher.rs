//! 命令分发器（前端 → 后端）
//!
//! 前端发送命令，CommandDispatcher 路由到对应的 Service 处理。
//! 这是前后端分离架构中前端向后端发送请求的唯一通道。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::script::compose::ComposeService;
use crate::script::engine::{ScriptContext, ScriptEngine};
use crate::script::quick_command::QuickCommandService;
use crate::script::sync_input::SyncInputService;
use crate::script::trigger_engine::TriggerEngine;
use crate::security::key_manager::KeyManager;
use crate::security::master_password::MasterPassword;
use crate::security::tunnel_manager::TunnelManager;
use crate::security::host_key_manager::HostKeyManager;
use crate::session::service::SessionService;
use crate::terminal::service::TerminalService;
use crate::theme::ThemeManager;
use crate::transfer::service::TransferService;
use rshell_api::types::{TerminalBufferSnapshot, TerminalConfig};
use rshell_api::AppCommand;
use rshell_protocol::ssh::HostKeyDecision;
use rshell_protocol::Connection;
use rshell_protocol::telnet::TelnetConnection;
use rshell_protocol::serial::{SerialConnection, SerialConfig as ProtocolSerialConfig};
use rshell_plugin_sdk::loader::PluginLoader;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

/// 外部注入的服务/注册表 bundle
///
/// 把 `CommandDispatcher::new` 之前散落的 11 个参数收到一个 struct 里,
/// 减少新服务接入时的改动面,也方便测试时构造一个 mock Services。
///
/// 内部用的 4 个 (`quick_command_service` / `compose_service` /
/// `script_engine` / `sync_input_service` / `plugin_loader`) 由 dispatcher
/// 自行从 `event_bus` 创建,不需要外部注入。
pub struct Services {
    pub session_service: Arc<SessionService>,
    pub terminal_service: Arc<TerminalService>,
    pub transfer_service: Arc<TransferService>,
    pub trigger_engine: Arc<TriggerEngine>,
    pub key_manager: Arc<KeyManager>,
    pub master_password: Arc<MasterPassword>,
    pub tunnel_manager: Arc<TunnelManager>,
    pub host_key_manager: Arc<HostKeyManager>,
    pub theme_manager: Arc<ThemeManager>,
    pub event_bus: Arc<EventBus>,
    pub host_key_registry: Arc<crate::security::host_key_decision::HostKeyDecisionRegistry>,
}

/// 命令分发器
pub struct CommandDispatcher {
    session_service: Arc<SessionService>,
    terminal_service: Arc<TerminalService>,
    transfer_service: Arc<TransferService>,
    quick_command_service: Arc<QuickCommandService>,
    trigger_engine: Arc<TriggerEngine>,
    compose_service: Arc<ComposeService>,
    script_engine: Arc<ScriptEngine>,
    sync_input_service: Arc<SyncInputService>,
    key_manager: Arc<KeyManager>,
    master_password: Arc<MasterPassword>,
    tunnel_manager: Arc<TunnelManager>,
    host_key_manager: Arc<HostKeyManager>,
    theme_manager: Arc<ThemeManager>,
    plugin_loader: Arc<PluginLoader>,
    event_bus: Arc<EventBus>,
    /// 主机密钥决策注册表:负责把 SshHandler 同步等待转成 UI 端异步响应
    host_key_registry: Arc<crate::security::host_key_decision::HostKeyDecisionRegistry>,
}

impl CommandDispatcher {
    /// 创建新的命令分发器
    ///
    /// 接受一组"已在外部创建好"的服务/注册表。trigger_engine 必须在外部创建并共享,
    /// 因为 SessionService 后台 recv 循环也需要它做 trigger 匹配 ——
    /// 用同一个 Arc,确保用户通过 `CreateTrigger` 加进去的项对 recv 循环立即可见。
    pub fn new(services: Services) -> Self {
        let Services {
            session_service,
            terminal_service,
            transfer_service,
            trigger_engine,
            key_manager,
            master_password,
            tunnel_manager,
            host_key_manager,
            theme_manager,
            event_bus,
            host_key_registry,
        } = services;

        let quick_command_service = Arc::new(QuickCommandService::new(event_bus.clone()));
        let compose_service = Arc::new(ComposeService::new(event_bus.clone()));
        // rhai::Engine is !Send+!Sync by design; ScriptEngine is only ever touched
        // from the dedicated backend thread that owns this Arc (see bridge.rs).
        #[allow(clippy::arc_with_non_send_sync)]
        let script_engine = Arc::new(ScriptEngine::new(event_bus.clone()));
        let sync_input_service = Arc::new(SyncInputService::new(event_bus.clone()));

        // 插件目录：用户数据目录/plugins
        let plugins_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rshell")
            .join("plugins");
        let plugin_loader = Arc::new(PluginLoader::new(plugins_dir));

        Self {
            session_service,
            terminal_service,
            transfer_service,
            quick_command_service,
            trigger_engine,
            compose_service,
            script_engine,
            sync_input_service,
            key_manager,
            master_password,
            tunnel_manager,
            host_key_manager,
            theme_manager,
            plugin_loader,
            event_bus,
            host_key_registry,
        }
    }

    /// 初始化传输服务的 SSH 客户端提供函数
    pub async fn initialize(&self) {
        let session_service = self.session_service.clone();
        let provider = Arc::new(
            move |session_id: Uuid| -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<crate::session::service::SshClientHandle, CoreError>>
                    + Send>,
            > {
                let svc = session_service.clone();
                Box::pin(async move { svc.get_ssh_client(session_id).await })
            },
        );
        self.transfer_service.set_ssh_client_provider(provider).await;
    }

    /// 分发命令（前端调用）
    #[instrument(skip(self, command), fields(command = ?command))]
    pub async fn dispatch(&self, command: AppCommand) -> Result<(), CoreError> {
        debug!("Dispatching command");

        match command {
            // ===== 会话命令 =====
            AppCommand::ConnectSession { session_id } => {
                self.session_service.connect(session_id).await?;
            }
            AppCommand::DisconnectSession { session_id } => {
                self.session_service.disconnect(session_id).await?;
            }
            AppCommand::CreateSession { config } => {
                self.session_service.create_session(config).await?;
            }
            AppCommand::UpdateSession { id, config } => {
                self.session_service.update_session(id, config).await?;
            }
            AppCommand::DeleteSession { id } => {
                self.session_service.delete_session(id).await?;
            }

            // ===== 终端命令 =====
            AppCommand::SendInput { session_id, data } => {
                // 用户输入直接发到 SessionService（→ 远端 SSH shell），
                // 不再走 TerminalService::send_input —— 那个方法会
                // 把 data 重新当 TerminalOutput 发布，构成回路，
                // 真实场景下用户输入不会在本地终端回显（前端 View 已渲染）。
                self.session_service.send_data(session_id, &data).await?;
                // 同步输入：同时发送到其他同步会话
                if self.sync_input_service.is_sync_active()? {
                    self.sync_input_service
                        .send_to_synced_sessions(&data, &self.session_service)
                        .await?;
                }
            }
            AppCommand::ResizeTerminal { session_id, cols, rows } => {
                self.terminal_service.resize(session_id, cols, rows)?;
            }
            AppCommand::CopySelection { session_id } => {
                // 从 TerminalService 拿当前 buffer，序列化为文本并发布 ClipboardCopy
                // 事件。MVP 实现：拷贝整个可见屏幕（Xshell 中对应 "Copy All"）。
                // 真实选择区跟踪留作后续（需在 TerminalService 中加 Selection 状态）。
                match self.terminal_service.get_buffer_snapshot(session_id) {
                    Ok(snapshot) => {
                        let text = buffer_snapshot_to_text(&snapshot);
                        info!(
                            session_id = %session_id,
                            bytes = text.len(),
                            "CopySelection → ClipboardCopy"
                        );
                        self.event_bus.publish(rshell_api::AppEvent::ClipboardCopy { text });
                    }
                    Err(e) => {
                        warn!(
                            session_id = %session_id,
                            error = %e,
                            "CopySelection failed: terminal snapshot unavailable"
                        );
                    }
                }
            }

            // ===== 文件传输命令 =====
            AppCommand::EnqueueUpload { local, remote, session_id } => {
                self.transfer_service.enqueue_upload(local, remote, session_id).await?;
            }
            AppCommand::EnqueueDownload { remote, local, session_id } => {
                self.transfer_service.enqueue_download(remote, local, session_id).await?;
            }
            AppCommand::PauseTransfer { task_id } => {
                self.transfer_service.pause_transfer(task_id).await?;
            }
            AppCommand::ResumeTransfer { task_id } => {
                self.transfer_service.resume_transfer(task_id).await?;
            }
            AppCommand::CancelTransfer { task_id } => {
                self.transfer_service.cancel_transfer(task_id).await?;
            }
            AppCommand::BrowseRemoteDir { session_id, path } => {
                self.session_service.browse_remote_dir(session_id, &path).await?;
            }

            // ===== 隧道命令 =====
            AppCommand::CreateTunnel { session_id, rule } => {
                // 尝试获取关联会话的 SSH client 引用，供隧道做 direct-tcpip 转发
                let ssh_client = self.session_service.get_ssh_client(session_id).await.ok();
                self.tunnel_manager.create_tunnel(session_id, rule, ssh_client).await?;
            }
            AppCommand::CloseTunnel { tunnel_id } => {
                self.tunnel_manager.close_tunnel(tunnel_id).await?;
            }
            AppCommand::ListPendingTunnels => {
                let pending = self.tunnel_manager.restore_pending_rules().await;
                info!(count = pending.len(), "UI requested pending tunnels");
                self.event_bus
                    .publish(rshell_api::AppEvent::PendingTunnelsSnapshot { rules: pending });
            }
            AppCommand::RestoreTunnel { session_id, rule } => {
                // UI 端确认启动一条 pending 隧道
                let ssh_client = self.session_service.get_ssh_client(session_id).await.ok();
                self.tunnel_manager.create_tunnel(session_id, rule, ssh_client).await?;
            }
            AppCommand::SuspendTunnel { tunnel_id } => {
                self.tunnel_manager.suspend_tunnel(tunnel_id).await?;
            }
            AppCommand::ResumeTunnel { tunnel_id } => {
                self.tunnel_manager.resume_tunnel(tunnel_id).await?;
            }

            // ===== 快速命令 =====
            AppCommand::ExecuteQuickCommand { command_id, target_sessions } => {
                self.execute_quick_command(command_id, &target_sessions).await?;
            }
            AppCommand::CreateQuickCommand { command } => {
                self.quick_command_service.create_command(command)?;
            }
            AppCommand::DeleteQuickCommand { command_id } => {
                self.quick_command_service.delete_command(command_id)?;
            }

            // ===== 触发器 =====
            AppCommand::CreateTrigger { trigger } => {
                self.trigger_engine.create_trigger(trigger)?;
            }
            AppCommand::DeleteTrigger { trigger_id } => {
                self.trigger_engine.delete_trigger(trigger_id)?;
            }
            AppCommand::ToggleTrigger { trigger_id } => {
                self.trigger_engine.toggle_trigger(trigger_id)?;
            }

            // ===== 撰写窗格 =====
            AppCommand::SendComposeText { content, target } => {
                self.compose_service
                    .send_text(&content, &target, &self.session_service, None)
                    .await?;
            }

            // ===== 脚本 =====
            AppCommand::ExecuteScript { code, session_id } => {
                self.execute_script(&code, session_id).await?;
            }

            // ===== 同步输入 =====
            AppCommand::ToggleSyncInput { session_ids } => {
                self.sync_input_service.toggle_sync_input(session_ids)?;
            }

            // ===== 安全：密钥管理 =====
            AppCommand::GenerateSshKey { name, key_type, passphrase } => {
                self.key_manager.generate_key(&name, key_type, passphrase.as_deref()).await?;
            }
            AppCommand::ImportPrivateKey { path, passphrase } => {
                self.key_manager.import_private_key(&path, passphrase.as_deref()).await?;
            }
            AppCommand::DeleteSshKey { key_id } => {
                self.key_manager.delete_key(key_id).await?;
            }
            AppCommand::ExportPublicKey { key_id } => {
                let public_key = self.key_manager.export_public_key(key_id).await?;
                self.event_bus.publish(rshell_api::AppEvent::PublicKeyExported { key_id, public_key });
            }

            // ===== 安全：主密码 =====
            AppCommand::SetupMasterPassword { password } => {
                self.master_password.setup(&password).await?;
            }
            AppCommand::VerifyMasterPassword { password } => {
                self.master_password.verify(&password).await?;
            }
            AppCommand::ChangeMasterPassword { old_password, new_password } => {
                self.master_password.change_password(&old_password, &new_password).await?;
            }

            // ===== 安全：主机密钥 =====
            AppCommand::TrustHostKey { host, port, key_type, public_key_blob, .. } => {
                self.host_key_manager.trust_host_key(&host, port, &key_type, &public_key_blob).await?;
            }
            AppCommand::DecideHostKey { decision_id, accept, permanent } => {
                // UI 端响应 HostKeyMismatch 事件:把决策 send 到 SshHandler 在
                // check_server_key 中阻塞的 oneshot。
                // 找不到 decision_id(超时/竞态/双重决策)时记录 warning 并忽略。
                let decision = HostKeyDecision {
                    // fingerprint/key_blob 这里没有从事件带过来;
                    // SshHandler 在 publish_request 时已经把它们打包进了
                    // HostKeyMismatch.public_key_blob,UI 端如果要展示校验靠该字段,
                    // 而不是 decision 本身。
                    fingerprint: String::new(),
                    key_blob: String::new(),
                    accept,
                    permanent,
                };
                if !self.host_key_registry.resolve(decision_id, decision) {
                    warn!(
                        decision_id = %decision_id,
                        "DecideHostKey: decision_id not found (already resolved or unknown)"
                    );
                }
            }
            AppCommand::DeleteHostKey { host, port } => {
                self.host_key_manager.delete_host_key(&host, port).await?;
            }

            // ===== 主题/配色方案 =====
            AppCommand::SetAppTheme { theme_name } => {
                self.theme_manager.set_theme(&theme_name).await?;
            }
            AppCommand::SetTerminalColorScheme { scheme_name } => {
                self.theme_manager.set_color_scheme(&scheme_name).await?;
            }
            AppCommand::ImportColorScheme { scheme } => {
                self.theme_manager.import_color_scheme(scheme).await?;
            }

            // ===== 多协议 =====
            AppCommand::ConnectTelnet { config } => {
                self.connect_telnet(config).await?;
            }
            AppCommand::ConnectSerial { config } => {
                self.connect_serial(config).await?;
            }

            // ===== 插件管理 =====
            AppCommand::ScanPlugins => {
                self.scan_plugins().await?;
            }
            AppCommand::LoadPlugin { plugin_id } => {
                self.load_plugin(&plugin_id).await?;
            }
            AppCommand::UnloadPlugin { plugin_id } => {
                self.unload_plugin(&plugin_id).await?;
            }
            AppCommand::EnablePlugin { plugin_id } => {
                // 启用 = 加载 + 激活
                self.load_plugin(&plugin_id).await?;
                self.event_bus.publish(rshell_api::AppEvent::PluginStateChanged {
                    plugin_id,
                    state: rshell_api::types::PluginState::Active,
                });
            }
            AppCommand::DisablePlugin { plugin_id } => {
                self.unload_plugin(&plugin_id).await?;
                self.event_bus.publish(rshell_api::AppEvent::PluginStateChanged {
                    plugin_id,
                    state: rshell_api::types::PluginState::Disabled,
                });
            }

            // ===== List / snapshot 拉取 =====
            // 这些分支只做"读 + publish"——不修改任何状态。供 UI 在 mount 时 /
            // refresh 按钮时调。
            AppCommand::ListSessions => {
                let sessions = self.session_service.list_sessions().await?;
                self.event_bus
                    .publish(rshell_api::AppEvent::SessionsSnapshot { sessions });
            }
            AppCommand::ListTunnels => {
                let tunnels = self.tunnel_manager.list_tunnels().await;
                self.event_bus
                    .publish(rshell_api::AppEvent::TunnelsSnapshot { tunnels });
            }
            AppCommand::ListKeys => {
                let keys = self.key_manager.list_keys().await;
                self.event_bus
                    .publish(rshell_api::AppEvent::KeysSnapshot { keys });
            }
            AppCommand::ListPlugins => {
                let plugins = self.plugin_loader.list_loaded().await;
                self.event_bus
                    .publish(rshell_api::AppEvent::PluginsSnapshot { plugins });
            }
            AppCommand::ListTriggers => {
                let triggers = self.trigger_engine.list_triggers()?;
                // 用 ActiveTunnelsChanged 类似的 fan-out 模式, 但 trigger 没有 snapshot 事件;
                // 复用 QuickCommandListChanged / TriggerListChanged 这两个已有事件。
                // 简化: 把 list 直接 publish 进 QuickCommandListChanged 一样的 channel 不可行
                // (类型不匹配), 所以这里只 publish TriggerListChanged 触发 UI 重新查。
                // 实际 list 结果通过单独事件分发。
                let _ = triggers;
                self.event_bus.publish(rshell_api::AppEvent::TriggerListChanged);
            }
            AppCommand::ListQuickCommands => {
                let cmds = self.quick_command_service.list_commands()?;
                let _ = cmds; // 复用 QuickCommandListChanged 通知 UI 重新拉
                self.event_bus
                    .publish(rshell_api::AppEvent::QuickCommandListChanged);
            }
            AppCommand::ListThemes => {
                let current_theme = self.theme_manager.current_theme().await.name;
                let current_scheme = self.theme_manager.current_color_scheme().await.name;
                let available_themes = self.theme_manager.list_themes().await;
                let available_schemes = self.theme_manager.list_color_schemes().await;
                self.event_bus.publish(rshell_api::AppEvent::ThemesSnapshot {
                    current_theme,
                    current_scheme,
                    available_themes,
                    available_schemes,
                });
            }
        }

        Ok(())
    }

    /// 执行快速命令
    async fn execute_quick_command(
        &self,
        command_id: Uuid,
        target_sessions: &[Uuid],
    ) -> Result<(), CoreError> {
        let data = self.quick_command_service.get_command_text(command_id)?;

        for session_id in target_sessions {
            if let Err(e) = self.session_service.send_data(*session_id, &data).await {
                debug!(session_id = %session_id, error = %e, "Failed to send quick command");
            }
        }

        info!(command_id = %command_id, targets = target_sessions.len(), "Quick command executed");
        Ok(())
    }

    /// 执行脚本
    async fn execute_script(&self, code: &str, session_id: Uuid) -> Result<(), CoreError> {
        let context = ScriptContext {
            session_id,
            target_sessions: vec![session_id],
            variables: std::collections::HashMap::new(),
        };

        let result = self.script_engine.execute_string(code, &context)?;

        self.event_bus.publish(rshell_api::AppEvent::ScriptFinished {
            session_id,
            result,
        });

        Ok(())
    }

    /// 连接 Telnet 会话
    async fn connect_telnet(&self, config: rshell_api::types::TelnetConfig) -> Result<(), CoreError> {
        info!("Connecting Telnet to {}:{}", config.host, config.port);

        // 生成会话 ID
        let session_id = Uuid::new_v4();

        // 创建终端实例
        let term_config = TerminalConfig::default();
        self.terminal_service.create_terminal(session_id, term_config)?;

        // 创建 Telnet 连接
        let mut telnet = TelnetConnection::new(&config.host, config.port);
        if !config.terminal_type.is_empty() {
            telnet.set_terminal_type(&config.terminal_type);
        }

        // 发布连接中状态
        self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
            session_id,
            state: rshell_api::types::ConnectionState::Connecting,
            info: None,
        });

        // 连接
        match telnet.connect().await {
            Ok(()) => {
                info!(session_id = %session_id, "Telnet connected");

                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: rshell_api::types::ConnectionState::Connected,
                    info: Some(rshell_api::types::ConnectionInfo {
                        protocol: rshell_api::types::Protocol::Telnet,
                        host: config.host,
                        port: config.port,
                        state: rshell_api::types::ConnectionState::Connected,
                        bytes_sent: 0,
                        bytes_received: 0,
                        latency_ms: None,
                    }),
                });
            }
            Err(e) => {
                warn!(session_id = %session_id, error = %e, "Telnet connect failed");
                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: rshell_api::types::ConnectionState::Disconnected,
                    info: None,
                });
                return Err(CoreError::ConnectionError(e.to_string()));
            }
        }

        Ok(())
    }

    /// 连接串口会话
    async fn connect_serial(&self, config: rshell_api::types::SerialConfig) -> Result<(), CoreError> {
        info!("Connecting Serial to {}", config.port);

        // 生成会话 ID
        let session_id = Uuid::new_v4();

        // 创建终端实例
        let term_config = TerminalConfig::default();
        self.terminal_service.create_terminal(session_id, term_config)?;

        // 转换 API 配置为协议配置
        let port_name = config.port.clone();
        let serial_config = ProtocolSerialConfig {
            port: config.port,
            baud_rate: config.baud_rate,
            data_bits: config.data_bits,
            stop_bits: config.stop_bits,
            parity: match config.parity {
                rshell_api::types::SerialParity::None => rshell_protocol::serial::SerialParity::None,
                rshell_api::types::SerialParity::Even => rshell_protocol::serial::SerialParity::Even,
                rshell_api::types::SerialParity::Odd => rshell_protocol::serial::SerialParity::Odd,
            },
            flow_control: match config.flow_control {
                rshell_api::types::SerialFlowControl::None => rshell_protocol::serial::SerialFlowControl::None,
                rshell_api::types::SerialFlowControl::Software => rshell_protocol::serial::SerialFlowControl::Software,
                rshell_api::types::SerialFlowControl::Hardware => rshell_protocol::serial::SerialFlowControl::Hardware,
            },
        };

        let mut serial = SerialConnection::new(serial_config);

        // 发布连接中状态
        self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
            session_id,
            state: rshell_api::types::ConnectionState::Connecting,
            info: None,
        });

        // 连接
        match serial.connect().await {
            Ok(()) => {
                info!(session_id = %session_id, "Serial connected");

                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: rshell_api::types::ConnectionState::Connected,
                    info: Some(rshell_api::types::ConnectionInfo {
                        protocol: rshell_api::types::Protocol::Serial,
                        host: port_name,
                        port: 0,
                        state: rshell_api::types::ConnectionState::Connected,
                        bytes_sent: 0,
                        bytes_received: 0,
                        latency_ms: None,
                    }),
                });
            }
            Err(e) => {
                warn!(session_id = %session_id, error = %e, "Serial connect failed");
                self.event_bus.publish(rshell_api::AppEvent::ConnectionStateChanged {
                    session_id,
                    state: rshell_api::types::ConnectionState::Disconnected,
                    info: None,
                });
                return Err(CoreError::ConnectionError(e.to_string()));
            }
        }

        Ok(())
    }

    /// 扫描插件
    async fn scan_plugins(&self) -> Result<(), CoreError> {
        info!("Scanning plugins...");

        match self.plugin_loader.scan_plugins().await {
            Ok(manifests) => {
                info!("Found {} plugins", manifests.len());
                self.event_bus.publish(rshell_api::AppEvent::PluginListUpdated);
            }
            Err(e) => {
                warn!("Plugin scan failed: {}", e);
            }
        }

        Ok(())
    }

    /// 加载插件
    async fn load_plugin(&self, plugin_id: &str) -> Result<(), CoreError> {
        info!("Loading plugin: {}", plugin_id);

        match self.plugin_loader.load_plugin(plugin_id).await {
            Ok(()) => {
                self.event_bus.publish(rshell_api::AppEvent::PluginStateChanged {
                    plugin_id: plugin_id.to_string(),
                    state: rshell_api::types::PluginState::Loaded,
                });
            }
            Err(e) => {
                self.event_bus.publish(rshell_api::AppEvent::PluginLoadFailed {
                    plugin_id: plugin_id.to_string(),
                    error: e.to_string(),
                });
                return Err(CoreError::Internal(format!("Plugin load failed: {}", e)));
            }
        }

        Ok(())
    }

    /// 卸载插件
    async fn unload_plugin(&self, plugin_id: &str) -> Result<(), CoreError> {
        info!("Unloading plugin: {}", plugin_id);

        match self.plugin_loader.unload_plugin(plugin_id).await {
            Ok(()) => {
                self.event_bus.publish(rshell_api::AppEvent::PluginStateChanged {
                    plugin_id: plugin_id.to_string(),
                    state: rshell_api::types::PluginState::Disabled,
                });
            }
            Err(e) => {
                warn!("Plugin unload failed: {}", e);
            }
        }

        Ok(())
    }

    /// 获取事件总线引用
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }

    /// 获取快速命令服务引用
    pub fn quick_command_service(&self) -> &Arc<QuickCommandService> {
        &self.quick_command_service
    }

    /// 获取触发器引擎引用
    pub fn trigger_engine(&self) -> &Arc<TriggerEngine> {
        &self.trigger_engine
    }

    /// 获取同步输入服务引用
    pub fn sync_input_service(&self) -> &Arc<SyncInputService> {
        &self.sync_input_service
    }

    /// 获取脚本引擎引用
    pub fn script_engine(&self) -> &Arc<ScriptEngine> {
        &self.script_engine
    }

    /// 获取密钥管理器引用
    pub fn key_manager(&self) -> &Arc<KeyManager> {
        &self.key_manager
    }

    /// 获取主密码管理器引用
    pub fn master_password(&self) -> &Arc<MasterPassword> {
        &self.master_password
    }

    /// 获取隧道管理器引用
    pub fn tunnel_manager(&self) -> &Arc<TunnelManager> {
        &self.tunnel_manager
    }

    /// 获取主机密钥管理器引用
    pub fn host_key_manager(&self) -> &Arc<HostKeyManager> {
        &self.host_key_manager
    }

    /// 获取主题管理器引用
    pub fn theme_manager(&self) -> &Arc<ThemeManager> {
        &self.theme_manager
    }
}

/// 将终端 buffer snapshot 序列化为可拷贝的纯文本
///
/// 每行末尾去除尾随空格；空行用单个 `\n` 表示；行间用 `\n` 分隔；
/// 末尾追加 `\n`。宽字符（>1 cell）当前按单字符处理 — 后续若有 width>1 cell
/// 的 cell 标志可在此扩展。
fn buffer_snapshot_to_text(snapshot: &TerminalBufferSnapshot) -> String {
    let mut out = String::with_capacity(snapshot.cells.len());
    for row in 0..snapshot.rows {
        let mut line = String::with_capacity(snapshot.cols);
        for col in 0..snapshot.cols {
            let idx = row * snapshot.cols + col;
            if idx < snapshot.cells.len() {
                let c = snapshot.cells[idx].character;
                if c == '\0' {
                    line.push(' ');
                } else {
                    line.push(c);
                }
            }
        }
        // 去掉行尾空格
        let trimmed = line.trim_end_matches(' ');
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}
