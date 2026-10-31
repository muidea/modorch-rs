//! Owner-bound supervised tasks and bounded timer dispatch.
#![forbid(unsafe_code)]
mod blocking;
mod resource_metrics;
pub use blocking::{BlockingExecutor, BlockingJob, BlockingTaskHandle, ExecutionResources};
use blocking::{ExecutionClass, TaskPermits};
use futures_util::FutureExt;
use modorch_core::*;
pub use resource_metrics::{ExecutionResourceMetrics, OwnerTaskMetrics, TaskClass};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{Semaphore, watch};
pub use tokio_util::sync::CancellationToken;
use tracing::Instrument;

pub type TaskFuture = Pin<Box<dyn Future<Output = FrameworkResult<()>> + Send>>;
pub type TaskFactory = Arc<dyn Fn(CancellationToken) -> TaskFuture + Send + Sync>;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Accepted,
    Running,
    Completed,
    Failed,
    CancelRequested,
    Cancelled,
}
impl TaskState {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone)]
pub struct TaskHandle {
    id: OperationId,
    cancel: CancellationToken,
    state: watch::Sender<TaskState>,
    result: watch::Receiver<Option<FrameworkResult<()>>>,
    finished: watch::Receiver<bool>,
}
fn request_cancel(cancel: &CancellationToken, state: &watch::Sender<TaskState>) {
    cancel.cancel();
    state.send_if_modified(|s| {
        if !s.terminal() {
            *s = TaskState::CancelRequested;
            true
        } else {
            false
        }
    });
}
impl TaskHandle {
    pub fn operation_id(&self) -> OperationId {
        self.id
    }
    pub fn state(&self) -> TaskState {
        *self.state.borrow()
    }
    /// Cooperative only: the operation remains counted until the future returns.
    pub fn cancel(&self) {
        request_cancel(&self.cancel, &self.state);
    }
    pub async fn wait(&self) -> FrameworkResult<()> {
        let mut result = self.result.clone();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result;
            }
            result.changed().await.map_err(|_| FrameworkError::Closed)?;
        }
    }
    /// Wait until the job's leases, permits and control record have been released.
    pub async fn wait_finished(&self) -> FrameworkResult<()> {
        let mut finished = self.finished.clone();
        while !*finished.borrow() {
            finished
                .changed()
                .await
                .map_err(|_| FrameworkError::Closed)?;
        }
        self.wait().await
    }
}
#[derive(Clone, Copy, Debug)]
pub enum TickPolicy {
    SkipIfRunning,
    QueueBounded { capacity: usize },
    AllowOverlap { max_in_flight: usize },
}
#[derive(Clone, Copy, Debug)]
pub struct IntervalSpec {
    pub period: Duration,
    pub policy: TickPolicy,
}
#[derive(Clone, Copy)]
enum TaskPhase {
    Queued,
    Running,
    Finishing,
}
struct TaskControl {
    class: TaskClass,
    phase: TaskPhase,
    cancel: CancellationToken,
    state: watch::Sender<TaskState>,
    abort: Option<tokio::task::AbortHandle>,
    owner: ComponentRef,
}
struct TimerControl {
    owner: ComponentRef,
    cancel: CancellationToken,
    abort: tokio::task::AbortHandle,
}
struct Inner {
    tracker: OperationTracker,
    failures: FailureReporter,
    tasks: Mutex<BTreeMap<OperationId, TaskControl>>,
    timers: Mutex<BTreeMap<OperationId, TimerControl>>,
    cpu: ExecutionClass,
    blocking_io: ExecutionClass,
    per_owner_limit: usize,
    owner_slots: Mutex<BTreeMap<ComponentRef, Weak<Semaphore>>>,
    task_slots: Arc<Semaphore>,
    timer_slots: Arc<Semaphore>,
    reservation: Mutex<()>,
    capacity_changed: Arc<tokio::sync::Notify>,
    wait_slots: Arc<Semaphore>,
    owner_wait_slots: Mutex<BTreeMap<ComponentRef, Weak<Semaphore>>>,
    per_owner_wait_limit: usize,
    max_tasks: usize,
}
#[derive(Clone)]
pub struct SchedulerRuntime(Arc<Inner>);
#[derive(Clone)]
pub struct TaskScheduler {
    runtime: SchedulerRuntime,
    owner: ComponentRef,
    generation: u64,
    slots: Arc<Semaphore>,
    wait_slots: Arc<Semaphore>,
}
pub type TaskGroup = TaskScheduler;
struct TaskGuard {
    runtime: Weak<Inner>,
    id: OperationId,
    state: watch::Sender<TaskState>,
    result: watch::Sender<Option<FrameworkResult<()>>>,
    finished: watch::Sender<bool>,
    lease: Option<OperationLease>,
    permits: Option<TaskPermits>,
    resource: Option<resource_metrics::ResourceGuard>,
}
impl TaskGuard {
    fn phase(&self, phase: TaskPhase) {
        if let Some(runtime) = self.runtime.upgrade() {
            if let Some(control) = runtime.tasks.lock().unwrap().get_mut(&self.id) {
                control.phase = phase;
            }
        }
    }
}
impl Drop for TaskGuard {
    fn drop(&mut self) {
        if !self.state.borrow().terminal() {
            self.state.send_replace(TaskState::Cancelled);
            self.result
                .send_replace(Some(Err(FrameworkError::Cancelled)));
        }
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.tasks.lock().unwrap().remove(&self.id);
        }
        drop(self.resource.take());
        drop(self.permits.take());
        drop(self.lease.take());
        self.finished.send_replace(true);
    }
}
// Keep unstarted user captures in the same ownership unit as the lifetime
// guard, including executor rejection and abort-before-first-poll paths.
struct OwnedTask<F> {
    work: Option<F>,
    guard: TaskGuard,
}
impl<F> Drop for OwnedTask<F> {
    fn drop(&mut self) {
        drop(self.work.take());
    }
}
#[derive(Clone)]
pub struct TimerHandle {
    cancel: CancellationToken,
    done: watch::Receiver<bool>,
}
impl TimerHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Acknowledges driver exit. Already dispatched tasks have separate handles/accounting.
    pub async fn wait(&self) {
        let mut done = self.done.clone();
        while !*done.borrow() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}
