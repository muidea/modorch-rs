use serde::Serialize;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Clone, Default, Serialize)]
pub struct Histogram {
    pub bins: [u64; 32],
    pub count: u64,
}
impl Histogram {
    pub fn record(&mut self, duration: Duration) {
        let micros = duration.as_micros().clamp(1, u64::MAX as u128) as u64;
        let bucket = (64 - (micros - 1).leading_zeros()).min(31) as usize;
        self.bins[bucket] += 1;
        self.count += 1;
    }
    pub fn merge(&mut self, other: &Self) {
        for (a, b) in self.bins.iter_mut().zip(other.bins) {
            *a += b;
        }
        self.count += other.count;
    }
    pub fn quantile(&self, percent: u64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let threshold = self.count.saturating_mul(percent).div_ceil(100);
        let mut count = 0;
        for (i, n) in self.bins.iter().enumerate() {
            count += n;
            if count >= threshold {
                return 1u64 << i;
            }
        }
        1 << 31
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"count":self.count,"p95_us":self.quantile(95),"p99_us":self.quantile(99),"bins":self.bins})
    }
}
#[derive(Default)]
pub struct Counters {
    pub live_bytes: AtomicUsize,
    pub peak_bytes: AtomicUsize,
    pub handlers: AtomicUsize,
    pub created_handlers: AtomicUsize,
    pub completed: AtomicUsize,
}
pub struct Payload {
    pub bytes: Vec<u8>,
    counters: std::sync::Arc<Counters>,
}
impl Payload {
    pub fn new(size: usize, counters: std::sync::Arc<Counters>) -> Self {
        let bytes = vec![0x5a; size];
        let live = counters.live_bytes.fetch_add(size, Ordering::Relaxed) + size;
        counters.peak_bytes.fetch_max(live, Ordering::Relaxed);
        Self { bytes, counters }
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.counters
            .live_bytes
            .fetch_sub(self.bytes.len(), Ordering::Relaxed);
    }
}
pub fn process_sample() -> serde_json::Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |key: &str| {
        status
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u64>().ok())
    };
    serde_json::json!({"rss_kib":field("VmRSS:"),"threads":field("Threads:")})
}
pub fn resource_json(m: modorch::ExecutionResourceMetrics) -> serde_json::Value {
    serde_json::json!({"capacity":m.capacity,"available":m.available,"queued":m.queued,"running":m.running,"waiters":m.waiters,
        "admitted":m.admitted,"completed":m.completed,"failed":m.failed,"canceled_before_start":m.canceled_before_start,
        "rejected_global":m.rejected_global,"rejected_owner":m.rejected_owner,"rejected_capacity":m.rejected_capacity,
        "rejected_waiters":m.rejected_waiters,"admission_timeouts":m.admission_timeouts,"executor_failures":m.executor_failures,
        "queue_p95_us":m.queue_p95_micros,"queue_p99_us":m.queue_p99_micros,"execution_p95_us":m.execution_p95_micros,
        "execution_p99_us":m.execution_p99_micros,"admission_wait_p99_us":m.admission_wait_p99_micros})
}
