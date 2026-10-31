use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskClass {
    Async,
    Cpu,
    BlockingIo,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct OwnerTaskMetrics {
    pub task_capacity: usize,
    pub available_tasks: usize,
    pub waiters: usize,
    pub async_queued: usize,
    pub async_running: usize,
    pub cpu_queued: usize,
    pub cpu_running: usize,
    pub blocking_io_queued: usize,
    pub blocking_io_running: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionResourceMetrics {
    pub capacity: usize,
    pub available: usize,
    pub queued: usize,
    pub running: usize,
    pub waiters: usize,
    /// Registrations, including jobs subsequently rejected/dropped by an adapter.
    pub admitted: u64,
    /// Started jobs that exited, including failures and cooperative cancellation.
    pub completed: u64,
    /// Subset of completed jobs whose result was an error.
    pub failed: u64,
    /// Registered jobs dropped or canceled before invoking the user closure.
    pub canceled_before_start: u64,
    /// Rejection counters count attempts, including admission waiter retries.
    pub rejected_global: u64,
    pub rejected_owner: u64,
    pub rejected_capacity: u64,
    pub rejected_waiters: u64,
    pub admission_timeouts: u64,
    pub executor_failures: u64,
    /// Cumulative upper-bucket estimates in microseconds.
    pub queue_p95_micros: u64,
    pub queue_p99_micros: u64,
    pub execution_p95_micros: u64,
    pub execution_p99_micros: u64,
    pub admission_wait_p99_micros: u64,
}
#[derive(Default)]
pub(super) struct Histogram([AtomicU64; 32]);
impl Histogram {
    pub fn record(&self, start: Instant) {
        let micros = start.elapsed().as_micros().clamp(1, u64::MAX as u128) as u64;
        let bucket = (u64::BITS - (micros - 1).leading_zeros()).min(31) as usize;
        self.0[bucket].fetch_add(1, Ordering::Relaxed);
    }
    fn quantiles(&self) -> (u64, u64) {
        let bins = self.0.each_ref().map(|v| v.load(Ordering::Relaxed));
        let total: u64 = bins.iter().sum();
        let quantile = |p: u64| {
            if total == 0 {
                return 0;
            }
            let threshold = total.saturating_mul(p).div_ceil(100);
            let mut count = 0;
            for (index, bin) in bins.iter().enumerate() {
                count += bin;
                if count >= threshold {
                    return 1u64 << index;
                }
            }
            1u64 << 31
        };
        (quantile(95), quantile(99))
    }
}
#[derive(Default)]
pub(super) struct ResourceStats {
    pub queued: AtomicUsize,
    pub running: AtomicUsize,
    pub waiters: AtomicUsize,
    pub admitted: AtomicU64,
    pub completed: AtomicU64,
    pub failed: AtomicU64,
    pub canceled_before_start: AtomicU64,
    pub rejected_global: AtomicU64,
    pub rejected_owner: AtomicU64,
    pub rejected_capacity: AtomicU64,
    pub rejected_waiters: AtomicU64,
    pub admission_timeouts: AtomicU64,
    pub executor_failures: AtomicU64,
    pub queue: Histogram,
    pub execution: Histogram,
    pub admission: Histogram,
}
impl ResourceStats {
    pub fn snapshot(&self, capacity: usize, available: usize) -> ExecutionResourceMetrics {
        let (queue_p95_micros, queue_p99_micros) = self.queue.quantiles();
        let (execution_p95_micros, execution_p99_micros) = self.execution.quantiles();
        ExecutionResourceMetrics {
            capacity,
            available,
            queued: self.queued.load(Ordering::Relaxed),
            running: self.running.load(Ordering::Relaxed),
            waiters: self.waiters.load(Ordering::Relaxed),
            admitted: self.admitted.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            canceled_before_start: self.canceled_before_start.load(Ordering::Relaxed),
            rejected_global: self.rejected_global.load(Ordering::Relaxed),
            rejected_owner: self.rejected_owner.load(Ordering::Relaxed),
            rejected_capacity: self.rejected_capacity.load(Ordering::Relaxed),
            rejected_waiters: self.rejected_waiters.load(Ordering::Relaxed),
            admission_timeouts: self.admission_timeouts.load(Ordering::Relaxed),
            executor_failures: self.executor_failures.load(Ordering::Relaxed),
            queue_p95_micros,
            queue_p99_micros,
            execution_p95_micros,
            execution_p99_micros,
            admission_wait_p99_micros: self.admission.quantiles().1,
        }
    }
}
pub(super) struct ResourceGuard {
    stats: Arc<ResourceStats>,
    accepted: Instant,
    started: Option<Instant>,
    pub success: bool,
}
impl ResourceGuard {
    pub fn new(stats: Arc<ResourceStats>) -> Self {
        stats.admitted.fetch_add(1, Ordering::Relaxed);
        stats.queued.fetch_add(1, Ordering::Relaxed);
        Self {
            stats,
            accepted: Instant::now(),
            started: None,
            success: false,
        }
    }
    pub fn start(&mut self) {
        self.stats.queue.record(self.accepted);
        self.stats.queued.fetch_sub(1, Ordering::Relaxed);
        self.stats.running.fetch_add(1, Ordering::Relaxed);
        self.started = Some(Instant::now());
    }
}
impl Drop for ResourceGuard {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            self.stats.running.fetch_sub(1, Ordering::Relaxed);
            self.stats.execution.record(started);
            self.stats.completed.fetch_add(1, Ordering::Relaxed);
            if !self.success {
                self.stats.failed.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.stats.queued.fetch_sub(1, Ordering::Relaxed);
            self.stats
                .canceled_before_start
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}
