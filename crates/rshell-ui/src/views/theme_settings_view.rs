//! 主题设置视图
//!
//! 显示应用主题和终端配色方案选择。

use gpui::*;
use rshell_api::types::{AppTheme, TerminalColorScheme, ThemeMode};
use rshell_api::AppEvent;

/// 主题设置视图
pub struct ThemeSettingsView {
    /// 当前主题
    current_theme: Option<AppTheme>,
    /// 可用主题列表
    available_themes: Vec<String>,
    /// 当前配色方案
    current_scheme: Option<TerminalColorScheme>,
    /// 可用配色方案列表
    available_schemes: Vec<String>,
    /// 选中的主题索引
    selected_theme: Option<usize>,
    /// 选中的配色方案索引
    selected_scheme: Option<usize>,
}

impl ThemeSettingsView {
    /// 创建新的主题设置视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            current_theme: None,
            available_themes: vec!["Dark".to_string(), "Light".to_string()],
            current_scheme: None,
            available_schemes: vec![
                "Default Dark".to_string(),
                "Monokai".to_string(),
                "Solarized Dark".to_string(),
                "Solarized Light".to_string(),
                "Dracula".to_string(),
                "Nord".to_string(),
            ],
            selected_theme: None,
            selected_scheme: None,
        }
    }

    /// 设置当前主题
    pub fn set_current_theme(&mut self, theme: AppTheme) {
        self.current_theme = Some(theme);
    }

    /// 设置当前配色方案
    pub fn set_current_scheme(&mut self, scheme: TerminalColorScheme) {
        self.current_scheme = Some(scheme);
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::ThemeChanged { theme } => {
                self.current_theme = Some(theme.clone());
            }
            AppEvent::ColorSchemeChanged { scheme } => {
                self.current_scheme = Some(scheme.clone());
            }
            _ => {}
        }
    }
}

impl Render for ThemeSettingsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(
                div()
                    .h(px(40.0))
                    .bg(rgb(0x252526))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .child(
                        div()
                            .child("主题设置")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(16.0))
                    .child(self.render_theme_section())
                    .child(div().h(px(16.0)))
                    .child(self.render_scheme_section()),
            )
    }
}

impl ThemeSettingsView {
    fn render_theme_section(&self) -> impl IntoElement {
        let current_name = self.current_theme.as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Dark".to_string());

        let mode_text = match self.current_theme.as_ref().map(|t| t.mode) {
            Some(ThemeMode::Dark) => "深色模式",
            Some(ThemeMode::Light) => "浅色模式",
            Some(ThemeMode::System) => "跟随系统",
            None => "未知",
        };

        div()
            .child(
                div()
                    .mb(px(8.0))
                    .child("应用主题")
                    .text_color(rgb(0xcccccc))
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::BOLD),
            )
            .child(
                div()
                    .mb(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .child(format!("当前: {}", current_name))
                            .text_color(rgb(0x4ec9b0))
                            .text_size(px(11.0)),
                    )
                    .child(
                        div()
                            .child(format!("({})", mode_text))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    ),
            )
            .child(
                div()
                    .child("可用主题:")
                    .text_color(rgb(0x808080))
                    .text_size(px(10.0))
                    .mb(px(4.0)),
            )
            .children(self.available_themes.iter().map(|name| {
                let is_current = self.current_theme.as_ref().map(|t| &t.name) == Some(name);
                let bg = if is_current { rgb(0x094771) } else { rgb(0x2d2d2d) };
                div()
                    .bg(bg)
                    .rounded(px(4.0))
                    .mb(px(2.0))
                    .p(px(6.0))
                    .cursor_pointer()
                    .child(
                        div()
                            .child(name.clone())
                            .text_color(rgb(0xcccccc))
                            .text_size(px(11.0)),
                    )
            }))
    }

    fn render_scheme_section(&self) -> impl IntoElement {
        let current_name = self.current_scheme.as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Default Dark".to_string());

        div()
            .child(
                div()
                    .mb(px(8.0))
                    .child("终端配色方案")
                    .text_color(rgb(0xcccccc))
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::BOLD),
            )
            .child(
                div()
                    .mb(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .child(format!("当前: {}", current_name))
                            .text_color(rgb(0x4ec9b0))
                            .text_size(px(11.0)),
                    ),
            )
            .child(
                div()
                    .child("可用方案:")
                    .text_color(rgb(0x808080))
                    .text_size(px(10.0))
                    .mb(px(4.0)),
            )
            .children(self.available_schemes.iter().map(|name| {
                let is_current = self.current_scheme.as_ref().map(|s| &s.name) == Some(name);
                let bg = if is_current { rgb(0x094771) } else { rgb(0x2d2d2d) };
                div()
                    .bg(bg)
                    .rounded(px(4.0))
                    .mb(px(2.0))
                    .p(px(6.0))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .child(name.clone())
                                    .text_color(rgb(0xcccccc))
                                    .text_size(px(11.0)),
                            ),
                    )
            }))
    }
}
