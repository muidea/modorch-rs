use super::*;
use crate::resource_metrics::{ResourceGuard, ResourceStats};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::{sync::atomic::Ordering, time::Instant};
use tokio::sync::{OwnedSemaphorePermit, oneshot};

pub type BlockingJob = Box<dyn FnOnce() + Send + 'static>;
/// Host adapter for a bounded execution resource. Enqueue promptly; do not run
/// the job inline. Accepted jobs must eventually run or be dropped on shutdown.
pub trait BlockingExecutor: Send + Sync {
    fn execute(&self, job: BlockingJob) -> FrameworkResult<()>;
}

/// CPU and synchronous I/O use independent pools and admission slots. Pools are
/// lazy, local to this scheduler, and never install a Tokio runtime.
#[derive(Clone)]
pub struct ExecutionResources {
    pub cpu_threads: usize,
    pub cpu_capacity: usize,
    pub blocking_io_threads: usize,
    pub blocking_io_capacity: usize,
    pub max_tasks_per_owner: usize,
    /// Maximum simultaneous admission waiters across CPU and blocking I/O.
    pub max_waiters: usize,
    /// Shared by all scheduler handles and execution classes for an Owner.
    pub max_waiters_per_owner: usize,
    pub cpu_executor: Option<Arc<dyn BlockingExecutor>>,
    pub blocking_io_executor: Option<Arc<dyn BlockingExecutor>>,
}
impl Default for ExecutionResources {
    fn default() -> Self {
        let cpu = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .clamp(1, 64);
        Self {
            cpu_threads: cpu,
            cpu_capacity: cpu * 2,
            blocking_io_threads: 8,
            blocking_io_capacity: 32,
            max_tasks_per_owner: 256,
            max_waiters: 1024,
            max_waiters_per_owner: 128,
            cpu_executor: None,
            blocking_io_executor: None,
        }
    }
}
impl ExecutionResources {
    pub fn validate(&self) -> FrameworkResult<()> {
        if self.cpu_threads == 0
            || self.blocking_io_threads == 0
            || self.cpu_threads > 4096
            || self.blocking_io_threads > 4096
            || self.cpu_capacity < self.cpu_threads
            || self.blocking_io_capacity < self.blocking_io_threads
            || [
                self.cpu_capacity,
                self.blocking_io_capacity,
                self.max_tasks_per_owner,
                self.max_waiters,
                self.max_waiters_per_owner,
            ]
            .into_iter()
            .any(|n| n == 0 || n > Semaphore::MAX_PERMITS)
        {
            return Err(FrameworkError::InvalidConfig(
                "invalid execution resource bounds".into(),
            ));
        }
        Ok(())
    }
}
struct ThreadPool {
    name: &'static str,
    threads: usize,
    capacity: usize,
    sender: Mutex<Option<SyncSender<BlockingJob>>>,
}
impl BlockingExecutor for ThreadPool {
    fn execute(&self, job: BlockingJob) -> FrameworkResult<()> {
        let mut sender = self.sender.lock().unwrap();
        if sender.is_none() {
            let (tx, rx) = sync_channel::<BlockingJob>(self.capacity);
            let receiver = Arc::new(Mutex::new(rx));
            for index in 0..self.threads {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("modorch-{}-{index}", self.name))
                    .spawn(move || {
                        loop {
                            let job = { receiver.lock().unwrap().recv() };
                            let Ok(job) = job else {
                                break;
                            };
                            // Jobs supplied by the scheduler catch user panics. Also keep
                            // an adapter panic from removing a pool worker permanently.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                        }
                    })
                    .map_err(|error| {
                        FrameworkError::InvalidConfig(format!("execution pool startup: {error}"))
                    })?;
            }
            *sender = Some(tx);
        }
        sender
            .as_ref()
            .unwrap()
            .try_send(job)
            .map_err(|_| FrameworkError::ResourceLimit("blocking executor queue".into()))
    }
}
pub(super) struct ExecutionClass {
    pub executor: Arc<dyn BlockingExecutor>,
    pub slots: Arc<Semaphore>,
    pub capacity: usize,
    pub stats: Arc<ResourceStats>,
    pub class: TaskClass,
}
impl ExecutionClass {
    pub fn new(
        name: &'static str,
        threads: usize,
        capacity: usize,
        executor: Option<Arc<dyn BlockingExecutor>>,
    ) -> Self {
        Self {
            executor: executor.unwrap_or_else(|| {
                Arc::new(ThreadPool {
                    name,
                    threads,
                    capacity,
                    sender: Mutex::new(None),
                })
            }),
            slots: Arc::new(Semaphore::new(capacity)),
            capacity,
            stats: Arc::default(),
            class: if name == "cpu" {
                TaskClass::Cpu
            } else {
                TaskClass::BlockingIo
            },
        }
    }
}
/// Owns only the result receiver. Dropping it does not stop or uncount the job.
pub struct BlockingTaskHandle<T> {
    task: TaskHandle,
    value: oneshot::Receiver<T>,
}
impl<T> BlockingTaskHandle<T> {
    pub fn operation_id(&self) -> OperationId {
        self.task.operation_id()
    }
    pub fn state(&self) -> TaskState {
        self.task.state()
    }
    pub fn cancel(&self) {
        self.task.cancel();
    }
    pub fn task(&self) -> &TaskHandle {
        &self.task
    }
    pub async fn wait(self) -> FrameworkResult<T> {
        self.task.wait().await?;
        self.value.await.map_err(|_| FrameworkError::Cancelled)
    }
    /// Return the result after task-owned captures, leases and quotas are released.
    /// Dropping this future does not cancel the synchronous work.
    pub async fn wait_finished(self) -> FrameworkResult<T> {
        self.task.wait_finished().await?;
        self.value.await.map_err(|_| FrameworkError::Cancelled)
    }
}
impl TaskScheduler {
    pub fn spawn_cpu<F, T>(&self, task: F) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_sync(&self.runtime.0.cpu, task)
    }
    pub fn spawn_blocking_io<F, T>(&self, task: F) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_sync(&self.runtime.0.blocking_io, task)
    }
    fn spawn_sync<F, T>(
        &self,
        class: &ExecutionClass,
        task: F,
    ) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let permits = self.reserve_slots(Some(class))?;
        self.spawn_sync_reserved(class, task, permits)
    }
    fn spawn_sync_reserved<F, T>(
        &self,
        class: &ExecutionClass,
        task: F,
        permits: TaskPermits,
    ) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let rt = &self.runtime.0;
        let lease = rt
            .tracker
            .admit_bound(self.owner.clone(), self.generation)?;
        let id = lease.id();
        let cancel = CancellationToken::new();
        let (state, _) = watch::channel(TaskState::Accepted);
        let (result_tx, result) = watch::channel(None);
        let (finished_tx, finished) = watch::channel(false);
        let guard = TaskGuard {
            runtime: Arc::downgrade(rt),
            id,
            state: state.clone(),
            result: result_tx.clone(),
            finished: finished_tx,
            lease: Some(lease),
            permits: Some(permits),
            resource: Some(ResourceGuard::new(class.stats.clone())),
        };
        let owned = OwnedTask {
            work: Some(task),
            guard,
        };
        let handle = TaskHandle {
            id,
            cancel: cancel.clone(),
            state: state.clone(),
            result,
            finished,
        };
        rt.tasks.lock().unwrap().insert(
            id,
            TaskControl {
                class: class.class,
                phase: TaskPhase::Queued,
                cancel: cancel.clone(),
                state: state.clone(),
                abort: None,
                owner: self.owner.clone(),
            },
        );
        if rt.tracker.is_closed() {
            request_cancel(&cancel, &state);
        }
        let runtime = self.runtime.clone();
        let owner = self.owner.clone();
        let generation = self.generation;
        let (value_tx, value) = oneshot::channel();
        let parent = rt.tracker.current().map(|p| p.id());
        let span = tracing::info_span!("blocking_task", application_id=rt.tracker.application_id(), operation_id=id, component=%owner, causation_id=?parent);
        let submitted = class.executor.execute(Box::new(move || {
            let mut owned = owned;
            let _entered = span.enter();
            let outcome =
                runtime
                    .0
                    .tracker
                    .scope_sync(owned.guard.lease.as_ref().unwrap().clone(), || {
                        if cancel.is_cancelled() {
                            return Ok(Err(FrameworkError::Cancelled));
                        }
                        owned.guard.resource.as_mut().unwrap().start();
                        owned.guard.phase(TaskPhase::Running);
                        state.send_if_modified(|s| {
                            if *s == TaskState::Accepted {
                                *s = TaskState::Running;
                                true
                            } else {
                                false
                            }
                        });
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            (owned.work.take().expect("unstarted task"))(cancel)
                        }))
                    });
            let result = match outcome {
                Ok(Ok(value)) => {
                    let _ = value_tx.send(value);
                    Ok(())
                }
                Ok(Err(FrameworkError::Cancelled)) => Err(FrameworkError::Cancelled),
                Ok(Err(error)) => Err(error.in_execution(owner.clone(), Some(id), Phase::Task)),
                Err(_) => Err(FrameworkError::ExecutionFailed {
                    component: owner.clone(),
                    operation_id: Some(id),
                    phase: Phase::Task,
                    detail: "blocking task panicked".into(),
                    source: None,
                }),
            };
            if let Err(error) = &result {
                if !matches!(error, FrameworkError::Cancelled) {
                    runtime
                        .0
                        .failures
                        .report_for(owner, generation, error.clone());
                }
            }
            owned.guard.resource.as_mut().unwrap().success = result.is_ok();
            owned.guard.phase(TaskPhase::Finishing);
            state.send_replace(match &result {
                Ok(()) => TaskState::Completed,
                Err(FrameworkError::Cancelled) => TaskState::Cancelled,
                Err(_) => TaskState::Failed,
            });
            result_tx.send_replace(Some(result));
        }));
        if submitted.is_err() {
            class
                .stats
                .executor_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        submitted?;
        Ok(BlockingTaskHandle {
            task: handle,
            value,
        })
    }
    pub(super) fn reserve_slots(
        &self,
        class: Option<&ExecutionClass>,
    ) -> FrameworkResult<TaskPermits> {
        let rt = &self.runtime.0;
        // Serialize tentative multi-quota reservations. A partial rollback must
        // never be observable by another admission waiter as durable exhaustion.
        let _reservation = rt.reservation.lock().unwrap();
        let global = rt.task_slots.clone().try_acquire_owned().map_err(|_| {
            if let Some(c) = class {
                c.stats.rejected_global.fetch_add(1, Ordering::Relaxed);
            }
            FrameworkError::Backpressure(self.owner.clone())
        })?;
        let owner = self.slots.clone().try_acquire_owned().map_err(|_| {
            if let Some(c) = class {
                c.stats.rejected_owner.fetch_add(1, Ordering::Relaxed);
            }
            FrameworkError::Backpressure(self.owner.clone())
        })?;
        let resource = match class {
            Some(c) => Some(c.slots.clone().try_acquire_owned().map_err(|_| {
                c.stats.rejected_capacity.fetch_add(1, Ordering::Relaxed);
                FrameworkError::Backpressure(self.owner.clone())
            })?),
            None => None,
        };
        Ok(TaskPermits {
            global: Some(global),
            owner: Some(owner),
            resource,
            changed: rt.capacity_changed.clone(),
        })
    }
    /// Wait for global, Owner and CPU capacity before submitting exactly once.
    /// Waiting is bounded by both waiter quotas and `timeout`, and does not hold
    /// a task slot or operation lease. Dropping the future before admission drops
    /// the closure without executing it. Admission order is not FIFO.
    pub async fn spawn_cpu_wait<F, T>(
        &self,
        task: F,
        timeout: Duration,
    ) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_sync_wait(&self.runtime.0.cpu, task, timeout)
            .await
    }
    /// Like [`Self::spawn_cpu_wait`], using the independent blocking I/O resource.
    pub async fn spawn_blocking_io_wait<F, T>(
        &self,
        task: F,
        timeout: Duration,
    ) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_sync_wait(&self.runtime.0.blocking_io, task, timeout)
            .await
    }
    async fn spawn_sync_wait<F, T>(
        &self,
        class: &ExecutionClass,
        task: F,
        timeout: Duration,
    ) -> FrameworkResult<BlockingTaskHandle<T>>
    where
        F: FnOnce(CancellationToken) -> FrameworkResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| {
                FrameworkError::InvalidConfig("execution admission timeout overflow".into())
            })?;
        let mut waiting = None;
        loop {
            let changed = self.runtime.0.capacity_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let gate = self.runtime.0.tracker.admission_notification();
            tokio::pin!(gate);
            gate.as_mut().enable();
            self.runtime
                .0
                .tracker
                .check_bound(&self.owner, self.generation)?;
            if tokio::time::Instant::now() >= deadline {
                class
                    .stats
                    .admission_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                return Err(FrameworkError::AdmissionTimeout);
            }
            match self.reserve_slots(Some(class)) {
                Ok(permits) => {
                    // Recheck after reservation-lock contention, before registration.
                    if tokio::time::Instant::now() >= deadline {
                        class
                            .stats
                            .admission_timeouts
                            .fetch_add(1, Ordering::Relaxed);
                        return Err(FrameworkError::AdmissionTimeout);
                    }
                    return self.spawn_sync_reserved(class, task, permits);
                }
                Err(FrameworkError::Backpressure(_)) => {}
                Err(error) => return Err(error),
            }
            if waiting.is_none() {
                let global = self.runtime.0.wait_slots.clone().try_acquire_owned();
                let owner = self.wait_slots.clone().try_acquire_owned();
                match (global, owner) {
                    (Ok(global), Ok(owner)) => {
                        class.stats.waiters.fetch_add(1, Ordering::Relaxed);
                        waiting = Some(ResourceWaitGuard {
                            _global: global,
                            _owner: owner,
                            stats: class.stats.clone(),
                            started: Instant::now(),
                        });
                    }
                    _ => {
                        class.stats.rejected_waiters.fetch_add(1, Ordering::Relaxed);
                        return Err(FrameworkError::ResourceLimit(
                            "execution admission waiters".into(),
                        ));
                    }
                }
            }
            tokio::select! { biased;
                _ = tokio::time::sleep_until(deadline) => {
                    class.stats.admission_timeouts.fetch_add(1, Ordering::Relaxed);
                    return Err(FrameworkError::AdmissionTimeout);
                }
                _ = &mut gate => {},
                _ = &mut changed => {},
            }
        }
    }
}

pub(super) struct TaskPermits {
    global: Option<OwnedSemaphorePermit>,
    owner: Option<OwnedSemaphorePermit>,
    resource: Option<OwnedSemaphorePermit>,
    changed: Arc<tokio::sync::Notify>,
}
impl Drop for TaskPermits {
    fn drop(&mut self) {
        drop(self.resource.take());
        drop(self.owner.take());
        drop(self.global.take());
        self.changed.notify_waiters();
    }
}
struct ResourceWaitGuard {
    _global: OwnedSemaphorePermit,
    _owner: OwnedSemaphorePermit,
    stats: Arc<ResourceStats>,
    started: Instant,
}
impl Drop for ResourceWaitGuard {
    fn drop(&mut self) {
        self.stats.waiters.fetch_sub(1, Ordering::Relaxed);
        self.stats.admission.record(self.started);
    }
}
