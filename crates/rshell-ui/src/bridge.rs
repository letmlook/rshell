//! UI ↔ Backend 桥接模块
//!
//! 负责：
//! 1. 初始化后端服务（EventBus、CommandDispatcher 等）
//! 2. 提供 UI → Backend 的命令通道（mpsc）
//! 3. 提供 Backend → UI 的事件转发（共享队列）

use rshell_api::{AppCommand, AppEvent};
use rshell_core::CommandDispatcher;
use rshell_core::event_bus::EventBus as EventBusImpl;
use rshell_core::security::key_manager::KeyManager;
use rshell_core::security::master_password::MasterPassword;
use rshell_core::security::tunnel_manager::TunnelManager;
use rshell_core::security::host_key_manager::HostKeyManager;
use rshell_core::session::service::SessionService;
use rshell_core::terminal::service::TerminalService;
use rshell_core::theme::ThemeManager;
use rshell_core::transfer::service::TransferService;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;
use tokio::task::LocalSet;
use tracing::{info, error};

/// 共享事件队列（Backend 写入，UI 读取）
pub type SharedEventQueue = Arc<Mutex<Vec<AppEvent>>>;

/// UI ↔ Backend 桥接
#[derive(Clone)]
pub struct AppBridge {
    /// 命令发送端（UI 持有）
    command_tx: mpsc::UnboundedSender<AppCommand>,
    /// 共享事件队列（UI 在 render 时排空）
    event_queue: SharedEventQueue,
    /// 事件总线引用（用于直接订阅特定事件）
    #[allow(dead_code)]
    event_bus: Arc<EventBusImpl>,
}

// gpui::Global 实现 — 让 view 层通过 cx.global::<AppBridge>() 拿到桥接
impl gpui::Global for AppBridge {}

impl AppBridge {
    /// 发送命令到后端
    pub fn send_command(&self, command: AppCommand) {
        if let Err(e) = self.command_tx.send(command) {
            error!("Failed to send command to backend: {}", e);
        }
    }

    /// 排空事件队列，返回所有待处理事件
    pub fn drain_events(&self) -> Vec<AppEvent> {
        let mut queue = self.event_queue.lock().unwrap();
        std::mem::take(&mut *queue)
    }

    /// 获取事件总线引用（用于直接订阅特定事件）
    #[allow(dead_code)]
    pub fn event_bus(&self) -> &Arc<EventBusImpl> {
        &self.event_bus
    }
}

/// 初始化后端服务并创建桥接
///
/// 后端运行在专用线程上（因为 rhai ScriptEngine 不是 Send，不能跨线程移动）。
/// 所有服务（CommandDispatcher 等）在后台线程内部创建。
pub fn init_backend() -> (AppBridge, ()) {
    info!("Initializing backend services...");

    // 创建共享的事件总线（EventBus 本身是 Send + Sync）
    let event_bus = Arc::new(EventBusImpl::new());

    // 创建共享事件队列
    let event_queue: SharedEventQueue = Arc::new(Mutex::new(Vec::new()));

    // 订阅所有事件，转发到共享队列
    let event_queue_for_sub = event_queue.clone();
    event_bus.subscribe(move |event: &AppEvent| {
        let mut queue = event_queue_for_sub.lock().unwrap();
        queue.push(event.clone());
    });

    // 创建命令通道
    let (command_tx, command_rx) = mpsc::unbounded_channel::<AppCommand>();

    // 准备数据目录路径（Send 类型，可以跨线程）
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rshell");

    // 在专用线程上创建所有服务并运行命令处理循环
    // 这样 CommandDispatcher（包含 !Send 的 rhai ScriptEngine）不需要跨线程移动
    let event_bus_for_thread = event_bus.clone();
    thread::Builder::new()
        .name("rshell-backend".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create backend runtime");

            let local = LocalSet::new();
            local.spawn_local(async move {
                // 在后台线程内部创建所有服务
                let keys_dir = data_dir.join("keys");
                let known_hosts_path = data_dir.join("known_hosts");
                let _ = std::fs::create_dir_all(&keys_dir);

                let terminal_service = Arc::new(TerminalService::new(event_bus_for_thread.clone()));
                let session_service = Arc::new(SessionService::new(event_bus_for_thread.clone(), terminal_service.clone()));
                let transfer_service = Arc::new(TransferService::new(event_bus_for_thread.clone()));
                let key_manager = Arc::new(KeyManager::new(keys_dir, event_bus_for_thread.clone()));
                let master_password = Arc::new(MasterPassword::new(event_bus_for_thread.clone()));
                let tunnel_manager = Arc::new(TunnelManager::new(event_bus_for_thread.clone()));
                let host_key_manager = Arc::new(HostKeyManager::new(known_hosts_path, event_bus_for_thread.clone()));
                let theme_manager = Arc::new(ThemeManager::new(event_bus_for_thread.clone()));

                let dispatcher = CommandDispatcher::new(
                    session_service,
                    terminal_service,
                    transfer_service,
                    key_manager,
                    master_password,
                    tunnel_manager,
                    host_key_manager,
                    theme_manager,
                    event_bus_for_thread.clone(),
                );

                // 初始化 dispatcher
                dispatcher.initialize().await;
                info!("Backend dispatcher initialized");

                // 命令处理循环
                let mut command_rx = command_rx;
                info!("Backend command processor started");
                while let Some(command) = command_rx.recv().await {
                    info!("Processing command: {:?}", command);
                    if let Err(e) = dispatcher.dispatch(command).await {
                        error!("Command dispatch error: {}", e);
                    }
                }
                info!("Backend command processor stopped");
            });
            rt.block_on(local);
        })
        .expect("Failed to spawn backend thread");

    info!("Backend services initialized");

    let bridge = AppBridge {
        command_tx,
        event_queue,
        event_bus,
    };

    (bridge, ())
}
