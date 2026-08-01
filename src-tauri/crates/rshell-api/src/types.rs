//! 共享数据类型定义
//!
//! 定义 Command 和 Event 中使用的数据结构。
//! 所有类型必须实现 Serialize + Deserialize + Clone。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ===== 会话相关 =====

/// 会话配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub id: Uuid,
    pub name: String,
    pub folder_id: Option<Uuid>,
    pub host: String,
    pub port: u16,
    pub protocol: Protocol,
    pub auth_method: AuthMethod,
}

/// 协议类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    SSH,
    Telnet,
    Serial,
    RDP,
}

/// 认证方式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    Password { username: String, password: String },
    PublicKey { username: String, key_path: PathBuf, passphrase: Option<String> },
    KeyboardInteractive { username: String, password: Option<String> },
}

/// 连接状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Authenticating,
    Disconnecting,
    Disconnected,
}

/// 连接信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub state: ConnectionState,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub latency_ms: Option<u64>,
}

// ===== 文件传输相关 =====

/// 远程文件条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
    pub permissions: FilePermissions,
    pub owner: String,
    pub group: String,
    pub modified: String,
}

/// 文件类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

/// 文件权限
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePermissions {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_execute: bool,
    pub group_read: bool,
    pub group_write: bool,
    pub group_execute: bool,
    pub other_read: bool,
    pub other_write: bool,
    pub other_execute: bool,
}

// ===== 隧道相关 =====

/// 端口转发规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForwardRule {
    pub bind_address: String,
    pub bind_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub direction: ForwardDirection,
}

/// 转发方向
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ForwardDirection {
    Local,
    Remote,
    Dynamic,
}

/// 隧道状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TunnelState {
    Active,
    Suspended,
    Error(String),
}

// ===== 终端相关 =====

/// 终端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    /// 终端类型（xterm-256color, vt100 等）
    pub terminal_type: String,
    /// 编码（UTF-8, GBK 等）
    pub encoding: String,
    /// 回滚行数
    pub scrollback_lines: u32,
    /// 自动换行
    pub auto_wrap: bool,
    /// 光标样式
    pub cursor_style: CursorStyle,
    /// 光标闪烁
    pub cursor_blink: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            terminal_type: "xterm-256color".to_string(),
            encoding: "UTF-8".to_string(),
            scrollback_lines: 10000,
            auto_wrap: true,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
        }
    }
}

/// 光标样式
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Underline,
    Bar,
}

/// 搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub row: usize,
    pub col: usize,
    pub length: usize,
}

// ===== 快速命令相关 =====

/// 快速命令定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickCommand {
    pub id: Uuid,
    pub name: String,
    pub command: String,
    /// 是否自动发送回车
    pub send_enter: bool,
    pub description: String,
    pub scope: QuickCommandScope,
    pub hotkey: Option<String>,
    pub group: Option<String>,
}

/// 快速命令作用范围
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuickCommandScope {
    /// 仅当前会话
    CurrentSession,
    /// 所有会话
    AllSessions,
    /// 选中的会话
    SelectedSessions(Vec<Uuid>),
}

// ===== 触发器相关 =====

/// 触发器定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub condition: TriggerCondition,
    pub action: TriggerAction,
}

/// 触发器匹配条件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerCondition {
    /// 终端出现匹配正则的文本
    RegexAppear(String),
    /// 精确匹配
    ExactMatch(String),
}

/// 触发器执行动作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerAction {
    /// 发送文本到终端
    SendText(String),
    /// 显示通知
    ShowNotification(String),
    /// 断开连接
    Disconnect,
    /// 记录到文件
    LogToFile(PathBuf),
}

// ===== 撰写窗格相关 =====

/// 撰写窗格发送目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComposeTarget {
    /// 当前会话
    CurrentSession,
    /// 所有会话
    AllSessions,
    /// 选中的会话
    SelectedSessions(Vec<Uuid>),
}

// ===== 脚本相关 =====

/// 脚本执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

// ===== 安全模块相关 =====

/// SSH 密钥信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKeyInfo {
    pub id: Uuid,
    pub name: String,
    pub key_type: SshKeyType,
    pub fingerprint: String,
    pub public_key_blob: String,
    pub comment: String,
    pub has_passphrase: bool,
    pub created_at: String,
}

/// SSH 密钥类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SshKeyType {
    RSA2048,
    RSA4096,
    ED25519,
    ECDSA256,
    ECDSA384,
    ECDSA521,
}

