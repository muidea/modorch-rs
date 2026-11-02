use crate::*;
use std::time::Instant;

/// Wall-clock drain accounting, including dependency-closure recovery drains.
#[derive(Clone, Debug, Default)]
pub struct DrainMetrics {
    /// Cumulative drain transactions started.
    pub started: u64,
    /// Cumulative completed executions/drains; executor failures may be a subset.
    pub completed: u64,
    /// Cumulative forcibly interrupted drains, not graceful completions.
    pub interrupted: u64,
    /// Currently active executions or drains.
    pub active: usize,
    /// Sampled monotonic time elapsed in the active drain.
    pub active_elapsed: Duration,
    /// Cumulative duration of completed drains.
    pub completed_elapsed: Duration,
    /// Cumulative duration of interrupted drains.
    pub interrupted_elapsed: Duration,
    /// Most recent completed drain duration, if any.
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
/// Combined application snapshots; fields are sampled independently.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct RuntimeMetrics {
    /// Isolated application identity attached to this snapshot.
    pub application_id: ApplicationId,
    /// Tracked operation count or owner snapshot.
    pub operations: PendingWork,
    /// Message admission/execution counters.
    pub hub: HubMetrics,
    /// Bounded per-label message metric snapshots.
    pub messages: Vec<MessageMetrics>,
    /// Current mailbox and lane snapshots.
    pub mailboxes: Vec<MailboxMetrics>,
    /// Current admitted tasks or their snapshot, including jobs still releasing resources.
    pub tasks: TaskMetrics,
    /// Drain transaction counters and timings.
    pub drain: DrainMetrics,
    /// Maximum retained lifecycle records.
    pub journal_capacity: usize,
    /// Cumulative lifecycle records evicted from the bounded journal.
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