struct TimerGuard {
    done: watch::Sender<bool>,
    runtime: Weak<Inner>,
    id: OperationId,
}
impl Drop for TimerGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.timers.lock().unwrap().remove(&self.id);
        }
        self.done.send_replace(true);
    }
}
impl SchedulerRuntime {
    pub fn new(
        tracker: OperationTracker,
        failures: FailureReporter,
        max_tasks: usize,
        max_timers: usize,
    ) -> FrameworkResult<Self> {
        Self::with_resources(
            tracker,
            failures,
            max_tasks,
            max_timers,
            ExecutionResources::default(),
        )
    }
    pub fn with_resources(
        tracker: OperationTracker,
        failures: FailureReporter,
        max_tasks: usize,
        max_timers: usize,
        resources: ExecutionResources,
    ) -> FrameworkResult<Self> {
        resources.validate()?;
        if max_tasks == 0
            || max_timers == 0
            || max_tasks > Semaphore::MAX_PERMITS
            || max_timers > Semaphore::MAX_PERMITS
        {
            return Err(FrameworkError::InvalidConfig(
                "invalid task/timer limits".into(),
            ));
        }
        Ok(Self(Arc::new(Inner {
            tracker,
            failures,
            tasks: Mutex::new(BTreeMap::new()),
            timers: Mutex::new(BTreeMap::new()),
            cpu: ExecutionClass::new(
                "cpu",
                resources.cpu_threads,
                resources.cpu_capacity,
                resources.cpu_executor,
            ),
            blocking_io: ExecutionClass::new(
                "io",
                resources.blocking_io_threads,
                resources.blocking_io_capacity,
                resources.blocking_io_executor,
            ),
            per_owner_limit: resources.max_tasks_per_owner,
            owner_slots: Mutex::new(BTreeMap::new()),
            task_slots: Arc::new(Semaphore::new(max_tasks)),
            timer_slots: Arc::new(Semaphore::new(max_timers)),
            reservation: Mutex::new(()),
            capacity_changed: Arc::new(tokio::sync::Notify::new()),
            wait_slots: Arc::new(Semaphore::new(resources.max_waiters)),
            owner_wait_slots: Mutex::new(BTreeMap::new()),
            per_owner_wait_limit: resources.max_waiters_per_owner,
            max_tasks,
        })))
    }
    pub fn for_owner(&self, owner: ComponentRef) -> TaskScheduler {
        let slots = {
            let mut owners = self.0.owner_slots.lock().unwrap();
            owners.retain(|_, slots| slots.strong_count() != 0);
            match owners.get(&owner).and_then(Weak::upgrade) {
                Some(slots) => slots,
                None => {
                    let slots = Arc::new(Semaphore::new(self.0.per_owner_limit));
                    owners.insert(owner.clone(), Arc::downgrade(&slots));
                    slots
                }
            }
        };
        let wait_slots = {
            let mut owners = self.0.owner_wait_slots.lock().unwrap();
            owners.retain(|_, slots| slots.strong_count() != 0);
            owners
                .get(&owner)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let slots = Arc::new(Semaphore::new(self.0.per_owner_wait_limit));
                    owners.insert(owner.clone(), Arc::downgrade(&slots));
                    slots
                })
        };
        TaskScheduler {
            wait_slots,
            slots,
            runtime: self.clone(),
            generation: self.0.tracker.generation(&owner),
            owner,
        }
    }
    pub fn begin_shutdown(&self) {
        for timer in self.0.timers.lock().unwrap().values() {
            timer.cancel.cancel();
        }
        for task in self.0.tasks.lock().unwrap().values() {
            request_cancel(&task.cancel, &task.state);
        }
    }
    pub fn pending_owners(&self) -> BTreeMap<ComponentRef, usize> {
        let mut result = BTreeMap::new();
        for task in self.0.tasks.lock().unwrap().values() {
            *result.entry(task.owner.clone()).or_default() += 1;
        }
        result
    }
    pub fn force_cancel(&self) {
        self.begin_shutdown();
        for timer in self.0.timers.lock().unwrap().values() {
            timer.abort.abort();
        }
        for task in self.0.tasks.lock().unwrap().values() {
            if let Some(abort) = &task.abort {
                abort.abort();
            }
        }
    }
}
impl TaskScheduler {
    pub fn owner(&self) -> &ComponentRef {
        &self.owner
    }
    pub fn spawn<F, Fut>(&self, task: F) -> FrameworkResult<TaskHandle>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = FrameworkResult<()>> + Send + 'static,
    {
        tokio::runtime::Handle::try_current().map_err(|_| {
            FrameworkError::InvalidConfig("TaskScheduler requires a host Tokio runtime".into())
        })?;
        let rt = &self.runtime.0;
        let permits = self.reserve_slots(None)?;
        let lease = rt
            .tracker
            .admit_bound(self.owner.clone(), self.generation)?;
        let id = lease.id();
        let cancel = CancellationToken::new();
        let (state_tx, _) = watch::channel(TaskState::Accepted);
        let (result_tx, result) = watch::channel(None);
        let (finished_tx, finished) = watch::channel(false);
        let guard = TaskGuard {
            runtime: Arc::downgrade(rt),
            id,
            state: state_tx.clone(),
            result: result_tx.clone(),
            finished: finished_tx,
            lease: Some(lease),
            permits: Some(permits),
            resource: None,
        };
        let owned = OwnedTask {
            work: Some(task),
            guard,
        };
        let runtime = self.runtime.clone();
        let owner = self.owner.clone();
        let generation = self.generation;
        let task_cancel = cancel.clone();
        let state = state_tx.clone();
        // Registration and completion removal are serialized, including fast futures.
        let mut controls = rt.tasks.lock().unwrap();
        let parent = rt.tracker.current().map(|lease| lease.id());
        let span = if parent.is_some() {
            tracing::info_span!("task", application_id=rt.tracker.application_id(), operation_id=id, component=%owner, causation_id=?parent)
        } else {
            tracing::info_span!(parent: None, "task", application_id=rt.tracker.application_id(), operation_id=id, component=%owner)
        };
        let join = tokio::spawn(
            async move {
                let mut owned = owned;
                let task = owned.work.take().expect("unstarted task");
                owned.guard.phase(TaskPhase::Running);
                state_tx.send_if_modified(|s| {
                    if *s == TaskState::Accepted {
                        *s = TaskState::Running;
                        true
                    } else {
                        false
                    }
                });
                let future =
                    std::panic::AssertUnwindSafe(async { task(task_cancel.clone()).await })
                        .catch_unwind();
                let outcome = runtime
                    .0
                    .tracker
                    .scope(owned.guard.lease.as_ref().unwrap().clone(), future)
                    .await;
                let result = match outcome {
                    Ok(result) => result.map_err(|error| {
                        if matches!(error, FrameworkError::Cancelled) {
                            error
                        } else {
                            error.in_execution(owner.clone(), Some(id), Phase::Task)
                        }
                    }),
                    Err(_) => Err(FrameworkError::ExecutionFailed {
                        component: owner.clone(),
                        operation_id: Some(id),
                        phase: Phase::Task,
                        detail: "background task panicked".into(),
                        source: None,
                    }),
                };
                let final_state = match &result {
                    Ok(()) => TaskState::Completed,
                    Err(FrameworkError::Cancelled) => TaskState::Cancelled,
                    Err(_) => TaskState::Failed,
                };
                if let Err(error) = &result {
                    if !matches!(error, FrameworkError::Cancelled) {
                        runtime
                            .0
                            .failures
                            .report_for(owner.clone(), generation, error.clone());
                    }
                }
                owned.guard.phase(TaskPhase::Finishing);
                state_tx.send_replace(final_state);
                result_tx.send_replace(Some(result));
            }
            .instrument(span),
        );
        controls.insert(
            id,
            TaskControl {
                class: TaskClass::Async,
                phase: TaskPhase::Queued,
                cancel: cancel.clone(),
                state: state.clone(),
                abort: Some(join.abort_handle()),
                owner: self.owner.clone(),
            },
        );
        // A task admitted immediately before shutdown must observe cancellation too.
        if rt.tracker.is_closed() {
            request_cancel(&cancel, &state);
        }
        Ok(TaskHandle {
            id,
            cancel,
            state,
            result,
            finished,
        })
    }
    pub fn delay(&self, delay: Duration, task: TaskFactory) -> FrameworkResult<TimerHandle> {
        self.timer(delay, None, TickPolicy::SkipIfRunning, task)
    }
    pub fn interval(&self, spec: IntervalSpec, task: TaskFactory) -> FrameworkResult<TimerHandle> {
        if spec.period.is_zero() {
            return Err(FrameworkError::InvalidConfig("zero timer period".into()));
        }
        self.timer(spec.period, Some(spec.period), spec.policy, task)
    }
    fn timer(
        &self,
        delay: Duration,
        period: Option<Duration>,
        policy: TickPolicy,
        task: TaskFactory,
    ) -> FrameworkResult<TimerHandle> {
        tokio::runtime::Handle::try_current().map_err(|_| {
            FrameworkError::InvalidConfig("timers require a host Tokio runtime".into())
        })?;
        let start = tokio::time::Instant::now()
            .checked_add(delay)
            .ok_or_else(|| FrameworkError::InvalidConfig("timer delay overflow".into()))?;
        let overlap = match policy {
            TickPolicy::AllowOverlap { max_in_flight }
                if max_in_flight > 0 && max_in_flight <= Semaphore::MAX_PERMITS =>
            {
                max_in_flight
            }
            TickPolicy::AllowOverlap { .. } => {
                return Err(FrameworkError::InvalidConfig(
                    "invalid timer concurrency".into(),
                ));
            }
            TickPolicy::QueueBounded { capacity: 0 } => {
                return Err(FrameworkError::InvalidConfig(
                    "zero timer queue capacity".into(),
                ));
            }
            _ => 1,
        };
        let rt = &self.runtime.0;
        let mut timers = rt.timers.lock().unwrap();
        if rt.tracker.is_closed() {
            return Err(FrameworkError::Closed);
        }
        let slot = rt
            .timer_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| FrameworkError::Backpressure(self.owner.clone()))?;
        let lease = rt
            .tracker
            .admit_bound(self.owner.clone(), self.generation)?;
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let scheduler = self.clone();
        let (done_tx, done) = watch::channel(false);
        let id = lease.id();
        let guard = TimerGuard {
            done: done_tx,
            id,
            runtime: Arc::downgrade(rt),
        };
        let parent = rt.tracker.current().map(|lease| lease.id());
        let span = if parent.is_some() {
            tracing::info_span!("timer", application_id=rt.tracker.application_id(), operation_id=lease.id(), component=%self.owner, causation_id=?parent)
        } else {
            tracing::info_span!(parent: None, "timer", application_id=rt.tracker.application_id(), operation_id=lease.id(), component=%self.owner)
        };
        let join = tokio::spawn(async move {
            let _guard = guard;
            let _slot = slot;
            let tracker = scheduler.runtime.0.tracker.clone();
            tracker.scope(lease, async {
                let mut ticks = tokio::time::interval_at(start, period.unwrap_or(Duration::from_secs(1)));
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut running: Vec<TaskHandle> = vec![]; let mut pending = 0usize;
                loop {
                    let first = running.first().cloned();
                    let tick = tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        _ = async { if let Some(handle) = first { let _ = handle.wait().await; } else { std::future::pending::<()>().await; } } => false,
                        _ = ticks.tick() => true,
                    };
                    if scheduler.runtime.0.tracker.is_closed() { break; }
                    running.retain(|h| !h.state().terminal());
                    if tick {
                        if running.len() < overlap { pending = pending.saturating_add(1); }
                        else if let TickPolicy::QueueBounded { capacity } = policy { pending = pending.saturating_add(1).min(capacity); }
                    }
                    while pending > 0 && running.len() < overlap {
                        let factory = task.clone(); pending -= 1;
                        match scheduler.spawn(move |cancel| factory(cancel)) {
                            Ok(handle) => running.push(handle),
                            Err(FrameworkError::Backpressure(_)) => break,
                            Err(_) => return,
                        }
                    }
                    if period.is_none() && tick { break; }
                }
            }).await;
        }.instrument(span));
        timers.insert(
            id,
            TimerControl {
                owner: self.owner.clone(),
                cancel: cancel.clone(),
                abort: join.abort_handle(),
            },
        );
        if rt.tracker.is_closed() {
            cancel.cancel();
        }
        Ok(TimerHandle { cancel, done })
    }
}

