//! 插件加载器
//!
//! 负责发现、验证和加载插件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::api::{PluginManifest, PluginType, PluginState};

/// 插件加载错误
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("Plugin not found: {0}")]
    NotFound(String),
    #[error("Invalid plugin: {0}")]
    InvalidPlugin(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
}

/// 已加载的插件实例
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub path: PathBuf,
}

/// 插件加载器
pub struct PluginLoader {
    /// 插件目录
    plugins_dir: PathBuf,
    /// 已发现的插件清单
    discovered: Arc<RwLock<HashMap<String, PluginManifest>>>,
    /// 已加载的插件
    loaded: Arc<RwLock<HashMap<String, LoadedPlugin>>>,
}

impl PluginLoader {
    /// 创建新的加载器
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            discovered: Arc::new(RwLock::new(HashMap::new())),
            loaded: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 扫描插件目录，发现所有插件
    pub async fn scan_plugins(&self) -> Result<Vec<PluginManifest>, LoadError> {
        let mut manifests = Vec::new();

        if !self.plugins_dir.exists() {
            info!("Plugins directory does not exist: {:?}", self.plugins_dir);
            return Ok(manifests);
        }

        let entries = std::fs::read_dir(&self.plugins_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let manifest_path = path.join("plugin.toml");
                if manifest_path.exists() {
                    match self.parse_manifest(&manifest_path) {
                        Ok(manifest) => {
                            info!("Discovered plugin: {} v{}", manifest.name, manifest.version);
                            manifests.push(manifest.clone());
                            let mut discovered = self.discovered.write().await;
                            discovered.insert(manifest.name.clone(), manifest);
                        }
                        Err(e) => {
                            warn!("Failed to parse manifest at {:?}: {}", manifest_path, e);
                        }
                    }
                }
            }
        }

        Ok(manifests)
    }

    /// 解析插件清单文件
    fn parse_manifest(&self, path: &Path) -> Result<PluginManifest, LoadError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| LoadError::Parse(format!("Failed to read manifest: {}", e)))?;

        // 简化实现：使用 TOML 格式解析
        // 实际需要 toml crate
        // 这里用简单的键值对解析
        let mut name = String::new();
        let mut version = String::from("0.1.0");
        let mut author = String::new();
        let mut description = String::new();

        for line in content.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"');
                match key {
                    "name" => name = value.to_string(),
                    "version" => version = value.to_string(),
                    "author" => author = value.to_string(),
                    "description" => description = value.to_string(),
                    _ => {}
                }
            }
        }

        if name.is_empty() {
            return Err(LoadError::Parse("Plugin name is required".to_string()));
        }

        Ok(PluginManifest {
            name,
            version,
            author,
            description,
            plugin_type: PluginType::Builtin, // 默认
            extensions: Vec::new(),
            permissions: Vec::new(),
            min_rshell_version: "0.1.0".to_string(),
        })
    }

    /// 加载指定插件
    pub async fn load_plugin(&self, plugin_id: &str) -> Result<(), LoadError> {
        let discovered = self.discovered.read().await;
        let manifest = discovered.get(plugin_id)
            .ok_or_else(|| LoadError::NotFound(plugin_id.to_string()))?
            .clone();
        drop(discovered);

        let plugin_path = self.plugins_dir.join(plugin_id);

        let loaded_plugin = LoadedPlugin {
            manifest: manifest.clone(),
            state: PluginState::Loaded,
            path: plugin_path,
        };

        let mut loaded = self.loaded.write().await;
        loaded.insert(plugin_id.to_string(), loaded_plugin);

        info!("Plugin loaded: {}", plugin_id);
        Ok(())
    }

    /// 卸载插件
    pub async fn unload_plugin(&self, plugin_id: &str) -> Result<(), LoadError> {
        let mut loaded = self.loaded.write().await;
        if loaded.remove(plugin_id).is_some() {
            info!("Plugin unloaded: {}", plugin_id);
            Ok(())
        } else {
            Err(LoadError::NotFound(plugin_id.to_string()))
        }
    }

    /// 获取已发现插件列表
    pub async fn discovered_plugins(&self) -> Vec<PluginManifest> {
        self.discovered.read().await.values().cloned().collect()
    }

    /// 获取已加载插件列表
    pub async fn loaded_plugins(&self) -> Vec<LoadedPlugin> {
        self.loaded.read().await.values().cloned().collect()
    }

    /// 获取插件状态
    pub async fn get_plugin_state(&self, plugin_id: &str) -> Option<PluginState> {
        self.loaded.read().await.get(plugin_id).map(|p| p.state.clone())
    }

    /// 列出已加载的插件(转换为 rshell_api::types::PluginInfo 列表)
    pub async fn list_loaded(&self) -> Vec<rshell_api::types::PluginInfo> {
        let guard = self.loaded.read().await;
        let values: Vec<LoadedPlugin> = guard.values().cloned().collect();
        drop(guard);
        values
            .into_iter()
            .map(|p| {
                let permissions: Vec<String> = p
                    .manifest
                    .permissions
                    .iter()
                    .map(|perm| format!("{:?}", perm))
                    .collect();
                let extensions: Vec<String> = p
                    .manifest
                    .extensions
                    .iter()
                    .map(|ext| format!("{:?}", ext))
                    .collect();
                let state = match p.state {
                    PluginState::Loaded => rshell_api::types::PluginState::Loaded,
                    PluginState::Active => rshell_api::types::PluginState::Active,
                    PluginState::Disabled => rshell_api::types::PluginState::Disabled,
                    _ => rshell_api::types::PluginState::Error,
                };
                rshell_api::types::PluginInfo {
                    id: p.manifest.name.clone(),
                    name: p.manifest.name.clone(),
                    version: p.manifest.version.clone(),
                    author: p.manifest.author.clone(),
                    description: p.manifest.description.clone(),
                    plugin_type: rshell_api::types::PluginType::Builtin,
                    state,
                    extensions,
                    permissions,
                }
            })
            .collect()
    }
}

impl Clone for LoadedPlugin {
    fn clone(&self) -> Self {
        Self {
            manifest: self.manifest.clone(),
            state: self.state.clone(),
            path: self.path.clone(),
        }
    }
}
