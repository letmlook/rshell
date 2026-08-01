//! EventBus → Tauri `app.emit` 桥（设计 §1.1 / §3.1）
//!
//! 后端 service 通过 `EventBus::publish` 推 `AppEvent`；本模块订阅后转
//! `emit("rshell://event", payload)` 推到前端 `main` window。
//! 前端 `src/ipc/events.ts:10` 已固定事件名为 `"rshell://event"`。
//!
//! 启动方式：`subscribe_bridge(event_bus, app_handle)` 在 `tauri::Builder::setup`
//! 内调用一次,返回的 `SubscriptionId` 在主进程退出时可丢弃 —— 订阅随 EventBus
//! 一起释放（同步回调模型,无需 spawn）。

use rshell_api::AppEvent;
use rshell_core::EventBus;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tracing::{error, warn};

const EVENT_NAME: &str = "rshell://event";

/// 注册 EventBus → Tauri emit 桥。返回 `SubscriptionId` —— 当前不主动 unsubscribe
/// （EventBus 与 AppHandle 同生命周期）。切片 1 阶段保留返回值供调试。
pub fn subscribe_bridge(event_bus: Arc<EventBus>, app_handle: AppHandle) -> u64 {
    event_bus.subscribe(move |event| {
        emit_one(&app_handle, event);
    })
}

fn emit_one(app_handle: &AppHandle, event: &AppEvent) {
    let kind_label = event_kind_label(event);
    if let Err(e) = app_handle.emit(EVENT_NAME, event) {
        warn!(error = %e, kind = %kind_label, "emit rshell://event failed");
    }
}

/// 从 `AppEvent` 派生稳定的日志标签。
///
/// 不维护枚举变体名镜像 —— 序列化为 `serde_json::Value` 后读顶层 `"kind"` 字段
/// (serde tagged enum 默认带 kind 标签;若未来调整 derive,本函数会回退到 "unknown")。
fn event_kind_label(event: &AppEvent) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str().map(|s| s.to_string())))
        .unwrap_or_else(|| "unknown".to_string())
}

/// 兜底：万一 EventBus 触发 panic（订阅者 callback 抛错），本函数把 panic 转化为
/// error 日志而非崩溃整个桥。当前实现未做 catch_unwind（不阻塞切片 1 进度）；
/// 切片 6 的密钥事件高敏感场景再考虑接入。
#[allow(dead_code)]
fn handle_subscriber_panic<T>(result: std::thread::Result<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            error!(panic = ?e, "EventBus subscriber panicked; bridge skipping");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_label_extracts_tagged_enum_tag() {
        let e = AppEvent::SessionListChanged;
        assert_eq!(event_kind_label(&e), "SessionListChanged");

        let e = AppEvent::TransferProgress {
            task_id: uuid::Uuid::nil(),
            bytes: 0,
            total: 0,
            speed_bps: 0.0,
        };
        assert_eq!(event_kind_label(&e), "TransferProgress");
    }
}