#[derive(Clone, Debug)]
pub struct TaskMetrics {
    pub application_id: ApplicationId,
    pub tasks: usize,
    pub timers: usize,
    pub cpu_tasks: usize,
    pub blocking_io_tasks: usize,
    pub owners: BTreeMap<ComponentRef, usize>,
    pub task_capacity: usize,
    pub available_task_slots: usize,
    pub cpu: ExecutionResourceMetrics,
    pub blocking_io: ExecutionResourceMetrics,
    pub owner_resources: BTreeMap<ComponentRef, OwnerTaskMetrics>,
}
impl SchedulerRuntime {
    pub fn metrics(&self) -> TaskMetrics {
        let owners = self.pending_owners();
        TaskMetrics {
            task_capacity: self.0.max_tasks,
            available_task_slots: self.0.task_slots.available_permits(),
            cpu: self
                .0
                .cpu
                .stats
                .snapshot(self.0.cpu.capacity, self.0.cpu.slots.available_permits()),
            blocking_io: self.0.blocking_io.stats.snapshot(
                self.0.blocking_io.capacity,
                self.0.blocking_io.slots.available_permits(),
            ),
            owner_resources: self.owner_resource_metrics(),
            application_id: self.0.tracker.application_id(),
            tasks: owners.values().sum(),
            cpu_tasks: self.0.cpu.capacity - self.0.cpu.slots.available_permits(),
            blocking_io_tasks: self.0.blocking_io.capacity
                - self.0.blocking_io.slots.available_permits(),
            timers: self.0.timers.lock().unwrap().len(),
            owners,
        }
    }
}

