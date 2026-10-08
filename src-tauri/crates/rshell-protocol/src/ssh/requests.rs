//! Per-PTY request lifecycle. One budget covers admission and execution.
use crate::ProtocolError;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

pub(super) const REQUEST_BUDGET: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Active,
    Recovery,
    Closed,
}
pub(super) enum Operation {
    Send(Vec<u8>),
    Resize(u32, u32),
}
pub(super) struct Request {
    operation: Operation,
    deadline: Instant,
    reply: oneshot::Sender<Result<(), ProtocolError>>,
}
#[derive(Clone)]
pub(super) struct TerminalHandle {
    pub channel: u32,
    sender: mpsc::Sender<Request>,
    state: watch::Sender<State>,
    budget: Duration,
}
impl TerminalHandle {
    pub fn new(channel: u32, capacity: usize, budget: Duration) -> (Self, mpsc::Receiver<Request>) {
        let (sender, requests) = mpsc::channel(capacity);
        let (state, _) = watch::channel(State::Active);
        (
            Self {
                channel,
                sender,
                state,
                budget,
            },
            requests,
        )
    }
    pub fn close(&self) {
        self.state.send_replace(State::Closed);
    }
    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }
    pub async fn request(&self, operation: Operation) -> Result<(), ProtocolError> {
        if *self.state.borrow() != State::Active {
            return Err(recovery());
        }
        let deadline = Instant::now() + self.budget;
        let (reply, received) = oneshot::channel();
        // If the caller disappears after admission (including an outer core
        // deadline), isolate this old PTY; no queued bytes can escape later.
        let mut guard = CancelOnDrop(Some(self.state.clone()));
        let mut state = self.subscribe();
        let result = tokio::select! {
            biased;
            _ = state.changed() => Err(recovery()),
            result = tokio::time::timeout_at(deadline, async {
                self.sender.send(Request { operation, deadline, reply }).await
                    .map_err(|_| recovery())?;
                received.await.map_err(|_| recovery())?
            }) => result.unwrap_or_else(|_| Err(recovery())),
        };
        if result.is_err() {
            self.state.send_if_modified(|state| {
                if *state == State::Active {
                    *state = State::Recovery;
                    true
                } else {
                    false
                }
            });
        }
        guard.0 = None;
        result
    }
}
struct CancelOnDrop(Option<watch::Sender<State>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            state.send_if_modified(|state| {
                if *state == State::Active {
                    *state = State::Recovery;
                    true
                } else {
                    false
                }
            });
        }
    }
}
fn recovery() -> ProtocolError {
    ProtocolError::TerminalRecoveryRequired
}

#[async_trait::async_trait]
pub(super) trait Writer: Send {
    async fn execute(&mut self, operation: Operation) -> Result<(), ProtocolError>;
    async fn close(&mut self);
}
#[async_trait::async_trait]
impl Writer for russh::ChannelWriteHalf<russh::client::Msg> {
    async fn execute(&mut self, operation: Operation) -> Result<(), ProtocolError> {
        match operation {
            Operation::Send(data) => self.data(std::io::Cursor::new(data)).await,
            Operation::Resize(cols, rows) => self.window_change(cols, rows, 0, 0).await,
        }
        .map_err(|_| recovery())
    }
    async fn close(&mut self) {
        let _ = russh::ChannelWriteHalf::close(self).await;
    }
}

