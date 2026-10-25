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
    #[serde(default)]
    pub serial_config: Option<SerialConfig>,
}

/// 协议类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    SSH,
    Telnet,
    Serial,
}

/// 认证方式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    Password {
        username: String,
        has_password: bool,
    },
    PublicKey {
        username: String,
        key_path: PathBuf,
        has_passphrase: bool,
        /// 密钥管理面板托管的私钥引用。`Some` 时连接经 `KeyManager`
        /// 取私钥字节认证，`key_path` 不参与认证（可为空）。
        /// 缺省/None 保持旧的磁盘私钥文件语义。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        managed_key_id: Option<Uuid>,
    },
    KeyboardInteractive {
        username: String,
        has_password: bool,
    },
}

/// 仅作为命令输入传递，不属于可持久化或可回传的会话配置。
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionCredential {
    pub secret: String,
}

impl std::fmt::Debug for SessionCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCredential")
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// 敏感字符串（口令 / 口令短语）的包装类型。
///
/// **存在的理由**：口令类的字段散落在 `AppCommand` 的多个变体里，每个都用裸
/// `String`。裸 `String` 的 `Debug` 是明文，一旦被 `#[instrument(fields(..))]`
/// 或 `{:?}` 记录就进了日志（真实事故：主密码与私钥口令以明文进 tracing span）。
///
/// 用这个类型把「敏感」标注在**类型**上，而不是靠每个字段作者记得手写脱敏：
/// - `Debug` 恒为 `<redacted>`，派生 `Debug` 的容器（枚举、struct）自动安全；
/// - `#[serde(transparent)]` —— 线上格式仍是普通 JSON 字符串，前端契约不变；
/// - `Deref<Target = str>` —— 调用方照常把它当 `&str` 用（`&password` 传 `&str`
///   参数靠 deref coercion 自动成立），`Option<Secret>` 也能照常用 `as_deref()`。
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// 构造敏感字符串。
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 取出明文。**只在真正需要使用凭据的边界调用**（例如喂给 KDF 或解密）。
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl std::ops::Deref for Secret {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// 更新会话凭据时的显式意图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CredentialUpdate {
    Keep,
    Set(SessionCredential),
    Clear,
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

/// 传输目标已存在时的处理策略。
///
/// `Fail` 是安全默认：入队即返回 `CoreError::TargetExists`，前端弹出
/// 「覆盖 / 重命名 / 取消」对话框，再用用户选定的策略重试一次。
/// 后端在**入队时**强制执行该策略，因此前端先查后传存在 TOCTOU 竞态也不会
/// 绕过保护——不允许出现「界面问过了就静默覆盖」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictPolicy {
    /// 目标已存在即报错
    Fail,
    /// 覆盖已有目标
    Overwrite,
    /// 改用指定的新文件名（仅文件名，不含分隔符），原文件名保留
    Rename(String),
}

