//! Keyed host-key decision registry.
//!
//! Each SSH handshake gets an independent UUID and a single terminal state.
//! A decision for one id can never resolve, cancel, or dismiss another id.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;

use rshell_api::events::HostKeyDecisionState;
use rshell_api::AppEvent;
use rshell_protocol::ssh::{HostKeyDecision, HostKeyDecisionRequest, HostKeyDecisionSink};

use crate::event_bus::EventBus;

/// Registry of host-key handshakes waiting for a human decision.
#[derive(Clone)]
pub struct HostKeyDecisionRegistry {
    inner: Arc<Mutex<HashMap<Uuid, oneshot::Sender<HostKeyDecision>>>>,
    requests: Arc<Mutex<HashMap<Uuid, HostKeyDecisionRequest>>>,
    states: Arc<Mutex<HashMap<Uuid, HostKeyDecisionState>>>,
    event_bus: Arc<EventBus>,
}

impl HostKeyDecisionRegistry {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
            states: Arc::new(Mutex::new(HashMap::new())),
            event_bus,
        }
    }

    /// Register a new pending decision and return its one-shot receiver.
    pub fn register(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.inner
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .insert(id, tx);
        self.states
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .insert(id, HostKeyDecisionState::Pending);
        (id, rx)
    }

    pub fn request_info(&self, decision_id: Uuid) -> Option<HostKeyDecisionRequest> {
        self.requests
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .get(&decision_id)
            .cloned()
    }

    pub fn state(&self, decision_id: Uuid) -> Option<HostKeyDecisionState> {
        self.states
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .get(&decision_id)
            .copied()
    }

    /// Atomically claim a pending decision for a persistence operation.
    ///
    /// Trust-always uses `DecidedTrustAlways` as a claim state while the
    /// trusted-store write is in flight. A failed write can release it back
    /// to `Pending`; no handshake one-shot is sent until persistence succeeds.
    pub fn claim(
        &self,
        decision_id: Uuid,
        state: HostKeyDecisionState,
    ) -> Option<HostKeyDecisionRequest> {
        let request = self.request_info(decision_id);
        let mut states = self
            .states
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned");
        if states.get(&decision_id).copied() != Some(HostKeyDecisionState::Pending) {
            return None;
        }
        states.insert(decision_id, state);
        request
    }

    /// Release a failed persistence claim without consuming the handshake.
    pub fn release_claim(&self, decision_id: Uuid, claimed: HostKeyDecisionState) -> bool {
        let mut states = self
            .states
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned");
        if states.get(&decision_id).copied() != Some(claimed) {
            return false;
        }
        states.insert(decision_id, HostKeyDecisionState::Pending);
        true
    }

    /// Settle a claimed decision and wake exactly its SSH handshake.
    pub fn settle_claimed(
        &self,
        decision_id: Uuid,
        claimed: HostKeyDecisionState,
        decision: HostKeyDecision,
    ) -> bool {
        let terminal = match claimed {
            HostKeyDecisionState::DecidedTrustOnce => HostKeyDecisionState::DecidedTrustOnce,
            HostKeyDecisionState::DecidedTrustAlways => HostKeyDecisionState::DecidedTrustAlways,
            HostKeyDecisionState::DecidedReject => HostKeyDecisionState::DecidedReject,
            _ => return false,
        };
        let sender = {
            if !self
                .inner
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned")
                .contains_key(&decision_id)
            {
                return false;
            }
            let mut states = self
                .states
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned");
            if states.get(&decision_id).copied() != Some(claimed) {
                return false;
            }
            states.insert(decision_id, terminal);
            self.inner
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned")
                .remove(&decision_id)
        };
        self.requests
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .remove(&decision_id);
        if let Some(sender) = sender {
            let _ = sender.send(decision);
        }
        self.publish_state(decision_id, terminal);
        true
    }

    /// Resolve a pending decision. Kept as the simple public seam used by
    /// non-persistent callers and tests.
    pub fn resolve(&self, decision_id: Uuid, decision: HostKeyDecision) -> bool {
        let claimed = if decision.accept && decision.permanent {
            HostKeyDecisionState::DecidedTrustAlways
        } else if decision.accept {
            HostKeyDecisionState::DecidedTrustOnce
        } else {
            HostKeyDecisionState::DecidedReject
        };
        let _ = self.claim(decision_id, claimed);
        self.settle_claimed(
            decision_id,
            claimed,
            HostKeyDecision {
                fingerprint: decision.fingerprint,
                key_blob: decision.key_blob,
                accept: decision.accept,
                permanent: decision.permanent,
            },
        )
    }

    /// Cancel one decision without accepting its handshake.
    pub fn cancel(&self, decision_id: Uuid) -> bool {
        self.close(decision_id, HostKeyDecisionState::Cancelled)
    }

    /// Mark one decision stale because its bounded wait expired.
    pub fn expire(&self, decision_id: Uuid) -> bool {
        self.close(decision_id, HostKeyDecisionState::Expired)
    }

    fn close(&self, decision_id: Uuid, state: HostKeyDecisionState) -> bool {
        let sender = {
            if !self
                .inner
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned")
                .contains_key(&decision_id)
            {
                return false;
            }
            let mut states = self
                .states
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned");
            let current = states.get(&decision_id).copied();
            if !matches!(
                current,
                Some(HostKeyDecisionState::Pending)
                    | Some(HostKeyDecisionState::DecidedTrustOnce)
                    | Some(HostKeyDecisionState::DecidedTrustAlways)
                    | Some(HostKeyDecisionState::DecidedReject)
            ) {
                return false;
            }
            states.insert(decision_id, state);
            self.inner
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned")
                .remove(&decision_id)
        };
        self.requests
            .lock()
            .expect("HostKeyDecisionRegistry mutex poisoned")
            .remove(&decision_id);
        // Dropping the sender cancels the waiting protocol future; sending a
        // synthetic reject would make cancellation look like a user decision.
        drop(sender);
        self.publish_state(decision_id, state);
        true
    }

    fn publish_state(&self, decision_id: Uuid, state: HostKeyDecisionState) {
        self.event_bus
            .publish(AppEvent::HostKeyDecisionStateChanged { decision_id, state });
    }
}

