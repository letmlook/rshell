//! 应用事件定义（后端 → 前端）
//!
//! Event 是只读快照，表示已发生的事实。
//! 携带完整数据，前端无需再查询。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{ActiveTunnelInfo, AppTheme, ConnectionInfo, ConnectionState, RemoteFileEntry, ScriptResult, SshKeyInfo, TerminalColorScheme, TunnelState};

/// 后端发布的所有事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppEvent {
    // ===== 连接状态变化 =====
    /// 连接状态改变
    ConnectionStateChanged {
        session_id: Uuid,
        state: ConnectionState,
        info: Option<ConnectionInfo>,
    },

    // ===== 终端输出 =====
    /// 终端输出数据（高频事件）
    TerminalOutput { session_id: Uuid, data: Vec<u8> },
    /// 终端标题改变
    TerminalTitleChanged { session_id: Uuid, title: String },

    // ===== 会话数据变化 =====
    /// 会话列表改变（前端需重新拉取）
    SessionListChanged,
    /// 指定会话更新
    SessionUpdated { session_id: Uuid },

    // ===== 传输进度 =====
    /// 传输进度更新
    TransferProgress {
        task_id: Uuid,
        bytes: u64,
        total: u64,
        speed_bps: f64,
    },
    /// 传输完成
    TransferCompleted { task_id: Uuid },
    /// 传输失败
    TransferFailed { task_id: Uuid, error: String },
    /// 传输队列改变
    TransferQueueChanged,
    /// 传输任务已添加
    TransferTaskAdded { task_id: Uuid, filename: String, direction: crate::types::TransferDirection },
    /// 传输任务已完成
    TransferTaskCompleted { task_id: Uuid },
    /// 传输任务已失败
    TransferTaskFailed { task_id: Uuid, error: String },

    // ===== 远程目录浏览 =====
    /// 远程目录列表返回
    RemoteDirListed {
        session_id: Uuid,
        path: String,
        entries: Vec<RemoteFileEntry>,
    },

    // ===== 隧道状态 =====
    /// 隧道状态改变
    TunnelStateChanged { tunnel_id: Uuid, state: TunnelState },

    // ===== 安全事件 =====
    /// 主机密钥不匹配
    HostKeyMismatch {
        host: String,
        expected: String,
        received: String,
    },
    /// 需要主密码
    MasterPasswordRequired,

    // ===== 效率工具 =====
    /// 快速命令列表变化
    QuickCommandListChanged,
    /// 触发器列表变化
    TriggerListChanged,
    /// 触发器触发
    TriggerFired {
        trigger_id: Uuid,
        session_id: Uuid,
        action_summary: String,
    },
    /// 脚本执行结果
    ScriptFinished {
        session_id: Uuid,
        result: ScriptResult,
    },
    /// 同步输入会话列表变化
    SyncInputSessionsChanged { session_ids: Vec<Uuid> },

    // ===== 安全事件 =====
    /// 密钥列表变化
    SshKeyListChanged,
    /// 密钥生成完成
    SshKeyGenerated { key: SshKeyInfo },
    /// 公钥导出结果
    PublicKeyExported { key_id: Uuid, public_key: String },
    /// 主密码状态变化
    MasterPasswordChanged { is_set: bool },
    /// 主密码验证结果
    MasterPasswordVerified { success: bool },
    /// 活动隧道列表变化
    ActiveTunnelsChanged,
    /// 隧道信息更新
    TunnelUpdated { tunnel: ActiveTunnelInfo },

    // ===== 主题事件 =====
    /// 主题已切换
    ThemeChanged { theme: AppTheme },
    /// 配色方案已切换
    ColorSchemeChanged { scheme: TerminalColorScheme },
    /// 配色方案列表更新
    ColorSchemeListChanged,

    // ===== 剪贴板事件 =====
    /// 用户请求拷贝文本到系统剪贴板（前端应调 arboard / nopclipboard 写入）
    ClipboardCopy { text: String },

    // ===== 插件事件 =====
    /// 插件列表已更新
    PluginListUpdated,
    /// 插件状态已变化
    PluginStateChanged { plugin_id: String, state: crate::types::PluginState },
    /// 插件加载失败
    PluginLoadFailed { plugin_id: String, error: String },
}