pub(super) async fn run_writer<W: Writer>(
    mut writer: W,
    mut requests: mpsc::Receiver<Request>,
    handle: TerminalHandle,
) {
    let mut state = handle.subscribe();
    while *state.borrow() == State::Active {
        let request = tokio::select! {
            biased;
            _ = state.changed() => break,
            request = requests.recv() => match request { Some(request) => request, None => break },
        };
        // Caller timeout/cancellation must not become delayed execution.
        if request.reply.is_closed() || Instant::now() >= request.deadline {
            continue;
        }

        let result = tokio::select! {
            biased;
            _ = state.changed() => Err(recovery()),
            result = tokio::time::timeout_at(request.deadline, writer.execute(request.operation)) =>
                result.unwrap_or_else(|_| Err(recovery())),
        };
        let failed = result.is_err();
        let _ = request.reply.send(result);
        if failed {
            handle.state.send_if_modified(|state| {
                if *state == State::Active {
                    *state = State::Recovery;
                    true
                } else {
                    false
                }
            });
            break;
        }
    }
    // Drop all admitted requests before attempting the best-effort wire close.
    drop(requests);
    let _ = tokio::time::timeout(Duration::from_secs(5), writer.close()).await;
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub async fn saturated() -> (
        TerminalHandle,
        mpsc::Receiver<Request>,
        oneshot::Receiver<Result<(), ProtocolError>>,
    ) {
        let (handle, receiver) = TerminalHandle::new(0, 1, REQUEST_BUDGET);
        let (reply, received) = oneshot::channel();
        handle
            .sender
            .send(Request {
                operation: Operation::Send(vec![1]),
                deadline: Instant::now() + REQUEST_BUDGET,
                reply,
            })
            .await
            .unwrap();
        (handle, receiver, received)
    }
    struct ControlledWriter {
        sent: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
        block: bool,
        closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl Writer for ControlledWriter {
        async fn execute(&mut self, operation: Operation) -> Result<(), ProtocolError> {
            if let Operation::Send(data) = operation {
                self.sent.lock().unwrap().push(data);
            }
            if self.block {
                std::future::pending().await
            } else {
                Ok(())
            }
        }
        async fn close(&mut self) {
            self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
    }
    fn writer(block: bool) -> ControlledWriter {
        ControlledWriter {
            sent: Default::default(),
            block,
            closed: Default::default(),
        }
    }
    #[tokio::test(start_paused = true)]
    async fn recovery_expired_and_abandoned_queued_input_never_executes() {
        for abandoned in [false, true] {
            let (handle, requests) = TerminalHandle::new(0, 2, Duration::from_millis(20));
            let (reply, received) = oneshot::channel();
            let received = if abandoned {
                drop(received);
                None
            } else {
                Some(received)
            };
            handle
                .sender
                .send(Request {
                    operation: Operation::Send(b"old".to_vec()),
                    deadline: if abandoned {
                        Instant::now() + Duration::from_secs(1)
                    } else {
                        Instant::now() - Duration::from_millis(1)
                    },
                    reply,
                })
                .await
                .unwrap();
            let writer = writer(false);
            let sent = writer.sent.clone();
            let task = tokio::spawn(run_writer(writer, requests, handle.clone()));
            tokio::task::yield_now().await;
            assert!(
                sent.lock().unwrap().is_empty(),
                "expired/cancelled input was replayed by old actor"
            );
            handle.close();
            task.await.unwrap();
            drop(received);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn recovery_stalled_execution_isolated_and_independent_terminal_still_writes() {
        let (stalled, requests) = TerminalHandle::new(0, 2, Duration::from_millis(20));
        let writer1 = writer(true);
        let old_sent = writer1.sent.clone();
        let closed = writer1.closed.clone();
        let task = tokio::spawn(run_writer(writer1, requests, stalled.clone()));
        let first = stalled.clone();
        let first =
            tokio::spawn(async move { first.request(Operation::Send(b"partial".to_vec())).await });
        tokio::task::yield_now().await;
        let queued = stalled.clone();
        let queued =
            tokio::spawn(
                async move { queued.request(Operation::Send(b"no replay".to_vec())).await },
            );
        assert!(matches!(
            first.await.unwrap(),
            Err(ProtocolError::TerminalRecoveryRequired)
        ));
        assert!(matches!(
            queued.await.unwrap(),
            Err(ProtocolError::TerminalRecoveryRequired)
        ));
        assert_eq!(old_sent.lock().unwrap().as_slice(), [b"partial".to_vec()]);
        let (other, requests) = TerminalHandle::new(1, 2, Duration::from_millis(20));
        let writer2 = writer(false);
        let other_sent = writer2.sent.clone();
        let task2 = tokio::spawn(run_writer(writer2, requests, other.clone()));
        other
            .request(Operation::Send(b"healthy".to_vec()))
            .await
            .unwrap();
        assert_eq!(other_sent.lock().unwrap().as_slice(), [b"healthy".to_vec()]);
        stalled.close();
        other.close();
        task.await.unwrap();
        task2.await.unwrap();
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    }
}
