use crate::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, watch};

#[derive(Clone, Debug)]
/// A failure associated with an optional component and its generation.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct FailureEvent {
    /// Component associated with this boundary; absent before routing when optional.
    pub component: Option<ComponentRef>,
    /// Component generation used to reject stale capabilities.
    pub generation: u64,
    /// Structured failure retaining its execution boundary and root cause.
    pub error: FrameworkError,
}
struct Inner {
    queue: Mutex<VecDeque<FailureEvent>>,
    changed: Notify,
    current: watch::Sender<Option<FrameworkError>>,
}
/// Instance-local, bounded fault mailbox. Overflow escalates to a global fault.
#[derive(Clone)]
pub struct FailureReporter(Arc<Inner>);
impl Default for FailureReporter {
    fn default() -> Self {
        Self::new()
    }
}
impl FailureReporter {
    /// Create an isolated value using the supplied application-scoped dependencies.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            queue: Mutex::new(VecDeque::new()),
            changed: Notify::new(),
            current: watch::channel(None).0,
        }))
    }
    /// Report a failure without a component identity.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn report(&self, error: FrameworkError) {
        self.enqueue(FailureEvent {
            component: None,
            generation: 0,
            error,
        });
    }
    /// Report a component-generation failure to the bounded supervision mailbox.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn report_for(&self, component: ComponentRef, generation: u64, error: FrameworkError) {
        self.enqueue(FailureEvent {
            component: Some(component),
            generation,
            error,
        });
    }
    fn enqueue(&self, event: FailureEvent) {
        tracing::error!(error=%event.error, component=?event.component, generation=event.generation, "supervised work failed");
        let mut queue = self.0.queue.lock().unwrap();
        if queue.len() >= 1024 {
            queue.clear();
            queue.push_back(FailureEvent {
                component: None,
                generation: 0,
                error: FrameworkError::InvalidConfig("supervisor fault mailbox overflow".into()),
            });
        } else {
            queue.push_back(event);
        }
        self.0
            .current
            .send_replace(queue.front().map(|e| e.error.clone()));
        self.0.changed.notify_waiters();
    }
    /// Subscribe to the latest failure notification.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn subscribe(&self) -> watch::Receiver<Option<FrameworkError>> {
        self.0.current.subscribe()
    }
    /// Read the latest unacknowledged failure, if present.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn current(&self) -> Option<FrameworkError> {
        self.0.current.borrow().clone()
    }
    /// One runtime supervisor consumes events; health retains the latest unhandled fault.
    pub async fn next(&self) -> FailureEvent {
        loop {
            let notified = self.0.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(event) = self.0.queue.lock().unwrap().pop_front() {
                return event;
            }
            notified.await;
        }
    }
    /// Acknowledge the current terminal failure notification.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn acknowledge(&self) {
        let queue = self.0.queue.lock().unwrap();
        self.0
            .current
            .send_replace(queue.front().map(|e| e.error.clone()));
    }
}
