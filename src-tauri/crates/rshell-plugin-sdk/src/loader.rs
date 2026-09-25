//! 插件加载器
//!
//! 负责发现、验证和加载插件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::api::{PluginManifest, PluginType, PluginState};
use crate::sandbox::{SandboxConfig, WasmModule, WasmSandbox};

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
    #[error("WASM error: {0}")]
    Wasm(String),
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
    sandbox: Arc<WasmSandbox>,
}

impl PluginLoader {
    /// 创建新的加载器
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            discovered: Arc::new(RwLock::new(HashMap::new())),
            loaded: Arc::new(RwLock::new(HashMap::new())),
            sandbox: Arc::new(WasmSandbox::new(SandboxConfig::default())
                .expect("default WASM sandbox configuration must be valid")),
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
        let mut discovered_now = HashMap::new();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let manifest_path = path.join("plugin.toml");
                if manifest_path.exists() {
                    match self.parse_manifest(&manifest_path) {
                        Ok(manifest) => {
                            if path.file_name().and_then(|name| name.to_str()) != Some(manifest.name.as_str()) {
                                warn!("Plugin directory and manifest name differ: {:?}", path);
                                continue;
                            }
                            info!("Discovered plugin: {} v{}", manifest.name, manifest.version);
                            manifests.push(manifest.clone());
                            discovered_now.insert(manifest.name.clone(), manifest);
                        }
                        Err(e) => {
                            warn!("Failed to parse manifest at {:?}: {}", manifest_path, e);
                        }
                    }
                }
            }
        }

        *self.discovered.write().await = discovered_now;

        Ok(manifests)
    }

    /// 解析插件清单文件
    fn parse_manifest(&self, path: &Path) -> Result<PluginManifest, LoadError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| LoadError::Parse(format!("Failed to read manifest: {}", e)))?;

        let manifest: PluginManifest = toml::from_str(&content)
            .map_err(|e| LoadError::Parse(e.to_string()))?;
        if manifest.name.is_empty() {
            return Err(LoadError::Parse("Plugin name is required".to_string()));
        }
        Ok(manifest)
    }

    /// 加载指定插件
    pub async fn load_plugin(&self, plugin_id: &str) -> Result<(), LoadError> {
        let discovered = self.discovered.read().await;
        let manifest = discovered.get(plugin_id)
            .ok_or_else(|| LoadError::NotFound(plugin_id.to_string()))?
            .clone();
        drop(discovered);

        let plugin_path = self.plugins_dir.join(plugin_id);
        if manifest.plugin_type != PluginType::Wasm {
            return Err(LoadError::InvalidPlugin("Only WASM plugins are supported".into()));
        }
        let mut module = WasmModule::from_file(plugin_path.join("plugin.wasm"))
            .map_err(|e| LoadError::Wasm(e.to_string()))?;
        module.name = plugin_id.to_string();
        let sandbox = Arc::clone(&self.sandbox);
        tokio::task::spawn_blocking(move || sandbox.load(&module))
            .await.map_err(|e| LoadError::Wasm(e.to_string()))?
            .map_err(|e| LoadError::Wasm(e.to_string()))?;

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
            self.sandbox.unload(plugin_id);
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

    /// List both discovered and loaded plugins for the UI.
    pub async fn list_plugins(&self) -> Vec<rshell_api::types::PluginInfo> {
        let guard = self.loaded.read().await;
        let values: HashMap<String, LoadedPlugin> = guard.iter().map(|(id, plugin)| (id.clone(), plugin.clone())).collect();
        drop(guard);
        let discovered = self.discovered.read().await;
        let mut plugins: Vec<_> = discovered.values()
            .map(|manifest| {
                let permissions: Vec<String> = manifest
                    .permissions
                    .iter()
                    .map(|perm| format!("{:?}", perm))
                    .collect();
                let extensions: Vec<String> = manifest
                    .extensions
                    .iter()
                    .map(|ext| format!("{:?}", ext))
                    .collect();
                let state = match values.get(&manifest.name).map(|p| &p.state).unwrap_or(&PluginState::Discovered) {
                    PluginState::Discovered => rshell_api::types::PluginState::Discovered,
                    PluginState::Loaded => rshell_api::types::PluginState::Loaded,
                    PluginState::Active => rshell_api::types::PluginState::Active,
                    PluginState::Disabled => rshell_api::types::PluginState::Disabled,
                    _ => rshell_api::types::PluginState::Error,
                };
                rshell_api::types::PluginInfo {
                    id: manifest.name.clone(),
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    author: manifest.author.clone(),
                    description: manifest.description.clone(),
                    plugin_type: match manifest.plugin_type {
                        PluginType::Builtin => rshell_api::types::PluginType::Builtin,
                        PluginType::Wasm => rshell_api::types::PluginType::Wasm,
                        PluginType::DynamicLib => rshell_api::types::PluginType::DynamicLib,
                    },
                    state,
                    extensions,
                    permissions,
                }
            })
            .collect();
        plugins.sort_by(|a, b| a.id.cmp(&b.id));
        plugins
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wasm_plugin_is_discovered_compiled_and_unloaded() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("example");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), r#"
name = "example"
version = "1.0.0"
author = "test"
description = "test plugin"
plugin_type = "Wasm"
extensions = []
permissions = []
min_rshell_version = "0.1.0"
"#).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), wat::parse_str("(module (func (export \"run\")))").unwrap()).unwrap();

        let loader = PluginLoader::new(dir.path().to_path_buf());
        assert_eq!(loader.scan_plugins().await.unwrap().len(), 1);
        let plugins = loader.list_plugins().await;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].state, rshell_api::types::PluginState::Discovered);
        assert_eq!(plugins[0].plugin_type, rshell_api::types::PluginType::Wasm);
        loader.load_plugin("example").await.unwrap();
        assert_eq!(loader.list_plugins().await[0].state, rshell_api::types::PluginState::Loaded);
        loader.unload_plugin("example").await.unwrap();
        assert_eq!(loader.list_plugins().await[0].state, rshell_api::types::PluginState::Discovered);
    }

    #[tokio::test]
    async fn invalid_wasm_cannot_report_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("bad");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), r#"
name = "bad"
version = "1.0.0"
author = "test"
description = "bad plugin"
plugin_type = "Wasm"
extensions = []
permissions = []
min_rshell_version = "0.1.0"
"#).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), b"not wasm").unwrap();
        let loader = PluginLoader::new(dir.path().to_path_buf());
        loader.scan_plugins().await.unwrap();
        assert!(loader.load_plugin("bad").await.is_err());
        assert_eq!(loader.list_plugins().await[0].state, rshell_api::types::PluginState::Discovered);
    }
}