impl SchedulerRuntime {
    pub fn stop_owners(&self, owners: &std::collections::BTreeSet<ComponentRef>) {
        for timer in self.0.timers.lock().unwrap().values() {
            if owners.contains(&timer.owner) {
                timer.cancel.cancel();
            }
        }
        for task in self.0.tasks.lock().unwrap().values() {
            if owners.contains(&task.owner) {
                request_cancel(&task.cancel, &task.state);
            }
        }
    }
}

impl SchedulerRuntime {
    fn owner_resource_metrics(&self) -> BTreeMap<ComponentRef, OwnerTaskMetrics> {
        let mut result = BTreeMap::new();
        {
            let mut owners = self.0.owner_slots.lock().unwrap();
            owners.retain(|_, slots| slots.strong_count() != 0);
            for (owner, slots) in owners.iter() {
                if let Some(slots) = slots.upgrade() {
                    result.insert(
                        owner.clone(),
                        OwnerTaskMetrics {
                            task_capacity: self.0.per_owner_limit,
                            available_tasks: slots.available_permits(),
                            ..OwnerTaskMetrics::default()
                        },
                    );
                }
            }
        }
        {
            let mut owners = self.0.owner_wait_slots.lock().unwrap();
            owners.retain(|_, slots| slots.strong_count() != 0);
            for (owner, slots) in owners.iter() {
                if let Some(slots) = slots.upgrade() {
                    result.entry(owner.clone()).or_default().waiters =
                        self.0.per_owner_wait_limit - slots.available_permits();
                }
            }
        }
        for control in self.0.tasks.lock().unwrap().values() {
            let row = result.entry(control.owner.clone()).or_default();
            match (control.class, control.phase) {
                (TaskClass::Async, TaskPhase::Queued) => row.async_queued += 1,
                (TaskClass::Async, TaskPhase::Running) => row.async_running += 1,
                (TaskClass::Cpu, TaskPhase::Queued) => row.cpu_queued += 1,
                (TaskClass::Cpu, TaskPhase::Running) => row.cpu_running += 1,
                (TaskClass::BlockingIo, TaskPhase::Queued) => row.blocking_io_queued += 1,
                (TaskClass::BlockingIo, TaskPhase::Running) => row.blocking_io_running += 1,
                (_, TaskPhase::Finishing) => {}
            }
        }
        result
    }
}
