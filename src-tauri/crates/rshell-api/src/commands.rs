//! 应用命令定义（前端 → 后端）
//!
//! Command 是不可变意图，表示用户希望执行的操作。
//! 所有字段必须具名，类型必须实现 Serialize + Deserialize + Clone。
//!
//! 切片 2.3 状态：ts-rs 12 已加入 workspace 依赖（`Cargo.toml` `workspace.dependencies.ts-rs`）。
//! 全量 `#[derive(TS)]` 需要 types.rs 中所有公共结构体同时 derive ——
//! 那是切片 3 域内的工作量（按功能域逐项 derive + 导出）。
//! 本切片仅记录:derive 模式已验证可用，导出路径以 `export_to = "../../../src/ipc/generated.ts"`
//! 形式约定，CI 加 `git diff --exit-code src/ipc/generated.ts` 拦截漂移。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{ComposeTarget, PortForwardRule, QuickCommand, SerialConfig, SessionConfig, SshKeyType, TelnetConfig, TerminalColorScheme, TrustHostKeyDecision, Trigger};

/// 前端发送的所有命令
// 切片 2.3 注释：ts-rs 全量 derive 等 types.rs 同步 derive 后再开。
// 当前依赖已就位 (`Cargo.toml` ts-rs workspace dep)，但 TS impl 链是
// 整树深度依赖 —— 全量激活是切片 3 域内的工作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppCommand {
    // ===== 会话命令 =====
    /// 连接到指定会话
    ConnectSession { session_id: Uuid },
    /// 断开指定会话
    DisconnectSession { session_id: Uuid },
    /// 创建新会话
    CreateSession { config: SessionConfig },
    /// 更新会话配置
    UpdateSession { id: Uuid, config: SessionConfig },
    /// 删除会话
    DeleteSession { id: Uuid },

    // ===== 终端命令 =====
    /// 发送输入到终端
    SendInput { session_id: Uuid, data: Vec<u8> },
    /// 调整终端大小
    ResizeTerminal { session_id: Uuid, cols: u16, rows: u16 },

    // ===== 文件传输命令 =====
    /// 添加上传任务
    EnqueueUpload {
        local: std::path::PathBuf,
        remote: String,
        session_id: Uuid,
    },
    /// 添加下载任务
    EnqueueDownload {
        remote: String,
        local: std::path::PathBuf,
        session_id: Uuid,
    },
    /// 暂停传输
    PauseTransfer { task_id: Uuid },
    /// 恢复传输
    ResumeTransfer { task_id: Uuid },
    /// 取消传输
    CancelTransfer { task_id: Uuid },
    /// 浏览远程目录
    BrowseRemoteDir { session_id: Uuid, path: String },

    // ===== 隧道命令 =====
    /// 创建端口转发隧道
    CreateTunnel { session_id: Uuid, rule: PortForwardRule },
    /// 关闭隧道
    CloseTunnel { tunnel_id: Uuid },

    /// 列出待重建隧道 (从磁盘恢复, 本次进程未启动 listener)
    ListPendingTunnels,
    /// 把一条 pending 规则升级为活动隧道 (UI 端"启动"按钮)
    RestoreTunnel { session_id: Uuid, rule: PortForwardRule },

    // ===== 快速命令 =====
    /// 执行快速命令
    ExecuteQuickCommand { command_id: Uuid, target_sessions: Vec<Uuid> },
    /// 创建快速命令
    CreateQuickCommand { command: QuickCommand },
    /// 删除快速命令
    DeleteQuickCommand { command_id: Uuid },

    // ===== 触发器 =====
    /// 创建触发器
    CreateTrigger { trigger: Trigger },
    /// 删除触发器
    DeleteTrigger { trigger_id: Uuid },
    /// 切换触发器启用/禁用
    ToggleTrigger { trigger_id: Uuid },

    // ===== 撰写窗格 =====
    /// 发送撰写窗格文本
    SendComposeText { content: String, target: ComposeTarget },

    // ===== 脚本 =====
    /// 执行脚本
    ExecuteScript { code: String, session_id: Uuid },

    // ===== 同步输入 =====
    /// 切换同步输入模式
    ToggleSyncInput { session_ids: Vec<Uuid> },

    // ===== 安全：密钥管理 =====
    /// 生成 SSH 密钥对
    GenerateSshKey { name: String, key_type: SshKeyType, passphrase: Option<String> },
    /// 导入私钥
    ImportPrivateKey { path: std::path::PathBuf, passphrase: Option<String> },
    /// 删除密钥
    DeleteSshKey { key_id: Uuid },
    /// 导出公钥
    ExportPublicKey { key_id: Uuid },

    // ===== 安全：主密码 =====
    /// 设置主密码
    SetupMasterPassword { password: String },
    /// 验证主密码
    VerifyMasterPassword { password: String },
    /// 修改主密码
    ChangeMasterPassword { old_password: String, new_password: String },

    // ===== 安全：主机密钥 =====
    /// 信任主机密钥（在 HostKeyMismatch 后用户选择接受）
    TrustHostKey {
        host: String,
        port: u16,
        key_type: String,
        public_key_blob: String,
        decision: TrustHostKeyDecision,
    },

    /// 实时决策某个 host key（握手期间通过 HostKeyMismatch 事件带过来的 decision_id）
    ///
    /// 这是**会话握手阶段**的决策通道；与 `TrustHostKey` 的区别:
    /// - `DecideHostKey` 是**带 decision_id**的实时响应,会唤醒阻塞在 SshHandler 的 oneshot
    /// - `TrustHostKey` 是已连接后对 known_hosts 的离线增删
    DecideHostKey {
        decision_id: Uuid,
        accept: bool,
        permanent: bool,
    },

    /// 删除主机密钥
    DeleteHostKey { host: String, port: u16 },

    // ===== 安全：隧道管理 =====
    /// 暂停隧道
    SuspendTunnel { tunnel_id: Uuid },
    /// 恢复隧道
    ResumeTunnel { tunnel_id: Uuid },

    // ===== 主题/配色方案 =====
    /// 设置应用主题
    SetAppTheme { theme_name: String },
    /// 设置终端配色方案
    SetTerminalColorScheme { scheme_name: String },
    /// 导入自定义配色方案
    ImportColorScheme { scheme: TerminalColorScheme },

    // ===== 多协议 =====
    /// 连接 Telnet 会话
    ConnectTelnet { config: TelnetConfig },
    /// 连接串口会话
    ConnectSerial { config: SerialConfig },

    // ===== 插件管理 =====
    /// 扫描插件目录
    ScanPlugins,
    /// 加载插件
    LoadPlugin { plugin_id: String },
    /// 卸载插件
    UnloadPlugin { plugin_id: String },
    /// 启用插件
    EnablePlugin { plugin_id: String },
    /// 禁用插件
    DisablePlugin { plugin_id: String },

    // ===== List / snapshot 拉取 =====
    // 这些命令让 UI 主动拉数据, 触发后端 publish 对应 XSnapshot 事件。
    // 解决 view.update_*() 从未被调用的问题——UI 在 mount 时 + 每次 refresh button 时调用。
    /// 拉取所有会话列表
    ListSessions,
    /// 拉取所有活动隧道列表
    ListTunnels,
    /// 拉取所有 SSH 密钥列表
    ListKeys,
    /// 拉取已加载插件列表
    ListPlugins,
    /// 拉取所有触发器
    ListTriggers,
    /// 拉取所有快速命令
    ListQuickCommands,
    /// 拉取当前主题和配色方案 (含可用列表)
    ListThemes,
}