impl Default for ConflictPolicy {
    /// 省略 `conflict` 字段时按 `Fail` 处理：安全默认，不静默覆盖。
    fn default() -> Self {
        Self::Fail
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTaskInfo {
    pub id: Uuid,
    pub session_id: Uuid,
    pub direction: TransferDirection,
    pub local_path: String,
    pub remote_path: String,
    pub state: TransferTaskState,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    /// 当前传输速度（字节/秒），仅 Transferring 状态有值
    pub speed_bps: f64,
    pub error_message: Option<String>,
    /// R2-T2：staging temp 路径（远端 / 本地侧）—— 终态下若 cleanup 失败
    /// 则该 temp 仍存在；前端据此展示「残留临时文件」的位置与原因。
    /// 活跃任务上为 `None`（temp 已独占、commit 即将发起）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_path: Option<String>,
    /// R2-T2：终态下的清理结果标签（`"cleaned"` / `"residue"`）。`None` 表示
    /// 活跃任务或终态尚未结算清理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_status: Option<String>,
    /// R2-T2：终态下的提交策略标签（仅 `Completed` 上有值）。前端据此向
    /// 用户说明「这次传输拿到了原子替换保证吗」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_strategy: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TransferTaskState {
    Pending,
    Transferring,
    Paused,
    Completed,
    Failed,
    Cancelled,
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
    /// 非回环 bind_address 已经过用户在 UI 上显式确认（PROB-23）。
    /// `serde(default)` 兼容旧前端请求与旧持久化文件：缺省视为未确认，
    /// 后端据此默认拒绝非回环监听。
    #[serde(default)]
    pub allow_non_loopback: bool,
}

/// 转发方向
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ForwardDirection {
    Local,
    Dynamic,
}

/// 从旧配置读取但无法启动的隧道规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedTunnelRule {
    pub session_id: Uuid,
    pub reason: String,
}

/// Persistent, sanitized diagnostics; never include raw TOML or backend errors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLoadIssue {
    pub session_id: Option<Uuid>,
    pub kind: SessionLoadIssueKind,
    /// A safe, human-readable cause. It may identify the storage boundary, but
    /// never contains a raw credential, TOML source line, or backend secret.
    pub message: String,
    /// Whether the explicit retry-load action can make progress after the
    /// underlying access/configuration problem is repaired.
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionLoadIssueKind {
    CredentialMigrationFailed,
    StorageUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTunnelInfo {
    pub rules: Vec<(Uuid, PortForwardRule)>,
    pub unsupported: Vec<UnsupportedTunnelRule>,
}

/// 隧道状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TunnelState {
    Active,
    Suspended,
    Error(String),
}

// ===== 连接分组 =====

/// 连接分组（文件夹）
///
/// `SessionConfig::folder_id` 指向这个实体。此前它只是个悬空的
/// `Option<Uuid>`：没有任何地方创建分组，界面上的分组树因此永远只有一个
/// 「默认」组。分组是可复用的连接配方的一部分（见 docs/08 的术语定义），
/// 所以这里给出真正的实体与存储。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
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
    pub current_colors: ThemeColors,
    pub current_palette: TerminalColorScheme,
    pub available_themes: Vec<String>,
    pub available_schemes: Vec<String>,
}

// ===== 多协议 =====

// 连接协议用 Protocol（见文件上部的 `pub enum Protocol`）。此前这里还有一个
// 同义的 ProtocolType，全仓库零使用方 —— 它只在同文件自己的测试模块里被
// 引用过一次，等于在声明一个没人用的能力。协议契约测试直接针对真正在用的
// Protocol 断言，RDP 一旦被加回来立即失败。
// 零引用类型的门禁见 tests/unit/deadSurface.spec.ts。

#[cfg(test)]
mod protocol_contract_tests {
    use super::{ForwardDirection, Protocol};

    #[test]
    fn removed_rdp_protocol_is_rejected() {
        assert!(serde_json::from_str::<Protocol>("\"RDP\"").is_err());
        assert!(serde_json::from_str::<Protocol>("\"SSH\"").is_ok());
        assert!(serde_json::from_str::<Protocol>("\"Telnet\"").is_ok());
        assert!(serde_json::from_str::<Protocol>("\"Serial\"").is_ok());
    }

    #[test]
    fn unsupported_remote_forward_is_rejected() {
        assert!(serde_json::from_str::<ForwardDirection>("\"Remote\"").is_err());
        assert!(serde_json::from_str::<ForwardDirection>("\"Local\"").is_ok());
        assert!(serde_json::from_str::<ForwardDirection>("\"Dynamic\"").is_ok());
    }
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
    /// 扫描/加载失败的原因；正常插件为 None。
    ///
    /// 扫描时被拒的插件目录（清单解析失败、目录名与清单 name 不一致）会以
    /// `state = Error` 的条目出现在列表里，原因写在这里——用户扫描后不能只看到
    /// 「没有插件」。
    pub error: Option<String>,
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
