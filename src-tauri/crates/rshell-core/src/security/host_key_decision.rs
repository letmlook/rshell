//! 主机密钥决策注册表
//!
//! SSH 握手期间 `SshHandler::check_server_key` 是**同步 trait 方法**,不能直接
//! `.await`。但 UI 端需要异步响应。为了把"同步检查"翻译成"异步等待决策",
//! 引入一个 registry:
//!
//! 1. `connect_ssh` 注入一个 `Arc<HostKeyDecisionRegistry>`
//! 2. 未知 host key 时分配 `decision_id`、建 `oneshot::channel`、sender 存进
//!    registry,然后 `event_bus.publish(HostKeyMismatch { decision_id, ... })`
//! 3. `Handle::current().block_on(rx)` 同步阻塞直到 UI 端 `AppCommand::DecideHostKey`
//!    通过 `CommandDispatcher` 取出 sender 并 `send(HostKeyDecision)` 唤醒
//! 4. `DecideHostKey` 找不到 decision_id 时(超时/竞态),返回 reject

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;

use rshell_api::AppEvent;
use rshell_protocol::ssh::{HostKeyDecision, HostKeyDecisionRequest, HostKeyDecisionSink};

use crate::event_bus::EventBus;

/// 主机密钥决策注册表
#[derive(Clone)]
pub struct HostKeyDecisionRegistry {
    inner: Arc<Mutex<HashMap<Uuid, oneshot::Sender<HostKeyDecision>>>>,
    /// 事件总线（用于在 `publish_request` 时投递 `HostKeyMismatch` 给 UI 端）
    event_bus: Arc<EventBus>,
}

impl HostKeyDecisionRegistry {
    /// 创建带事件总线的 registry
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            event_bus,
        }
    }

    /// 注册一个新的待决策项,返回 (decision_id, receiver)
    ///
    /// 调用方应在随后 publish `HostKeyMismatch { decision_id, ... }` 事件,
    /// 然后 `block_on(receiver)` 等待 UI 响应。
    pub fn register(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.inner
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .insert(id, tx);
        (id, rx)
    }

    /// 提交决策（取出发送者并 send）
    ///
    /// 找不到 decision_id（UI 端超时/竞态/双重决策）时返回 false,调用方
    /// 应当视为 reject。
    pub fn resolve(&self, decision_id: Uuid, decision: HostKeyDecision) -> bool {
        let mut map = self
            .inner
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned");
        if let Some(tx) = map.remove(&decision_id) {
            // send 失败说明接收方已 drop(SshHandler 提前返回),忽略
            let _ = tx.send(decision);
            true
        } else {
            false
        }
    }
}

impl HostKeyDecisionSink for HostKeyDecisionRegistry {
    fn register_decision(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>) {
        HostKeyDecisionRegistry::register(self)
    }

    fn publish_request(&self, info: HostKeyDecisionRequest) {
        self.event_bus.publish(AppEvent::HostKeyMismatch {
            decision_id: info.decision_id,
            host: info.host,
            port: info.port,
            key_type: info.key_type,
            expected: String::new(),
            received: info.fingerprint,
            public_key_blob: info.public_key_blob,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    #[test]
    fn register_then_resolve_sends_decision() {
        let rt = Runtime::new().unwrap();
        let reg = HostKeyDecisionRegistry::new(Arc::new(EventBus::new()));
        let (id, rx) = reg.register();

        rt.spawn(async move {
            reg.resolve(
                id,
                HostKeyDecision {
                    fingerprint: "fp".to_string(),
                    key_blob: "blob".to_string(),
                    accept: true,
                    permanent: false,
                },
            );
        });

        let got = rt.block_on(rx).unwrap();
        assert!(got.accept);
        assert_eq!(got.fingerprint, "fp");
    }

    #[test]
    fn resolve_unknown_id_returns_false() {
        let reg = HostKeyDecisionRegistry::new(Arc::new(EventBus::new()));
        assert!(!reg.resolve(
            Uuid::new_v4(),
            HostKeyDecision {
                fingerprint: "x".to_string(),
                key_blob: "y".to_string(),
                accept: false,
                permanent: false,
            }
        ));
    }

    #[test]
    fn resolve_only_works_once() {
        let reg = HostKeyDecisionRegistry::new(Arc::new(EventBus::new()));
        let (id, _rx) = reg.register();
        assert!(reg.resolve(
            id,
            HostKeyDecision {
                fingerprint: String::new(),
                key_blob: String::new(),
                accept: true,
                permanent: false,
            }
        ));
        // 第二次 resolve 同一 id 应返回 false
        assert!(!reg.resolve(
            id,
            HostKeyDecision {
                fingerprint: String::new(),
                key_blob: String::new(),
                accept: true,
                permanent: false,
            }
        ));
    }

    #[test]
    fn publish_request_sends_host_key_mismatch() {
        let bus = Arc::new(EventBus::new());
        let reg = HostKeyDecisionRegistry::new(bus.clone());
        let got = Arc::new(std::sync::Mutex::new(None::<AppEvent>));
        let g = got.clone();
        bus.subscribe(move |event| {
            if let AppEvent::HostKeyMismatch { .. } = event {
                *g.lock().unwrap() = Some(event.clone());
            }
        });
        let id = Uuid::new_v4();
        reg.publish_request(HostKeyDecisionRequest {
            decision_id: id,
            host: "example.com".to_string(),
            port: 22,
            key_type: "Ed25519".to_string(),
            fingerprint: "SHA256:xxx".to_string(),
            public_key_blob: "ssh-ed25519 AAAA...".to_string(),
        });
        let evt = got.lock().unwrap().clone().unwrap();
        if let AppEvent::HostKeyMismatch {
            decision_id,
            host,
            port,
            received,
            ..
        } = evt
        {
            assert_eq!(decision_id, id);
            assert_eq!(host, "example.com");
            assert_eq!(port, 22);
            assert_eq!(received, "SHA256:xxx");
        } else {
            panic!("expected HostKeyMismatch event");
        }
    }
}
