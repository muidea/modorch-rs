use crate::*;
use std::time::Instant;

/// Wall-clock drain accounting, including dependency-closure recovery drains.
#[derive(Clone, Debug, Default)]
pub struct DrainMetrics {
    pub started: u64,
    pub completed: u64,
    pub interrupted: u64,
    pub active: usize,
    pub active_elapsed: Duration,
    pub completed_elapsed: Duration,
    pub interrupted_elapsed: Duration,
    pub last_completed: Option<Duration>,
}
#[derive(Default)]
struct DrainState {
    snapshot: DrainMetrics,
    next: u64,
    active: BTreeMap<u64, Instant>,
}
#[derive(Default, Clone)]
pub(crate) struct DrainTracker(Arc<Mutex<DrainState>>);
impl DrainTracker {
    pub fn begin(&self) -> DrainGuard {
        let mut state = self.0.lock().unwrap();
        let id = state.next;
        state.next += 1;
        let started = Instant::now();
        state.active.insert(id, started);
        state.snapshot.started += 1;
        DrainGuard {
            tracker: self.clone(),
            id,
            started,
            complete: false,
        }
    }
    pub fn snapshot(&self) -> DrainMetrics {
        let state = self.0.lock().unwrap();
        let mut snapshot = state.snapshot.clone();
        snapshot.active = state.active.len();
        snapshot.active_elapsed = state.active.values().map(Instant::elapsed).sum();
        snapshot
    }
}
pub(crate) struct DrainGuard {
    tracker: DrainTracker,
    id: u64,
    started: Instant,
    complete: bool,
}
impl DrainGuard {
    pub fn complete(mut self) {
        self.complete = true;
    }
}
impl Drop for DrainGuard {
    fn drop(&mut self) {
        let mut state = self.tracker.0.lock().unwrap();
        state.active.remove(&self.id);
        let elapsed = self.started.elapsed();
        if self.complete {
            state.snapshot.completed += 1;
            state.snapshot.completed_elapsed += elapsed;
            state.snapshot.last_completed = Some(elapsed);
        } else {
            state.snapshot.interrupted += 1;
            state.snapshot.interrupted_elapsed += elapsed;
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeMetrics {
    pub application_id: ApplicationId,
    pub operations: PendingWork,
    pub hub: HubMetrics,
    pub messages: Vec<MessageMetrics>,
    pub mailboxes: Vec<MailboxMetrics>,
    pub tasks: TaskMetrics,
    pub drain: DrainMetrics,
    pub journal_capacity: usize,
    pub journal_dropped: u64,
}
impl ApplicationHandle {
    /// Each subsystem is sampled independently; collection does not stop admission.
    pub fn metrics(&self) -> RuntimeMetrics {
        RuntimeMetrics {
            application_id: self.id(),
            operations: self.inner.tracker.pending(),
            hub: self.inner.hub.metrics(),
            messages: self.inner.hub.message_metrics(),
            mailboxes: self.inner.hub.mailbox_metrics(),
            tasks: self.inner.scheduler.metrics(),
            drain: self.inner.drain_metrics.snapshot(),
            journal_capacity: self.inner.journal_capacity,
            journal_dropped: self
                .inner
                .journal_dropped
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}