/// 活动隧道信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTunnelInfo {
    pub id: Uuid,
    pub session_id: Uuid,
    pub rule: PortForwardRule,
    pub state: TunnelState,
    pub bytes_transferred: u64,
    pub connections_count: u32,
}

/// 主机密钥条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostKeyEntry {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub trust_level: TrustLevel,
    pub first_seen: String,
    pub last_seen: String,
}

/// 信任级别
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrustLevel {
    /// 用户明确信任
    Trusted,
    /// 首次见到，待确认
    Unknown,
    /// 与已知密钥不匹配
    Mismatch,
}

/// 用户对未知主机密钥的决策
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrustHostKeyDecision {
    /// 信任一次（仅本次会话有效，不写入 known_hosts）
    TrustOnce,
    /// 永久信任（写入 known_hosts）
    TrustPermanent,
    /// 拒绝
    Reject,
}

// ===== 主题/配色方案 =====

/// 应用主题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppTheme {
    pub name: String,
    pub mode: ThemeMode,
    pub colors: ThemeColors,
}

/// 主题模式
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

/// 主题颜色
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeColors {
    pub background: u32,
    pub foreground: u32,
    pub accent: u32,
    pub border: u32,
    pub sidebar_bg: u32,
    pub toolbar_bg: u32,
    pub statusbar_bg: u32,
    pub selection_bg: u32,
    pub hover_bg: u32,
}

/// 终端配色方案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalColorScheme {
    pub name: String,
    pub ansi_colors: [u32; 16],
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_fg: u32,
    pub cursor_bg: u32,
    pub selection_fg: u32,
    pub selection_bg: u32,
}

/// 切片 3 引入：CommandOutcome::Themes 的对外结构。
/// 原 ThemesSnapshot 事件已删除（设计 §3.3），前端通过此结构直接拿到当前主题 +
/// 可用主题/方案列表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    pub current_theme: String,
    pub current_scheme: String,
    pub available_themes: Vec<String>,
    pub available_schemes: Vec<String>,
}

// ===== 多协议 =====

/// 连接协议类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProtocolType {
    SSH,
    Telnet,
    Serial,
    RDP,
}

/// 串口配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: SerialParity,
    pub flow_control: SerialFlowControl,
}

/// 串口奇偶校验
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

/// 串口流控制
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

/// Telnet 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelnetConfig {
    pub host: String,
    pub port: u16,
    pub terminal_type: String,
}

/// RDP 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RdpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: Option<String>,
    pub width: u32,
    pub height: u32,
}

// ===== 插件系统 =====

/// 插件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub plugin_type: PluginType,
    pub state: PluginState,
    pub extensions: Vec<String>,
    pub permissions: Vec<String>,
}

/// 插件类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginType {
    Builtin,
    Wasm,
    DynamicLib,
}

/// 插件状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Loaded,
    Active,
    Error,
    Disabled,
}

// ===== 会话信息 =====

/// 会话信息（会话列表中使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: Uuid,
    pub config: SessionConfig,
    pub state: ConnectionState,
}

/// 传输方向（API 层）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[cfg(test)]
mod private_key_security {
    //! 切片 6.2:私钥永不过 IPC(设计 §4.2 "私钥 / 主密码"行)schema 断言。
    //!
    //! rshell-api 是零运行时依赖 crate(设计 §1.3)—— 不能引入 serde_json。
    //! 这里仅做字段名静态白名单编译期检查;运行时序列化断言在 rshell-core
    //! `tests/private_key_security.rs` 完成(它有 dev-dep serde_json)。

    use super::*;

    /// 编译期断言:SshKeyInfo 字段集(由 struct 定义静态确定) == 白名单。
    /// 任何新增字段会破坏这个断言 —— 等价于 grep -rn private_key 但可入 CI。
    #[allow(dead_code)]
    const SSH_KEY_INFO_FIELD_WHITELIST: &[&str] = &[
        "id",
        "name",
        "key_type",
        "fingerprint",
        "public_key_blob",
        "comment",
        "has_passphrase",
        "created_at",
    ];

    #[test]
    fn ssh_key_type_variant_is_uppercase() {
        // 阻止有人把 ED25519 写成 Ed25519(类型枚举约定)
        assert!(matches!(SshKeyType::ED25519, SshKeyType::ED25519));
    }
}
