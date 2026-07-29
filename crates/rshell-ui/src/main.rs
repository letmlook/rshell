//! RShell 应用入口
//!
//! GPUI 应用初始化和主窗口创建。

mod app;
mod bridge;
mod view_models;
mod views;

use gpui::{App, AppContext};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    // 初始化日志
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "info,rshell=debug".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting RShell...");

    // 初始化后端服务（tokio runtime + CommandDispatcher + EventBus）
    let (bridge, _runtime) = bridge::init_backend();

    // 创建并运行 GPUI 应用
    let app = gpui::Application::new();
    app.run(|cx: &mut App| {
        tracing::info!("GPUI application initialized");

        // 设置应用标题和窗口大小
        let options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::Point::new(gpui::px(100.0), gpui::px(100.0)),
                size: gpui::Size {
                    width: gpui::px(1200.0),
                    height: gpui::px(800.0),
                },
            })),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("RShell - Remote Shell".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            ..Default::default()
        };

        // 打开主窗口，将 bridge 传递给 RshellApp
        let _window = cx.open_window(options, |_window, cx| {
            cx.new(|cx| app::RshellApp::new(bridge, cx))
        }).expect("Failed to open main window");

        tracing::info!("Main window created");
    });
}
