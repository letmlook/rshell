//! 事件总线（后端 → 前端）
//!
//! 后端 Service 发布事件，前端 ViewModel 订阅感兴趣的事件。
//! 这是前后端分离架构中后端向前端通知状态变化的唯一通道。

use rshell_api::AppEvent;
use std::sync::{Arc, RwLock};
use tracing::{debug, instrument};

/// 事件订阅句柄（用于取消订阅）
pub type SubscriptionId = u64;

/// 事件总线
pub struct EventBus {
    /// 订阅者列表
    subscribers: Arc<RwLock<Vec<(SubscriptionId, Box<dyn Fn(&AppEvent) + Send + Sync>)>>>,
    /// 下一个订阅 ID
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl EventBus {
    /// 创建新的事件总线
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// 发布事件（后端调用）
    #[instrument(skip(self, event), fields(event = ?std::any::type_name::<AppEvent>()))]
    pub fn publish(&self, event: AppEvent) {
        debug!("Publishing event");
        let subscribers = self.subscribers.read().unwrap();
        for (_, handler) in subscribers.iter() {
            handler(&event);
        }
    }

    /// 订阅事件（前端调用）
    ///
    /// 返回订阅 ID，可用于取消订阅。
    pub fn subscribe<F>(&self, handler: F) -> SubscriptionId
    where
        F: Fn(&AppEvent) + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut subscribers = self.subscribers.write().unwrap();
        subscribers.push((id, Box::new(handler)));
        debug!(subscription_id = id, "New subscription");
        id
    }

    /// 取消订阅
    pub fn unsubscribe(&self, id: SubscriptionId) {
        let mut subscribers = self.subscribers.write().unwrap();
        subscribers.retain(|(sid, _)| *sid != id);
        debug!(subscription_id = id, "Unsubscribed");
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// EventBus 需要是 Send + Sync
unsafe impl Send for EventBus {}
unsafe impl Sync for EventBus {}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::ConnectionState;
    use uuid::Uuid;

    #[test]
    fn test_publish_subscribe() {
        let bus = EventBus::new();
        let received = Arc::new(RwLock::new(Vec::new()));

        let received_clone = received.clone();
        bus.subscribe(move |event| {
            received_clone.write().unwrap().push(format!("{:?}", event));
        });

        let session_id = Uuid::new_v4();
        bus.publish(AppEvent::ConnectionStateChanged {
            session_id,
            state: ConnectionState::Connected,
            info: None,
        });

        let events = received.read().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("ConnectionStateChanged"));
    }

    #[test]
    fn test_unsubscribe() {
        let bus = EventBus::new();
        let received = Arc::new(RwLock::new(0));

        let received_clone = received.clone();
        let id = bus.subscribe(move |_| {
            *received_clone.write().unwrap() += 1;
        });

        bus.publish(AppEvent::SessionListChanged);
        assert_eq!(*received.read().unwrap(), 1);

        bus.unsubscribe(id);
        bus.publish(AppEvent::SessionListChanged);
        assert_eq!(*received.read().unwrap(), 1); // 不再增加
    }
}
