//! 主题与配色方案管理
//!
//! 管理应用主题和终端配色方案。

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use rshell_api::events::AppEvent;
use rshell_api::types::{AppTheme, TerminalColorScheme, ThemeColors, ThemeMode};

use crate::error::CoreError;
use crate::event_bus::EventBus;

/// 主题管理器
pub struct ThemeManager {
    /// 当前应用主题
    current_theme: Arc<RwLock<AppTheme>>,
    /// 当前终端配色方案
    current_color_scheme: Arc<RwLock<TerminalColorScheme>>,
    /// 可用主题列表
    themes: Arc<RwLock<HashMap<String, AppTheme>>>,
    /// 可用配色方案列表
    color_schemes: Arc<RwLock<HashMap<String, TerminalColorScheme>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
}

impl ThemeManager {
    /// 创建新的主题管理器
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        let mut themes = HashMap::new();
        let mut color_schemes = HashMap::new();

        // 初始化内置主题
        themes.insert("Dark".to_string(), Self::default_dark_theme());
        themes.insert("Light".to_string(), Self::default_light_theme());

        // 初始化内置配色方案
        color_schemes.insert("Monokai".to_string(), Self::monokai_scheme());
        color_schemes.insert("Solarized Dark".to_string(), Self::solarized_dark_scheme());
        color_schemes.insert("Solarized Light".to_string(), Self::solarized_light_scheme());
        color_schemes.insert("Dracula".to_string(), Self::dracula_scheme());
        color_schemes.insert("Nord".to_string(), Self::nord_scheme());
        color_schemes.insert("Default Dark".to_string(), Self::default_dark_scheme());

        let current_theme = themes.get("Dark").unwrap().clone();
        let current_scheme = color_schemes.get("Default Dark").unwrap().clone();

