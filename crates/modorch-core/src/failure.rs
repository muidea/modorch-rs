use crate::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, watch};

#[derive(Clone, Debug)]
pub struct FailureEvent {
    pub component: Option<ComponentRef>,
    pub generation: u64,
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
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            queue: Mutex::new(VecDeque::new()),
            changed: Notify::new(),
            current: watch::channel(None).0,
        }))
    }
    pub fn report(&self, error: FrameworkError) {
        self.enqueue(FailureEvent {
            component: None,
            generation: 0,
            error,
        });
    }
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
    pub fn subscribe(&self) -> watch::Receiver<Option<FrameworkError>> {
        self.0.current.subscribe()
    }
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
    pub fn acknowledge(&self) {
        let queue = self.0.queue.lock().unwrap();
        self.0
            .current
            .send_replace(queue.front().map(|e| e.error.clone()));
    }
}
