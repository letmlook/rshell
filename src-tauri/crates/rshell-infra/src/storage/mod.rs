//! 持久化存储模块
//!
//! 提供会话元数据等数据的持久化能力。
//!
//! 原本这里还有一份 `AppConfig`（theme / font_size / color_scheme …，落在
//! `config.toml`）。它从未被调用：应用的主题与终端外观改走前端偏好
//! （`utils/terminalPrefs` 的 localStorage），而 docs/08 明确「主题选择不承诺
//! 跨进程恢复」。留着它等于第三套并存的存储方案，故移除。

pub mod session_store;