        Self {
            current_theme: Arc::new(RwLock::new(current_theme)),
            current_color_scheme: Arc::new(RwLock::new(current_scheme)),
            themes: Arc::new(RwLock::new(themes)),
            color_schemes: Arc::new(RwLock::new(color_schemes)),
            event_bus,
        }
    }

    /// 设置应用主题
    pub async fn set_theme(&self, theme_name: &str) -> Result<(), CoreError> {
        let themes = self.themes.read().await;
        let theme = themes.get(theme_name)
            .ok_or_else(|| CoreError::InvalidState(format!("Theme '{}' not found", theme_name)))?
            .clone();
        drop(themes);

        let mut current = self.current_theme.write().await;
        *current = theme.clone();
        drop(current);

        info!("Theme changed to: {}", theme_name);
        self.event_bus.publish(AppEvent::ThemeChanged { theme });
        Ok(())
    }

    /// 设置终端配色方案
    pub async fn set_color_scheme(&self, scheme_name: &str) -> Result<(), CoreError> {
        let schemes = self.color_schemes.read().await;
        let scheme = schemes.get(scheme_name)
            .ok_or_else(|| CoreError::InvalidState(format!("Color scheme '{}' not found", scheme_name)))?
            .clone();
        drop(schemes);

        let mut current = self.current_color_scheme.write().await;
        *current = scheme.clone();
        drop(current);

        info!("Color scheme changed to: {}", scheme_name);
        self.event_bus.publish(AppEvent::ColorSchemeChanged { scheme });
        Ok(())
    }

    /// 导入自定义配色方案
    pub async fn import_color_scheme(&self, scheme: TerminalColorScheme) -> Result<(), CoreError> {
        let name = scheme.name.clone();
        let mut schemes = self.color_schemes.write().await;
        schemes.insert(name.clone(), scheme);
        drop(schemes);

        info!("Color scheme imported: {}", name);
        self.event_bus.publish(AppEvent::ColorSchemeListChanged);
        Ok(())
    }

    /// 获取当前主题
    pub async fn current_theme(&self) -> AppTheme {
        self.current_theme.read().await.clone()
    }

    /// 获取当前配色方案
    pub async fn current_color_scheme(&self) -> TerminalColorScheme {
        self.current_color_scheme.read().await.clone()
    }

    /// 获取可用主题列表
    pub async fn list_themes(&self) -> Vec<String> {
        self.themes.read().await.keys().cloned().collect()
    }

    /// 获取可用配色方案列表
    pub async fn list_color_schemes(&self) -> Vec<String> {
        self.color_schemes.read().await.keys().cloned().collect()
    }

    // ===== 内置主题 =====

    fn default_dark_theme() -> AppTheme {
        AppTheme {
            name: "Dark".to_string(),
            mode: ThemeMode::Dark,
            colors: ThemeColors {
                background: 0x1e1e2e,
                foreground: 0xcdd6f4,
                accent: 0x89b4fa,
                border: 0x45475a,
                sidebar_bg: 0x181825,
                toolbar_bg: 0x1e1e2e,
                statusbar_bg: 0x181825,
                selection_bg: 0x45475a,
                hover_bg: 0x313244,
            },
        }
    }

    fn default_light_theme() -> AppTheme {
        AppTheme {
            name: "Light".to_string(),
            mode: ThemeMode::Light,
            colors: ThemeColors {
                background: 0xffffff,
                foreground: 0x1e1e2e,
                accent: 0x1e66f5,
                border: 0xcccdd6,
                sidebar_bg: 0xf5f5f5,
                toolbar_bg: 0xffffff,
                statusbar_bg: 0xf5f5f5,
                selection_bg: 0xcccdd6,
                hover_bg: 0xe6e6e6,
            },
        }
    }

    // ===== 内置配色方案 =====

    fn default_dark_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Default Dark".to_string(),
            ansi_colors: [
                0x000000, 0xcd0000, 0x00cd00, 0xcdcd00,
                0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
                0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00,
                0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
            ],
            default_fg: 0xcdd6f4,
            default_bg: 0x1e1e2e,
            cursor_fg: 0x1e1e2e,
            cursor_bg: 0xf5e0dc,
            selection_fg: 0xcdd6f4,
            selection_bg: 0x45475a,
        }
    }

    fn monokai_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Monokai".to_string(),
            ansi_colors: [
                0x272822, 0xf92672, 0xa6e22e, 0xf4bf75,
                0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75,
                0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
            ],
            default_fg: 0xf8f8f2,
            default_bg: 0x272822,
            cursor_fg: 0x272822,
            cursor_bg: 0xf8f8f0,
            selection_fg: 0xf8f8f2,
            selection_bg: 0x49483e,
        }
    }

    fn solarized_dark_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Solarized Dark".to_string(),
            ansi_colors: [
                0x073642, 0xdc322f, 0x859900, 0xb58900,
                0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83,
                0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ],
            default_fg: 0x839496,
            default_bg: 0x002b36,
            cursor_fg: 0x002b36,
            cursor_bg: 0x839496,
            selection_fg: 0x93a1a1,
            selection_bg: 0x073642,
        }
    }

    fn solarized_light_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Solarized Light".to_string(),
            ansi_colors: [
                0x073642, 0xdc322f, 0x859900, 0xb58900,
                0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83,
                0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ],
            default_fg: 0x657b83,
            default_bg: 0xfdf6e3,
            cursor_fg: 0xfdf6e3,
            cursor_bg: 0x657b83,
            selection_fg: 0x586e75,
            selection_bg: 0xeee8d5,
        }
    }

    fn dracula_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Dracula".to_string(),
            ansi_colors: [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c,
                0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5,
                0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ],
            default_fg: 0xf8f8f2,
            default_bg: 0x282a36,
            cursor_fg: 0x282a36,
            cursor_bg: 0xf8f8f2,
            selection_fg: 0xf8f8f2,
            selection_bg: 0x44475a,
        }
    }

    fn nord_scheme() -> TerminalColorScheme {
        TerminalColorScheme {
            name: "Nord".to_string(),
            ansi_colors: [
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b,
                0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b,
                0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
            ],
            default_fg: 0xd8dee9,
            default_bg: 0x2e3440,
            cursor_fg: 0x2e3440,
            cursor_bg: 0xd8dee9,
            selection_fg: 0xe5e9f0,
            selection_bg: 0x434c5e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::ThemeMode;
    use std::sync::Arc;

    fn make_bus() -> Arc<EventBus> {
        Arc::new(EventBus::new())
    }

    #[tokio::test]
    async fn test_theme_switch() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus.clone());

        // 默认是 Dark
        assert_eq!(mgr.current_theme().await.name, "Dark");
        assert_eq!(mgr.current_theme().await.mode, ThemeMode::Dark);

        // 切换到 Light
        mgr.set_theme("Light").await.unwrap();
        assert_eq!(mgr.current_theme().await.name, "Light");
        assert_eq!(mgr.current_theme().await.mode, ThemeMode::Light);

        // 切回 Dark
        mgr.set_theme("Dark").await.unwrap();
        assert_eq!(mgr.current_theme().await.name, "Dark");
    }

    #[tokio::test]
    async fn test_theme_switch_invalid_name() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus);

        let result = mgr.set_theme("nonexistent").await;
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("not found"));
    }

    #[tokio::test]
    async fn test_color_scheme_switch() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus.clone());

        // 默认是 Default Dark
        assert_eq!(mgr.current_color_scheme().await.name, "Default Dark");

        // 切换到 Monokai
        mgr.set_color_scheme("Monokai").await.unwrap();
        assert_eq!(mgr.current_color_scheme().await.name, "Monokai");

        // 切换到 Dracula
        mgr.set_color_scheme("Dracula").await.unwrap();
        assert_eq!(mgr.current_color_scheme().await.name, "Dracula");
    }

    #[tokio::test]
    async fn test_color_scheme_switch_invalid_name() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus);

        let result = mgr.set_color_scheme("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_themes() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus);

        let themes = mgr.list_themes().await;
        assert!(themes.contains(&"Dark".to_string()));
        assert!(themes.contains(&"Light".to_string()));
        assert_eq!(themes.len(), 2);
    }

    #[tokio::test]
    async fn test_list_color_schemes() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus);

        let schemes = mgr.list_color_schemes().await;
        assert!(schemes.contains(&"Monokai".to_string()));
        assert!(schemes.contains(&"Dracula".to_string()));
        assert!(schemes.contains(&"Nord".to_string()));
        assert!(schemes.contains(&"Solarized Dark".to_string()));
        assert!(schemes.contains(&"Default Dark".to_string()));
        assert_eq!(schemes.len(), 6);
    }

    #[tokio::test]
    async fn test_import_color_scheme() {
        let bus = make_bus();
        let mgr = ThemeManager::new(bus.clone());

        let custom = TerminalColorScheme {
            name: "MyScheme".to_string(),
            ansi_colors: [0u32; 16],
            default_fg: 0xffffff,
            default_bg: 0x000000,
            cursor_fg: 0xffffff,
            cursor_bg: 0x00ff00,
            selection_fg: 0xffffff,
            selection_bg: 0x0000ff,
        };

        mgr.import_color_scheme(custom.clone()).await.unwrap();

        // 现在列表里应该多一个
        let schemes = mgr.list_color_schemes().await;
        assert!(schemes.contains(&"MyScheme".to_string()));
        assert_eq!(schemes.len(), 7);

        // 可以切换到它
        mgr.set_color_scheme("MyScheme").await.unwrap();
        assert_eq!(mgr.current_color_scheme().await.name, "MyScheme");
    }
}
