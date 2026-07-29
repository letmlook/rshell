//! 应用命令定义（前端 → 后端）
//!
//! Command 是不可变意图，表示用户希望执行的操作。
//! 所有字段必须具名，类型必须实现 Serialize + Deserialize + Clone。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{ComposeTarget, PortForwardRule, QuickCommand, SerialConfig, SessionConfig, SshKeyType, TelnetConfig, TerminalColorScheme, Trigger};

/// 前端发送的所有命令
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
    /// 复制终端选中内容
    CopySelection { session_id: Uuid },

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
    /// 信任主机密钥
    TrustHostKey { host: String, port: u16 },
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
}
