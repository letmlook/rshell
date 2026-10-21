//! 命令分发器（前端 → 后端）
//!
//! 前端发送命令，CommandDispatcher 路由到对应的 Service 处理。
//! 这是前后端分离架构中前端向后端发送请求的唯一通道。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::script::compose::ComposeService;
use crate::script::engine::{ScriptContext, ScriptEngine, ScriptHost};
use crate::script::quick_command::QuickCommandService;
use crate::script::sync_input::SyncInputService;
use crate::script::trigger_engine::TriggerEngine;
use crate::security::host_key_manager::HostKeyManager;
use crate::security::key_manager::KeyManager;
use crate::security::master_password::MasterPassword;
use crate::security::tunnel_manager::TunnelManager;
use crate::session::folder_service::FolderService;
use crate::session::service::SessionService;
use crate::terminal::service::TerminalService;
use crate::theme::ThemeManager;
use crate::transfer::service::TransferService;
use rshell_api::events::HostKeyDecisionState;
use rshell_api::{AppCommand, CommandOutcome, TrustHostKeyDecision};
use rshell_plugin_sdk::loader::PluginLoader;
use rshell_protocol::ssh::HostKeyDecision;
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
    pub folder_service: Arc<FolderService>,
    pub tunnel_manager: Arc<TunnelManager>,
    pub host_key_manager: Arc<HostKeyManager>,
    pub theme_manager: Arc<ThemeManager>,
    pub event_bus: Arc<EventBus>,
    pub host_key_registry: Arc<crate::security::host_key_decision::HostKeyDecisionRegistry>,
}

struct CoreScriptHost {
    sessions: Arc<SessionService>,
    quick_commands: Arc<QuickCommandService>,
}

impl ScriptHost for CoreScriptHost {
    fn send_text(&self, session_id: Uuid, text: &str) -> Result<(), CoreError> {
        tokio::runtime::Handle::current().block_on(self.sessions.send_data(
            session_id,
            None,
            text.as_bytes(),
        ))
    }

    fn list_sessions(&self) -> Result<Vec<Uuid>, CoreError> {
        Ok(tokio::runtime::Handle::current()
            .block_on(self.sessions.list_sessions())?
            .into_iter()
            .map(|s| s.id)
            .collect())
    }

