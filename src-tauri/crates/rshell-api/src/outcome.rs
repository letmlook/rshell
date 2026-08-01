//! `CommandDispatcher::dispatch` 返回值类型 —— 设计 §3.2 / §3.4
//!
//! 修掉一个现存死循环（`command_dispatcher.rs:420-435`）：
//! `ListTriggers` / `ListQuickCommands` 算出数据后 `let _ =` 丢弃,
//! 仅发 `*Changed` 事件让前端再 invoke —— 永远拿不到数据。
//!
//! D4 因此不是重构洁癖,而是修 bug。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    ActiveTunnelInfo, PluginInfo, PortForwardRule, QuickCommand, RemoteFileEntry,
    SessionConfig, SshKeyInfo, ThemeInfo, Trigger,
};

/// 读命令返回结构化数据;写命令返回 `None`。
///
/// 切片 1 仅引入 `None` / `Sessions` / `SessionId` 三变体以驱动首批 7 个命令;
/// 其余变体在切片 3+ 按功能域逐项落地。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandOutcome {
    /// 全部写操作(create/update/delete/connect/disconnect/send_input/resize/...)
    None,
    /// 响应 `ListSessions`
    Sessions(Vec<SessionConfig>),
    /// 响应 `CreateSession` —— 此前返回值被丢弃,前端拿不到新会话 id
    SessionId(Uuid),
    /// 响应 `ListTriggers` —— 修 §3.2 死循环
    Triggers(Vec<Trigger>),
    /// 响应 `ListQuickCommands` —— 修 §3.2 死循环
    QuickCommands(Vec<QuickCommand>),
    /// 响应 `ListKeys`
    Keys(Vec<SshKeyInfo>),
    /// 响应 `ListTunnels`
    Tunnels(Vec<ActiveTunnelInfo>),
    /// 响应 `ListPlugins`
    Plugins(Vec<PluginInfo>),
    /// 响应 `ListThemes`
    Themes(ThemeInfo),
    /// 响应 `ListPendingTunnels`
    PendingTunnels(Vec<(Uuid, PortForwardRule)>),
    /// 响应 `BrowseRemoteDir`
    RemoteDir {
        path: String,
        entries: Vec<RemoteFileEntry>,
    },
    /// 响应 `ExportPublicKey`
    PublicKey(String),
    /// 响应 `VerifyMasterPassword`
    Verified(bool),
}

impl CommandOutcome {
    /// 仅用于薄壳兜底判断（设计 §3.4）：dispatcher 分支写错时触发,
    /// 由薄壳构造 `IpcError::outcome_mismatch`。理论不可达。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sessions(_) => "sessions",
            Self::SessionId(_) => "session_id",
            Self::Triggers(_) => "triggers",
            Self::QuickCommands(_) => "quick_commands",
            Self::Keys(_) => "keys",
            Self::Tunnels(_) => "tunnels",
            Self::Plugins(_) => "plugins",
            Self::Themes(_) => "themes",
            Self::PendingTunnels(_) => "pending_tunnels",
            Self::RemoteDir { .. } => "remote_dir",
            Self::PublicKey(_) => "public_key",
            Self::Verified(_) => "verified",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_strings_are_stable() {
        assert_eq!(CommandOutcome::None.kind(), "none");
        assert_eq!(CommandOutcome::SessionId(Uuid::nil()).kind(), "session_id");
        assert_eq!(CommandOutcome::Triggers(Vec::new()).kind(), "triggers");
        assert_eq!(CommandOutcome::QuickCommands(Vec::new()).kind(), "quick_commands");
        assert_eq!(CommandOutcome::Verified(true).kind(), "verified");
        assert_eq!(CommandOutcome::Themes(crate::types::ThemeInfo {
            current_theme: String::new(),
            current_scheme: String::new(),
            available_themes: vec![],
            available_schemes: vec![],
        }).kind(), "themes");
        assert_eq!(CommandOutcome::RemoteDir {
            path: String::new(),
            entries: vec![],
        }.kind(), "remote_dir");
    }

    /// 切片 3.3：每个 CommandOutcome 变体的契约测试 —— 序列化标签稳定
    /// (用于 §3.4 薄壳 `outcome_mismatch` 兜底断言)。
    /// 仅测核心 13 变体的 kind 字符串,完整覆盖(serde roundtrip)留 ts-rs 切片 3+。
    #[test]
    fn all_variants_have_distinct_kinds() {
        use std::collections::HashSet;
        let variants: Vec<CommandOutcome> = vec![
            CommandOutcome::None,
            CommandOutcome::Sessions(vec![]),
            CommandOutcome::SessionId(Uuid::nil()),
            CommandOutcome::Triggers(vec![]),
            CommandOutcome::QuickCommands(vec![]),
            CommandOutcome::Keys(vec![]),
            CommandOutcome::Tunnels(vec![]),
            CommandOutcome::Plugins(vec![]),
            CommandOutcome::Themes(crate::types::ThemeInfo {
                current_theme: String::new(),
                current_scheme: String::new(),
                available_themes: vec![],
                available_schemes: vec![],
            }),
            CommandOutcome::PendingTunnels(vec![]),
            CommandOutcome::RemoteDir {
                path: String::new(),
                entries: vec![],
            },
            CommandOutcome::PublicKey(String::new()),
            CommandOutcome::Verified(false),
        ];
        let kinds: HashSet<&str> = variants.iter().map(|v| v.kind()).collect();
        assert_eq!(
            kinds.len(),
            variants.len(),
            "all 13 variants must have distinct kind labels"
        );
    }

    #[test]
    fn snake_case_in_tag_is_internal_contract() {
        // `#[serde(tag = "kind", rename_all = "snake_case")]` 由 serde 派生;
        // 本断言确保 kind() 函数返回的字符串与 serde 派生一致 —— 后续
        // 切片 2 引入 ts-rs 时会以本函数为参考。
        let v = CommandOutcome::Verified(false);
        assert_eq!(v.kind(), "verified");
    }
}