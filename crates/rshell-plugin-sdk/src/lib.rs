//! RShell 插件 SDK
//!
//! 提供插件系统的核心能力：
//! - 插件 API（api）：插件 trait、清单、扩展点
//! - 插件加载（loader）：发现、验证、加载插件
//! - WASM 沙箱（sandbox）：安全执行 WASM 插件

pub mod api;
pub mod loader;
pub mod sandbox;

// Re-export 主要类型
pub use api::{
    ExtensionPoint, PluginContext, PluginError, PluginLogger,
    PluginManifest, PluginPermission, PluginState, PluginType,
    RShellPlugin, PluginConfigStore,
};
pub use loader::{LoadError, LoadedPlugin, PluginLoader};
pub use sandbox::{SandboxConfig, SandboxError, WasmModule, WasmSandbox, WasmValue};