    fn execute_quick_command(&self, command_id: Uuid, session_id: Uuid) -> Result<(), CoreError> {
        let data = self.quick_commands.get_command_text(command_id)?;
        tokio::runtime::Handle::current().block_on(self.sessions.send_data(session_id, None, &data))
    }
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
    folder_service: Arc<FolderService>,
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
            folder_service,
            tunnel_manager,
            host_key_manager,
            theme_manager,
            event_bus,
            host_key_registry,
        } = services;

        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rshell");
        let quick_command_service = Arc::new(QuickCommandService::with_path(
            event_bus.clone(),
            data_dir.join("quick-commands.json"),
        ));
        let compose_service = Arc::new(ComposeService::new(event_bus.clone()));
        // rhai::Engine 启用 sync feature 后 Arc<Dynamic> 内部走 Arc，可 Send+Sync；
        // 该 crate 在 Tauri 模式下由 app.manage() 直接持有（见设计 §1.2）。
        let script_engine = Arc::new(ScriptEngine::with_host(
            event_bus.clone(),
            Arc::new(CoreScriptHost {
                sessions: session_service.clone(),
                quick_commands: quick_command_service.clone(),
            }),
        ));
        let sync_input_service = Arc::new(SyncInputService::new(event_bus.clone()));

        // 插件目录：用户数据目录/plugins
        let plugins_dir = data_dir.join("plugins");
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
            folder_service,
            tunnel_manager,
            host_key_manager,
            theme_manager,
            plugin_loader,
            event_bus,
            host_key_registry,
        }
    }

    /// 初始化传输服务的 SSH 客户端提供函数
    pub fn initialize(&self) {
        let session_service = self.session_service.clone();
        let provider = Arc::new(
            move |session_id: Uuid| -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<crate::session::service::SshClientHandle, CoreError>,
                        > + Send,
                >,
            > {
                let svc = session_service.clone();
                Box::pin(async move { svc.get_ssh_client(session_id).await })
            },
        );
        self.transfer_service.set_ssh_client_provider(provider);
    }

    /// 分发命令（前端调用）
    ///
    /// 切片 1.2 起返回 `Result<CommandOutcome, CoreError>`（设计 §3.2 / D4）：
    /// 写命令返回 `Ok(CommandOutcome::None)`；读命令返回数据变体。
    /// 切片 1 仅迁移首批 7 个命令的分支；其余分支保持返回 `None`,
    /// 在切片 3+ 按功能域逐项完成 CommandOutcome 全貌（设计 §3.2 完整 13 变体）。
    #[instrument(skip(self, command), fields(command = ?command))]
    pub async fn dispatch(&self, command: AppCommand) -> Result<CommandOutcome, CoreError> {
        debug!("Dispatching command");

        match command {
            // ===== 会话命令 =====
            AppCommand::ConnectSession { session_id } => {
                self.session_service.connect(session_id).await?;
                // 连接成功 → 更新该主机密钥的「上次连接时间」。
                // `HostKeyEntry::last_seen` 说的是这件事，不更新它就等于字段撒谎。
                if let Ok(config) = self.session_service.get_session_config(session_id).await {
                    self.host_key_manager
                        .update_last_seen(&config.host, config.port)
                        .await;
                }
                Ok(CommandOutcome::None)
            }
            AppCommand::DisconnectSession { session_id } => {
                self.session_service.disconnect(session_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::CreateSession { config, credential } => {
                let id = self
                    .session_service
                    .create_session_with_credential(config, credential)
                    .await?;
                // 切片 1.2：CreateSession 此前只返回 Ok(()) —— 修复点见设计 §3.2
                Ok(CommandOutcome::SessionId(id))
            }
            AppCommand::UpdateSession {
                id,
                config,
                credential,
            } => {
                self.session_service
                    .update_session_with_credential(id, config, credential)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteSession { id } => {
                self.session_service.delete_session(id).await?;
                Ok(CommandOutcome::None)
            }

            // ===== 串口 =====
            AppCommand::ListSerialPorts => {
                let ports = rshell_protocol::serial::SerialConnection::list_ports()
                    .await
                    .map_err(|e| CoreError::ServiceError(e.to_string()))?;
                Ok(CommandOutcome::SerialPorts(ports))
            }

            // ===== 连接分组 =====
            AppCommand::ListFolders => {
                let folders = self.folder_service.list_folders()?;
                Ok(CommandOutcome::Folders(folders))
            }
            AppCommand::CreateFolder { name } => {
                let id = self.folder_service.create_folder(&name)?;
                Ok(CommandOutcome::FolderId(id))
            }
            AppCommand::RenameFolder { folder_id, name } => {
                self.folder_service.rename_folder(folder_id, &name)?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteFolder { folder_id } => {
                // 删分组不删连接：组内的连接先回到「默认」（folder_id = None），
                // 凭据保持不变（Keep）。
                let sessions = self.session_service.list_sessions().await?;
                for session in sessions {
                    if session.folder_id != Some(folder_id) {
                        continue;
                    }
                    let mut config = session;
                    config.folder_id = None;
                    let session_id = config.id;
                    // 单条会话更新失败（比如它正在连接）不该让整个删除卡住：
                    // 残留的 folder_id 会被前端按「分组已不存在」处理，落到「默认」组
                    if let Err(error) = self
                        .session_service
                        .update_session_with_credential(
                            config.id,
                            config,
                            rshell_api::types::CredentialUpdate::Keep,
                        )
                        .await
                    {
                        warn!(
                            session_id = %session_id,
                            %error,
                            "Failed to detach session from deleted folder; it stays in the default group"
                        );
                    }
                }
                self.folder_service.delete_folder(folder_id)?;
                Ok(CommandOutcome::None)
            }

            // ===== 终端命令 =====
            // terminal_id 缺省 = 主 pty（连接建立时创建的首标签）；给了就是
            // 指定那个 pty，输出/输入/尺寸都按它隔离。
            AppCommand::SendInput {
                session_id,
                terminal_id,
                data,
            } => {
                self.session_service
                    .send_data(session_id, terminal_id, &data)
                    .await?;
                if self.sync_input_service.is_sync_active()? {
                    self.sync_input_service
                        .send_to_synced_sessions(session_id, &data, &self.session_service)
                        .await?;
                }
                Ok(CommandOutcome::None)
            }
            AppCommand::ResizeTerminal {
                session_id,
                terminal_id,
                cols,
                rows,
            } => {
                // 尺寸登记键用 terminal_id 优先：多标签时不能都记到 session_id 上
                let key = terminal_id.unwrap_or(session_id);
                self.terminal_service.resize(key, cols, rows)?;
                if matches!(
                    self.session_service.get_state(session_id).await,
                    Ok(rshell_api::types::ConnectionState::Connected)
                ) {
                    self.session_service
                        .resize_terminal(session_id, terminal_id, cols as u32, rows as u32)
                        .await?;
                }
                Ok(CommandOutcome::None)
            }
            AppCommand::OpenTerminal {
                session_id,
                terminal_id,
                cols,
                rows,
            } => {
                self.session_service
                    .open_terminal(session_id, terminal_id, cols as u32, rows as u32)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::CloseTerminal {
                session_id,
                terminal_id,
            } => {
                self.session_service
                    .close_terminal(session_id, terminal_id)
                    .await?;
                Ok(CommandOutcome::None)
            }

            // ===== 文件传输命令 =====
            AppCommand::EnqueueUpload {
                local,
                remote,
                session_id,
                conflict,
            } => {
                self.transfer_service
                    .enqueue_upload(local, remote, session_id, conflict)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::EnqueueDownload {
                remote,
                local,
                session_id,
                conflict,
            } => {
                self.transfer_service
                    .enqueue_download(remote, local, session_id, conflict)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::PauseTransfer { task_id } => {
                self.transfer_service.pause_transfer(task_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ResumeTransfer { task_id } => {
                self.transfer_service.resume_transfer(task_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::CancelTransfer { task_id } => {
                self.transfer_service.cancel_transfer(task_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::RemoveTransfer { task_id } => {
                self.transfer_service.remove_transfer(task_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::RetryTransfer { task_id } => {
                let new_task_id = self.transfer_service.enqueue_retry(task_id).await?;
                Ok(CommandOutcome::TaskId(new_task_id))
            }
            AppCommand::BrowseRemoteDir { session_id, path } => {
                let entries = self
                    .session_service
                    .browse_remote_dir(session_id, &path)
                    .await?;
                Ok(CommandOutcome::RemoteDir { path, entries })
            }
            AppCommand::CreateRemoteDirectory { session_id, path } => {
                self.session_service
                    .create_remote_directory(session_id, &path)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::GetRemoteHomeDir { session_id } => {
                let path = self.session_service.remote_home_dir(session_id).await?;
                Ok(CommandOutcome::RemoteHomeDir(path))
            }
            AppCommand::DeleteRemoteEntry { session_id, path } => {
                self.session_service
                    .delete_remote_entry(session_id, &path)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ListTransfers => {
                let tasks = self
                    .transfer_service
                    .list_tasks()
                    .await
                    .into_iter()
                    .map(Into::into)
                    .collect();
                Ok(CommandOutcome::Transfers(tasks))
            }

            // ===== 隧道命令 =====
            AppCommand::CreateTunnel { session_id, rule } => {
                let ssh_client = self.session_service.get_ssh_client(session_id).await?;
                self.tunnel_manager
                    .create_tunnel(session_id, rule, Some(ssh_client))
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::CloseTunnel { tunnel_id } => {
                self.tunnel_manager.close_tunnel(tunnel_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ListPendingTunnels => {
                let pending = self.tunnel_manager.restore_pending_rules_info().await;
                info!(
                    count = pending.rules.len(),
                    unsupported = pending.unsupported.len(),
                    "UI requested pending tunnels"
                );
                Ok(CommandOutcome::PendingTunnels(pending))
            }
            AppCommand::RestoreTunnel { session_id, rule } => {
                let ssh_client = self.session_service.get_ssh_client(session_id).await?;
                self.tunnel_manager
                    .create_tunnel(session_id, rule, Some(ssh_client))
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::SuspendTunnel { tunnel_id } => {
                self.tunnel_manager.suspend_tunnel(tunnel_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ResumeTunnel { tunnel_id } => {
                self.tunnel_manager.resume_tunnel(tunnel_id).await?;
                Ok(CommandOutcome::None)
            }

            // ===== 快速命令 =====
            AppCommand::ExecuteQuickCommand {
                command_id,
                target_sessions,
            } => {
                self.execute_quick_command(command_id, &target_sessions)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::CreateQuickCommand { command } => {
                self.quick_command_service.create_command(command)?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteQuickCommand { command_id } => {
                self.quick_command_service.delete_command(command_id)?;
                Ok(CommandOutcome::None)
            }

            // ===== 触发器 =====
            AppCommand::CreateTrigger { trigger } => {
                self.trigger_engine.create_trigger(trigger)?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteTrigger { trigger_id } => {
                self.trigger_engine.delete_trigger(trigger_id)?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ToggleTrigger { trigger_id } => {
                self.trigger_engine.toggle_trigger(trigger_id)?;
                Ok(CommandOutcome::None)
            }

            // ===== 撰写窗格 =====
            AppCommand::SendComposeText {
                content,
                target,
                active_session_id,
            } => {
                self.compose_service
                    .send_text(&content, &target, &self.session_service, active_session_id)
                    .await?;
                Ok(CommandOutcome::None)
            }

            // ===== 脚本 =====
            AppCommand::ExecuteScript { code, session_id } => {
                self.execute_script(&code, session_id).await?;
                Ok(CommandOutcome::None)
            }

            // ===== 同步输入 =====
            AppCommand::ToggleSyncInput { session_ids } => {
                self.sync_input_service.toggle_sync_input(session_ids)?;
                Ok(CommandOutcome::None)
            }
            AppCommand::GetSyncInputSessions => {
                let session_ids = self.sync_input_service.get_sync_sessions()?;
                Ok(CommandOutcome::SyncSessions(session_ids))
            }

            // ===== 安全：密钥管理 =====
            AppCommand::GenerateSshKey {
                name,
                key_type,
                passphrase,
            } => {
                self.key_manager
                    .generate_key(&name, key_type, passphrase.as_deref())
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ImportPrivateKey { path, passphrase } => {
                self.key_manager
                    .import_private_key(&path, passphrase.as_deref())
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteSshKey { key_id } => {
                self.key_manager.delete_key(key_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ExportPublicKey { key_id } => {
                let public_key = self.key_manager.export_public_key(key_id).await?;
                self.event_bus
                    .publish(rshell_api::AppEvent::PublicKeyExported { key_id, public_key });
                Ok(CommandOutcome::None)
            }

            // ===== 安全：主密码 =====
            AppCommand::SetupMasterPassword { password } => {
                self.master_password.setup(&password).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::VerifyMasterPassword { password } => {
                let ok = self.master_password.verify(&password).await?;
                Ok(CommandOutcome::Verified(ok))
            }
            AppCommand::ChangeMasterPassword {
                old_password,
                new_password,
            } => {
                self.master_password
                    .change_password(&old_password, &new_password)
                    .await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::DisableMasterPassword { password } => {
                self.master_password.disable(&password).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::LockMasterPassword => {
                self.master_password.lock().await;
                Ok(CommandOutcome::None)
            }
            AppCommand::GetMasterPasswordState => {
                let state = self.master_password.get_state().await;
                Ok(CommandOutcome::MasterPasswordState(
                    state.as_str().to_string(),
                ))
            }

            // ===== 安全：主机密钥 =====
            // PROB-06:decision 必须消费。原实现用 `..` 丢弃 decision,导致传
            // Reject/TrustOnce 也会把密钥永久写入 known_hosts。仅 TrustPermanent
            // 允许落盘;Reject/TrustOnce 显式报错、不产生任何持久化条目 ——
            // 会话内信任须走带 decision_id 的 DecideHostKey 决策链。
            AppCommand::TrustHostKey {
                host,
                port,
                key_type,
                public_key_blob,
                decision,
            } => match decision {
                TrustHostKeyDecision::TrustPermanent => {
                    self.host_key_manager
                        .trust_host_key(&host, port, &key_type, &public_key_blob)
                        .await?;
                    Ok(CommandOutcome::None)
                }
                TrustHostKeyDecision::TrustOnce => Err(CoreError::InvalidState(
                    "TrustOnce is not accepted by the offline trust channel: \
                         session-scoped trust must go through the DecideHostKey \
                         decision chain; nothing was written to known_hosts"
                        .into(),
                )),
                TrustHostKeyDecision::Reject => Err(CoreError::InvalidState(format!(
                    "Host key trust for {host}:{port} was rejected; \
                         nothing was written to known_hosts"
                ))),
            },
            AppCommand::CancelHostKey { decision_id } => {
                if self.host_key_registry.cancel(decision_id) {
                    Ok(CommandOutcome::None)
                } else {
                    Err(CoreError::NotFound(format!(
                        "Host key decision {decision_id} is no longer pending"
                    )))
                }
            }
            AppCommand::DecideHostKey {
                decision_id,
                accept,
                permanent,
            } => {
                let request = self
                    .host_key_registry
                    .request_info(decision_id)
                    .ok_or_else(|| {
                        CoreError::NotFound(format!(
                            "Host key decision {decision_id} is no longer pending"
                        ))
                    })?;
                let claimed = if accept && permanent {
                    HostKeyDecisionState::DecidedTrustAlways
                } else if accept {
                    HostKeyDecisionState::DecidedTrustOnce
                } else {
                    HostKeyDecisionState::DecidedReject
                };
                if self.host_key_registry.claim(decision_id, claimed).is_none() {
                    return Err(CoreError::NotFound(format!(
                        "Host key decision {decision_id} is no longer pending"
                    )));
                }
                let decision = HostKeyDecision {
                    fingerprint: request.fingerprint.clone(),
                    key_blob: request.public_key_blob.clone(),
                    accept,
                    permanent,
                };
                if accept && permanent {
                    let mut parts = request.public_key_blob.split_whitespace();
                    let key_type = parts.next().ok_or_else(|| {
                        self.host_key_registry.release_claim(decision_id, claimed);
                        CoreError::InvalidState("Host key type is missing".into())
                    })?;
                    let key_blob = parts.next().ok_or_else(|| {
                        self.host_key_registry.release_claim(decision_id, claimed);
                        CoreError::InvalidState("Host key blob is missing".into())
                    })?;
                    if let Err(error) = self
                        .host_key_manager
                        .trust_host_key(&request.host, request.port, key_type, key_blob)
                        .await
                    {
                        self.host_key_registry.release_claim(decision_id, claimed);
                        return Err(CoreError::HostKeyTrustPersistenceFailed(error.to_string()));
                    }
                }
                if !self
                    .host_key_registry
                    .settle_claimed(decision_id, claimed, decision)
                {
                    return Err(CoreError::NotFound(format!(
                        "Host key decision {decision_id} is no longer pending"
                    )));
                }
                Ok(CommandOutcome::None)
            }
            AppCommand::DeleteHostKey { host, port } => {
                self.host_key_manager.delete_host_key(&host, port).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ListHostKeys => {
                let entries = self.host_key_manager.list_hosts().await;
                Ok(CommandOutcome::HostKeys(entries))
            }

            // ===== 主题/配色方案 =====
            AppCommand::SetAppTheme { theme_name } => {
                self.theme_manager.set_theme(&theme_name).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::SetTerminalColorScheme { scheme_name } => {
                self.theme_manager.set_color_scheme(&scheme_name).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::ImportColorScheme { scheme } => {
                self.theme_manager.import_color_scheme(scheme).await?;
                Ok(CommandOutcome::None)
            }

            // ===== 插件管理 =====
            AppCommand::ScanPlugins => {
                self.scan_plugins().await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::LoadPlugin { plugin_id } => {
                self.load_plugin(&plugin_id).await?;
                Ok(CommandOutcome::None)
            }
            AppCommand::UnloadPlugin { plugin_id } => {
                self.unload_plugin(&plugin_id).await?;
                Ok(CommandOutcome::None)
            }
            // ===== List / snapshot 拉取 =====
            // 切片 1.2 首批迁移：ListSessions / ListTriggers / ListQuickCommands 返回 CommandOutcome;
            // 其余保持 publish（向后兼容旧事件订阅），切片 3+ 逐项迁移。
            AppCommand::ListSessions => {
                let sessions = self.session_service.list_sessions().await?;
                // 设计 §3.2 死循环修复：直接返回数据,不再 publish *Snapshot。
                Ok(CommandOutcome::Sessions(sessions))
            }
            AppCommand::ListSessionLoadIssues => Ok(CommandOutcome::SessionLoadIssues(
                self.session_service.list_load_issues().await,
            )),
            AppCommand::RetrySessionLoad => {
                self.session_service.load_from_disk().await;
                Ok(CommandOutcome::None)
            }
            AppCommand::ListTunnels => {
                let tunnels = self.tunnel_manager.list_tunnels().await;
                Ok(CommandOutcome::Tunnels(tunnels))
            }
            AppCommand::ListKeys => {
                let keys = self.key_manager.list_keys().await;
                Ok(CommandOutcome::Keys(keys))
            }
            AppCommand::ListPlugins => {
                let plugins = self.plugin_loader.list_plugins().await;
                Ok(CommandOutcome::Plugins(plugins))
            }
            AppCommand::ListTriggers => {
                let triggers = self.trigger_engine.list_triggers()?;
                Ok(CommandOutcome::Triggers(triggers))
            }
            AppCommand::ListQuickCommands => {
                let cmds = self.quick_command_service.list_commands()?;
                Ok(CommandOutcome::QuickCommands(cmds))
            }
            AppCommand::ListThemes => {
                let theme = self.theme_manager.current_theme().await;
                let palette = self.theme_manager.current_color_scheme().await;
                let available_themes = self.theme_manager.list_themes().await;
                let available_schemes = self.theme_manager.list_color_schemes().await;
                Ok(CommandOutcome::Themes(rshell_api::types::ThemeInfo {
                    current_theme: theme.name,
                    current_scheme: palette.name.clone(),
                    current_colors: theme.colors,
                    current_palette: palette,
                    available_themes,
                    available_schemes,
                }))
            }
        }
    }

    /// 执行快速命令
    async fn execute_quick_command(
        &self,
        command_id: Uuid,
        target_sessions: &[Uuid],
    ) -> Result<(), CoreError> {
        let data = self.quick_command_service.get_command_text(command_id)?;

        for session_id in target_sessions {
            self.session_service
                .send_data(*session_id, None, &data)
                .await?;
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

        let engine = self.script_engine.clone();
        let code = code.to_owned();
        let result = tokio::task::spawn_blocking(move || engine.execute_string(&code, &context))
            .await
            .map_err(|e| CoreError::Internal(e.to_string()))??;

        self.event_bus
            .publish(rshell_api::AppEvent::ScriptFinished {
                session_id,
                result: result.clone(),
            });

        if !result.success {
            return Err(CoreError::InvalidState(
                result.error.unwrap_or_else(|| "Script failed".into()),
            ));
        }

        Ok(())
    }

    /// 扫描插件
    async fn scan_plugins(&self) -> Result<(), CoreError> {
        info!("Scanning plugins...");

        match self.plugin_loader.scan_plugins().await {
            Ok(manifests) => {
                info!("Found {} plugins", manifests.len());
                self.event_bus
                    .publish(rshell_api::AppEvent::PluginListUpdated);
            }
            Err(e) => {
                warn!("Plugin scan failed: {}", e);
                return Err(CoreError::Internal(format!("Plugin scan failed: {}", e)));
            }
        }

        Ok(())
    }

    /// 加载插件
    async fn load_plugin(&self, plugin_id: &str) -> Result<(), CoreError> {
        info!("Loading plugin: {}", plugin_id);

        match self.plugin_loader.load_plugin(plugin_id).await {
            Ok(()) => {
                self.event_bus
                    .publish(rshell_api::AppEvent::PluginStateChanged {
                        plugin_id: plugin_id.to_string(),
                        state: rshell_api::types::PluginState::Loaded,
                    });
            }
            Err(e) => {
                self.event_bus
                    .publish(rshell_api::AppEvent::PluginLoadFailed {
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
                self.event_bus
                    .publish(rshell_api::AppEvent::PluginStateChanged {
                        plugin_id: plugin_id.to_string(),
                        state: rshell_api::types::PluginState::Disabled,
                    });
            }
            Err(e) => {
                warn!("Plugin unload failed: {}", e);
                return Err(CoreError::Internal(format!("Plugin unload failed: {}", e)));
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
    pub fn session_service(&self) -> &Arc<SessionService> {
        &self.session_service
    }

    pub fn folder_service(&self) -> &Arc<FolderService> {
        &self.folder_service
    }

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

// 切片 2.1 删除:`buffer_snapshot_to_text` 与 CopySelection arm 一并移除
// —— 设计 §5 上移剪贴板到前端,xterm.js 自持选区,后端无需序列化整屏文本。

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_protocol::ssh::HostKeyDecisionSink;

    /// 构造一个不触碰真实用户目录的最小 dispatcher(无持久化仓库)。
    fn make_dispatcher(dir: &std::path::Path) -> CommandDispatcher {
        let event_bus = Arc::new(EventBus::new());
        let terminal_service = Arc::new(TerminalService::new(event_bus.clone()));
        let trigger_engine = Arc::new(TriggerEngine::new(event_bus.clone()));
        let host_key_registry = Arc::new(
            crate::security::host_key_decision::HostKeyDecisionRegistry::new(event_bus.clone()),
        );
        let session_service = Arc::new(SessionService::new(
            event_bus.clone(),
            terminal_service.clone(),
            trigger_engine.clone(),
            host_key_registry.clone(),
        ));
        CommandDispatcher::new(Services {
            session_service,
            terminal_service,
            transfer_service: Arc::new(TransferService::new(event_bus.clone())),
            trigger_engine,
            key_manager: Arc::new(KeyManager::new(dir.join("keys"), event_bus.clone())),
            master_password: Arc::new(MasterPassword::new(event_bus.clone())),
            folder_service: Arc::new(FolderService::with_path(
                event_bus.clone(),
                dir.join("folders.json"),
            )),
            tunnel_manager: Arc::new(TunnelManager::new(event_bus.clone())),
            host_key_manager: Arc::new(HostKeyManager::new(dir.join("known_hosts"))),
            theme_manager: Arc::new(ThemeManager::new(event_bus.clone())),
            event_bus,
            host_key_registry,
        })
    }

    /// PROB-05:VerifyMasterPassword 必须把 verify() 的 bool 透传为
    /// `CommandOutcome::Verified`,薄壳(commands.rs)依赖该分支,否则报 outcome_mismatch。
    /// 口令在运行期随机生成,不在源码中写入任何凭据字面量。
    #[tokio::test]
    async fn verify_master_password_yields_verified_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        // 两个互不相同的运行期随机口令:一个用于 setup,另一个必然验证失败。
        let correct = format!("pw-{}", Uuid::new_v4());
        let wrong = format!("pw-{}", Uuid::new_v4());
        assert_ne!(correct, wrong);

        dispatcher
            .dispatch(AppCommand::SetupMasterPassword {
                password: correct.clone(),
            })
            .await
            .unwrap();

        let ok = dispatcher
            .dispatch(AppCommand::VerifyMasterPassword { password: correct })
            .await
            .unwrap();
        assert!(matches!(ok, CommandOutcome::Verified(true)));

        let mismatch = dispatcher
            .dispatch(AppCommand::VerifyMasterPassword { password: wrong })
            .await
            .unwrap();
        assert!(matches!(mismatch, CommandOutcome::Verified(false)));
    }

    /// PROB-17:SendComposeText 在目标会话全部未连接时必须返回错误,
    /// 薄壳 commands.rs 把该 Err 映射为 IpcError,前端 invoke 即拒绝,
    /// 不得静默成功。
    #[tokio::test]
    async fn send_compose_text_all_targets_unreachable_fails() {
        use rshell_api::types::ComposeTarget;

        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        // 两个从未创建的会话 ID:全部不可达
        let result = dispatcher
            .dispatch(AppCommand::SendComposeText {
                content: "hello".into(),
                target: ComposeTarget::SelectedSessions(vec![Uuid::new_v4(), Uuid::new_v4()]),
                active_session_id: None,
            })
            .await;

        assert!(result.is_err());
    }

    /// `ComposeTarget::CurrentSession` 必须真的能解析出当前会话。
    ///
    /// 此前 dispatcher 把 `active_session_id` 写死为 `None`，于是
    /// `CurrentSession` 永远走 `No active session` 分支——「发送到当前会话」
    /// 这条路径从 IPC 进来即必然失败。带真实会话 id 时不应再报这条错。
    #[tokio::test]
    async fn send_compose_text_current_session_uses_active_session_id() {
        use rshell_api::types::ComposeTarget;

        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        // 会话不存在 → 失败原因应是「会话不存在」而不是「没有当前会话」
        let result = dispatcher
            .dispatch(AppCommand::SendComposeText {
                content: "hello".into(),
                target: ComposeTarget::CurrentSession,
                active_session_id: Some(Uuid::new_v4()),
            })
            .await;

        let error = result.expect_err("unreachable session must fail");
        assert!(
            !error.to_string().contains("No active session"),
            "active_session_id must reach the compose service, got: {}",
            error
        );
    }

    /// 缺省 `active_session_id` 时 `CurrentSession` 依然不可用——
    /// 锁死这条错误路径，前端漏传 id 时能看到真实原因而不是静默成功。
    #[tokio::test]
    async fn send_compose_text_current_session_without_id_reports_no_active_session() {
        use rshell_api::types::ComposeTarget;

        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        let result = dispatcher
            .dispatch(AppCommand::SendComposeText {
                content: "hello".into(),
                target: ComposeTarget::CurrentSession,
                active_session_id: None,
            })
            .await;

        let error = result.expect_err("current session without id must fail");
        assert!(
            error.to_string().contains("No active session"),
            "expected the no-active-session error, got: {}",
            error
        );
    }

    /// ListHostKeys 必须真的列出已信任的主机密钥。
    ///
    /// `DeleteHostKey` 单独存在时用户无从知道哪些条目可以删——known_hosts 是
    /// 应用数据目录里的文件，用户不直接编辑它。
    #[tokio::test]
    async fn list_host_keys_returns_trusted_entries() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        // 先落一条信任记录
        dispatcher
            .host_key_manager()
            .trust_host_key("10.0.0.1", 22, "ssh-ed25519", "SHA256:abc")
            .await
            .unwrap();

        let outcome = dispatcher.dispatch(AppCommand::ListHostKeys).await.unwrap();
        let CommandOutcome::HostKeys(entries) = outcome else {
            panic!("expected HostKeys outcome");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, "10.0.0.1");
        assert_eq!(entries[0].port, 22);
        assert_eq!(entries[0].fingerprint, "SHA256:abc");
    }

    /// 没有信任过任何主机时列表为空（不是报错，也不是残留旧数据）。
    #[tokio::test]
    async fn list_host_keys_is_empty_without_trust() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        let outcome = dispatcher.dispatch(AppCommand::ListHostKeys).await.unwrap();
        let CommandOutcome::HostKeys(entries) = outcome else {
            panic!("expected HostKeys outcome");
        };
        assert!(entries.is_empty());
    }

    /// 分组命令：列出 / 新建 / 重命名。
    #[tokio::test]
    async fn folder_commands_manage_folders() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        let outcome = dispatcher.dispatch(AppCommand::ListFolders).await.unwrap();
        assert!(matches!(outcome, CommandOutcome::Folders(ref folders) if folders.is_empty()));

        let outcome = dispatcher
            .dispatch(AppCommand::CreateFolder {
                name: "生产".into(),
            })
            .await
            .unwrap();
        let CommandOutcome::FolderId(id) = outcome else {
            panic!("expected FolderId outcome");
        };

        let outcome = dispatcher.dispatch(AppCommand::ListFolders).await.unwrap();
        let CommandOutcome::Folders(folders) = outcome else {
            panic!("expected Folders outcome");
        };
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id, id);
        assert_eq!(folders[0].name, "生产");

        dispatcher
            .dispatch(AppCommand::RenameFolder {
                folder_id: id,
                name: "生产环境".into(),
            })
            .await
            .unwrap();
        let outcome = dispatcher.dispatch(AppCommand::ListFolders).await.unwrap();
        let CommandOutcome::Folders(folders) = outcome else {
            panic!("expected Folders outcome");
        };
        assert_eq!(folders[0].name, "生产环境");
    }

    /// 删除分组必须把组内的连接挪回「默认」（folder_id = None），
    /// **不能**连带删掉连接——连接是用户的数据，分组只是组织方式。
    #[tokio::test]
    async fn deleting_folder_moves_sessions_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());

        let outcome = dispatcher
            .dispatch(AppCommand::CreateFolder {
                name: "生产".into(),
            })
            .await
            .unwrap();
        let CommandOutcome::FolderId(folder_id) = outcome else {
            panic!("expected FolderId outcome");
        };

        // 建一条属于该分组的连接
        let session_id = dispatcher
            .session_service()
            .create_session_with_credential(
                rshell_api::types::SessionConfig {
                    id: Uuid::new_v4(),
                    name: "prod-host".into(),
                    folder_id: Some(folder_id),
                    host: "10.0.0.1".into(),
                    port: 22,
                    protocol: rshell_api::types::Protocol::SSH,
                    auth_method: rshell_api::types::AuthMethod::Password {
                        username: "alice".into(),
                        has_password: false,
                    },
                    serial_config: None,
                },
                None,
            )
            .await
            .unwrap();

        dispatcher
            .dispatch(AppCommand::DeleteFolder { folder_id })
            .await
            .unwrap();

        // 连接还在，只是回到了「默认」
        let sessions = dispatcher.session_service().list_sessions().await.unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "deleting a folder must not delete sessions"
        );
        assert_eq!(sessions[0].id, session_id);
        assert_eq!(sessions[0].folder_id, None);

        // 分组本身也没了
        let outcome = dispatcher.dispatch(AppCommand::ListFolders).await.unwrap();
        let CommandOutcome::Folders(folders) = outcome else {
            panic!("expected Folders outcome");
        };
        assert!(folders.is_empty());
    }

    /// 空分组名必须被拒（否则界面会出现无法区分的空分组）。
    #[tokio::test]
    async fn create_folder_rejects_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());
        let result = dispatcher
            .dispatch(AppCommand::CreateFolder { name: "   ".into() })
            .await;
        assert!(result.is_err());
    }

    /// PROB-06:TrustHostKey 必须按 decision 分派,三种 decision 的落盘行为各断言一次。
    /// Reject/TrustOnce 不得产生 known_hosts 条目(内存与磁盘均无),
    /// 仅 TrustPermanent 允许走 trust_host_key 落盘。
    #[tokio::test]
    async fn trust_host_key_only_persists_on_trust_permanent() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = make_dispatcher(dir.path());
        let known_hosts = dir.path().join("known_hosts");

        // Reject:报错且不落盘
        let rejected = dispatcher
            .dispatch(AppCommand::TrustHostKey {
                host: "reject.test".into(),
                port: 2201,
                key_type: "ssh-ed25519".into(),
                public_key_blob: "AAAAreject".into(),
                decision: TrustHostKeyDecision::Reject,
            })
            .await
            .unwrap_err();
        assert!(matches!(rejected, CoreError::InvalidState(_)));

        // TrustOnce:报错且不落盘(会话内信任须走带 decision_id 的 DecideHostKey)
        let once = dispatcher
            .dispatch(AppCommand::TrustHostKey {
                host: "once.test".into(),
                port: 2202,
                key_type: "ssh-ed25519".into(),
                public_key_blob: "AAAAonce".into(),
                decision: TrustHostKeyDecision::TrustOnce,
            })
            .await
            .unwrap_err();
        assert!(matches!(once, CoreError::InvalidState(_)));

        // 两次非永久决策后:磁盘上没有 known_hosts 文件,内存中也没有任何条目
        assert!(
            !known_hosts.exists(),
            "Reject/TrustOnce 不得创建 known_hosts 文件"
        );
        assert!(
            dispatcher.host_key_manager().list_hosts().await.is_empty(),
            "Reject/TrustOnce 不得产生内存信任条目"
        );

        // TrustPermanent:唯一允许落盘的决策
        dispatcher
            .dispatch(AppCommand::TrustHostKey {
                host: "perm.test".into(),
                port: 2203,
                key_type: "ssh-ed25519".into(),
                public_key_blob: "AAAAperm".into(),
                decision: TrustHostKeyDecision::TrustPermanent,
            })
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(&known_hosts).unwrap();
        assert!(on_disk.contains("[perm.test]:2203 ssh-ed25519 AAAAperm"));
    }

    #[tokio::test]
    async fn permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let known_hosts = dir.path().join("known_hosts");
        let tmp = dir.path().join("known_hosts.tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let dispatcher = make_dispatcher(dir.path());
        let registry = dispatcher.host_key_registry.clone();
        let (decision_id, receiver) = registry.register();
        registry.publish_request(rshell_protocol::ssh::HostKeyDecisionRequest {
            decision_id,
            host: "fallback.test".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:fallback".into(),
            expected: String::new(),
            public_key_blob: "ssh-ed25519 AAAAfallback".into(),
        });

        let failure = dispatcher
            .dispatch(AppCommand::DecideHostKey {
                decision_id,
                accept: true,
                permanent: true,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            failure,
            CoreError::HostKeyTrustPersistenceFailed(_)
        ));
        assert_eq!(
            registry.state(decision_id),
            Some(HostKeyDecisionState::Pending)
        );

        dispatcher
            .dispatch(AppCommand::DecideHostKey {
                decision_id,
                accept: true,
                permanent: false,
            })
            .await
            .unwrap();
        assert!(receiver.await.unwrap().accept);
        assert!(!known_hosts.exists());
    }
}
