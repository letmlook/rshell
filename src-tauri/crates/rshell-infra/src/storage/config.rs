//! 配置管理

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, debug};

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub font_size: u16,
    pub font_family: String,
    pub color_scheme: String,
    pub scrollback_lines: u32,
    pub encoding: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "Dark".to_string(),
            font_size: 14,
            font_family: "Consolas".to_string(),
            color_scheme: "Default Dark".to_string(),
            scrollback_lines: 10000,
            encoding: "UTF-8".to_string(),
        }
    }
}

/// 获取配置文件路径
fn config_path() -> PathBuf {
    let mut path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    path.push("rshell");
    path.push("config.toml");
    path
}

/// 加载配置
pub fn load_config() -> anyhow::Result<AppConfig> {
    let path = config_path();
    debug!("Loading config from: {:?}", path);

    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let config: AppConfig = toml::from_str(&content)?;
        info!("Config loaded from {:?}", path);
        Ok(config)
    } else {
        info!("Config file not found, using defaults");
        Ok(AppConfig::default())
    }
}

/// 保存配置
pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path();
    debug!("Saving config to: {:?}", path);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    info!("Config saved to {:?}", path);
    Ok(())
}
