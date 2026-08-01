//! 插件 API
//!
//! 提供给插件的接口定义，包括插件 trait、清单、扩展点等。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 插件清单（plugin.toml）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub plugin_type: PluginType,
    pub extensions: Vec<ExtensionPoint>,
    pub permissions: Vec<PluginPermission>,
    pub min_rshell_version: String,
}

/// 插件类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginType {
    Builtin,
    Wasm,
    DynamicLib,
}

/// 扩展点声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtensionPoint {
    Protocol { scheme: String },
    Theme { name: String },
    ColorScheme { name: String },
    ToolPanel { id: String, title: String },
    QuickCommand { commands: Vec<String> },
    FileAction { name: String },
    TriggerAction { name: String },
    StatusBar { position: String },
}

/// 插件权限
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginPermission {
    NetworkAccess,
    FileSystemAccess,
    SessionAccess,
    TerminalAccess,
    ClipboardAccess,
    ProcessExecution,
}

/// 插件状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Loaded,
    Active,
    Error(String),
    Disabled,
}

/// 插件 trait（所有插件实现此接口）
pub trait RShellPlugin: Send + Sync {
    /// 插件初始化
    fn init(&mut self, ctx: PluginContext) -> Result<(), PluginError>;
    /// 插件卸载
    fn shutdown(&mut self) -> Result<(), PluginError>;
    /// 获取插件清单
    fn manifest(&self) -> &PluginManifest;
    /// 获取插件状态
    fn state(&self) -> PluginState;
}

/// 插件上下文（插件通过此与宿主交互）
pub struct PluginContext {
    /// 插件 ID
    pub plugin_id: String,
    /// 插件数据目录
    pub data_dir: PathBuf,
    /// 配置存储
    pub config: PluginConfigStore,
    /// 日志接口
    pub logger: PluginLogger,
}

/// 插件配置存储
pub struct PluginConfigStore {
    plugin_id: String,
    config_dir: PathBuf,
    cache: HashMap<String, String>,
}

impl PluginConfigStore {
    /// 创建新的配置存储
    pub fn new(plugin_id: &str, config_dir: PathBuf) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
            config_dir,
            cache: HashMap::new(),
        }
    }

    /// 读取配置
    pub fn get(&self, key: &str) -> Option<&str> {
        self.cache.get(key).map(|s| s.as_str())
    }

    /// 写入配置
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), PluginError> {
        self.cache.insert(key.to_string(), value.to_string());
        // 持久化到文件（简化实现）
        let path = self.config_dir.join(format!("{}.json", self.plugin_id));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&self.cache)
            .map_err(|e| PluginError::ConfigError(format!("Serialize failed: {}", e)))?;
        std::fs::write(&path, json)
            .map_err(|e| PluginError::ConfigError(format!("Write failed: {}", e)))?;
        Ok(())
    }

    /// 加载已保存的配置
    pub fn load(&mut self) -> Result<(), PluginError> {
        let path = self.config_dir.join(format!("{}.json", self.plugin_id));
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| PluginError::ConfigError(format!("Read failed: {}", e)))?;
            self.cache = serde_json::from_str(&content)
                .map_err(|e| PluginError::ConfigError(format!("Deserialize failed: {}", e)))?;
        }
        Ok(())
    }
}

/// 插件日志接口
pub struct PluginLogger {
    plugin_id: String,
}

impl PluginLogger {
    pub fn new(plugin_id: &str) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
        }
    }

    pub fn info(&self, message: &str) {
        tracing::info!("[plugin:{}] {}", self.plugin_id, message);
    }

    pub fn warn(&self, message: &str) {
        tracing::warn!("[plugin:{}] {}", self.plugin_id, message);
    }

    pub fn error(&self, message: &str) {
        tracing::error!("[plugin:{}] {}", self.plugin_id, message);
    }

    pub fn debug(&self, message: &str) {
        tracing::debug!("[plugin:{}] {}", self.plugin_id, message);
    }
}

/// 插件错误
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Plugin initialization failed: {0}")]
    InitFailed(String),
    #[error("Plugin not found: {0}")]
    NotFound(String),
    #[error("Invalid plugin manifest: {0}")]
    InvalidManifest(String),
    #[error("Plugin execution error: {0}")]
    ExecutionError(String),
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Incompatible version: {0}")]
    IncompatibleVersion(String),
}