impl HostKeyDecisionSink for HostKeyDecisionRegistry {
    fn register_decision(&self) -> (Uuid, oneshot::Receiver<HostKeyDecision>) {
        self.register()
    }

    fn publish_request(&self, info: HostKeyDecisionRequest) {
        let accepted = {
            let states = self
                .states
                .lock()
                .expect("HostKeyDecisionRegistry mutex poisoned");
            if states.get(&info.decision_id).copied() != Some(HostKeyDecisionState::Pending) {
                false
            } else {
                self.requests
                    .lock()
                    .expect("HostKeyDecisionRegistry mutex poisoned")
                    .insert(info.decision_id, info.clone());
                true
            }
        };
        if !accepted {
            return;
        }
        self.event_bus.publish(AppEvent::HostKeyMismatch {
            decision_id: info.decision_id,
            host: info.host,
            port: info.port,
            key_type: info.key_type,
            expected: info.expected,
            received: info.fingerprint,
            public_key_blob: info.public_key_blob,
        });
    }

    fn cancel_decision(&self, decision_id: Uuid) {
        let _ = self.cancel(decision_id);
    }

    fn expire_decision(&self, decision_id: Uuid) {
        let _ = self.expire(decision_id);
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

    #[tokio::test]
    async fn concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other() {
        let bus = Arc::new(EventBus::new());
        let reg = HostKeyDecisionRegistry::new(bus.clone());
        let events = Arc::new(std::sync::Mutex::new(Vec::<AppEvent>::new()));
        let sink = events.clone();
        bus.subscribe(move |event| sink.lock().unwrap().push(event.clone()));

        let (first_id, first_rx) = reg.register();
        let (second_id, second_rx) = reg.register();
        assert_ne!(first_id, second_id);
        reg.publish_request(HostKeyDecisionRequest {
            decision_id: first_id,
            host: "first.test".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:first".into(),
            expected: String::new(),
            public_key_blob: "ssh-ed25519 AAAAfirst".into(),
        });
        reg.publish_request(HostKeyDecisionRequest {
            decision_id: second_id,
            host: "second.test".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:second".into(),
            expected: String::new(),
            public_key_blob: "ssh-ed25519 AAAAsecond".into(),
        });

        assert!(reg.resolve(
            first_id,
            HostKeyDecision {
                fingerprint: "SHA256:first".into(),
                key_blob: "ssh-ed25519 AAAAfirst".into(),
                accept: true,
                permanent: false,
            },
        ));
        assert!(reg.request_info(second_id).is_some());
        assert!(reg.resolve(
            second_id,
            HostKeyDecision {
                fingerprint: "SHA256:second".into(),
                key_blob: "ssh-ed25519 AAAAsecond".into(),
                accept: false,
                permanent: false,
            },
        ));
        assert!(first_rx.await.unwrap().accept);
        assert!(!second_rx.await.unwrap().accept);
        let states: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AppEvent::HostKeyDecisionStateChanged { decision_id, state } => {
                    Some((*decision_id, *state))
                }
                _ => None,
            })
            .collect();
        assert!(states.contains(&(first_id, HostKeyDecisionState::DecidedTrustOnce)));
        assert!(states.contains(&(second_id, HostKeyDecisionState::DecidedReject)));
    }

    #[tokio::test]
    async fn cancellation_and_expiry_close_only_their_decision_and_drop_late_events() {
        let bus = Arc::new(EventBus::new());
        let reg = HostKeyDecisionRegistry::new(bus.clone());
        let (cancelled_id, cancelled_rx) = reg.register();
        let (expired_id, expired_rx) = reg.register();
        reg.cancel(cancelled_id);
        assert!(cancelled_rx.await.is_err());
        assert!(!reg.resolve(
            cancelled_id,
            HostKeyDecision {
                fingerprint: "SHA256:cancelled".into(),
                key_blob: "blob".into(),
                accept: true,
                permanent: false,
            },
        ));
        assert!(reg.expire(expired_id));
        assert!(expired_rx.await.is_err());
        assert!(!reg.expire(expired_id));
        let (next_id, next_rx) = reg.register();
        reg.publish_request(HostKeyDecisionRequest {
            decision_id: next_id,
            host: "next.test".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:next".into(),
            expected: String::new(),
            public_key_blob: "ssh-ed25519 AAAAnext".into(),
        });
        assert!(reg.request_info(next_id).is_some());
        assert!(reg.cancel(next_id));
        assert!(next_rx.await.is_err());
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
        let (id, _rx) = reg.register();
        reg.publish_request(HostKeyDecisionRequest {
            decision_id: id,
            host: "example.com".to_string(),
            port: 22,
            key_type: "Ed25519".to_string(),
            fingerprint: "SHA256:xxx".to_string(),
            expected: String::new(),
            public_key_blob: "ssh-ed25519 AAAA...".to_string(),
        });
        assert_eq!(
            reg.request_info(id).unwrap().public_key_blob,
            "ssh-ed25519 AAAA..."
        );
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
