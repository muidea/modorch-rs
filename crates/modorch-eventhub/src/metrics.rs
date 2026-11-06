use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};
#[derive(Default)]
pub(crate) struct Metrics {
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
    pub completed: AtomicU64,
    pub failed: AtomicU64,
    pub timeouts: AtomicU64,
    pub canceled: AtomicU64,
    pub caller_cancellations: AtomicU64,
    latency: [AtomicU64; 32],
}
#[derive(Clone, Copy, Debug, Default)]
/// Handler execution counters and logarithmic microsecond latency estimates.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ExecutionMetrics {
    /// Currently active executions or drains.
    pub active: usize,
    /// Peak simultaneous handler executions.
    pub peak: usize,
    /// Cumulative completed executions/drains; executor failures may be a subset.
    pub completed: u64,
    /// Cumulative failed executions.
    pub failed: u64,
    /// Response wait deadlines exceeded; not proof of nonexecution.
    pub caller_timeouts: u64,
    /// Queued work canceled before the business handler began.
    pub canceled_before_start: u64,
    /// Explicitly abandoned caller waits.
    pub caller_cancellations: u64,
    /// Upper bucket estimates, cumulative since application startup, in microseconds.
    pub p95_micros: u64,
    /// Estimated P99 handler execution duration, not end-to-end latency.
    pub p99_micros: u64,
}
impl Metrics {
    pub fn snapshot(&self) -> ExecutionMetrics {
        let bins = self.latency.each_ref().map(|n| n.load(Ordering::Relaxed));
        let total: u64 = bins.iter().sum();
        let quantile = |percent: u64| {
            if total == 0 {
                return 0;
            }
            let threshold = total.saturating_mul(percent).div_ceil(100);
            let mut count = 0;
            for (i, bin) in bins.iter().enumerate() {
                count += bin;
                if count >= threshold {
                    return 1u64 << i;
                }
            }
            1u64 << 31
        };
        ExecutionMetrics {
            active: self.active.load(Ordering::Relaxed),
            peak: self.peak.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            caller_timeouts: self.timeouts.load(Ordering::Relaxed),
            canceled_before_start: self.canceled.load(Ordering::Relaxed),
            caller_cancellations: self.caller_cancellations.load(Ordering::Relaxed),
            p95_micros: quantile(95),
            p99_micros: quantile(99),
        }
    }
}
pub(crate) struct ExecutionGuard {
    metrics: MetricSet,
    started: Instant,
    pub success: bool,
}
impl ExecutionGuard {
    pub fn new(metrics: MetricSet) -> Self {
        for m in &metrics.0 {
            let active = m.active.fetch_add(1, Ordering::Relaxed) + 1;
            m.peak.fetch_max(active, Ordering::Relaxed);
        }
        Self {
            metrics,
            started: Instant::now(),
            success: false,
        }
    }
}
impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        let micros = self
            .started
            .elapsed()
            .as_micros()
            .clamp(1, u64::MAX as u128) as u64;
        let bucket = (u64::BITS - (micros - 1).leading_zeros()).min(31) as usize;
        for metrics in &self.metrics.0 {
            metrics.active.fetch_sub(1, Ordering::Relaxed);
            metrics.completed.fetch_add(1, Ordering::Relaxed);
            if !self.success {
                metrics.failed.fetch_add(1, Ordering::Relaxed);
            }
            metrics.latency[bucket].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Maximum exact label sets retained per hub. Additional sets share an explicit
/// overflow row so untrusted, dynamically generated Topics cannot grow memory.
pub const MESSAGE_METRIC_LABEL_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
/// Classification of a routing/admission attempt, not a business outcome.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum RouteOutcome {
    /// Bounded admission succeeded; business completion is not implied.
    Accepted,
    /// No subscribed target matches topic and destination.
    Unroutable,
    /// A Command or Request matched multiple target components.
    AmbiguousDestination,
    /// Immediate admission has no available bounded capacity.
    Backpressure,
    /// Root/owner admission or a generation-bound capability is closed.
    Closed,
    /// Root admission is not open yet.
    NotReady,
    /// The separate admission wait budget expired.
    AdmissionTimeout,
    /// The framework request wait graph would contain a cycle.
    CircularWait,
    /// Routing failure outside the separately classified outcomes.
    OtherFailure,
}
impl RouteOutcome {
    const ALL: [Self; 9] = [
        Self::Accepted,
        Self::Unroutable,
        Self::AmbiguousDestination,
        Self::Backpressure,
        Self::Closed,
        Self::NotReady,
        Self::AdmissionTimeout,
        Self::CircularWait,
        Self::OtherFailure,
    ];
    pub(crate) fn from_result<T>(result: &modorch_core::FrameworkResult<T>) -> Self {
        use modorch_core::FrameworkError as E;
        match result {
            Ok(_) => Self::Accepted,
            Err(E::Unroutable(_)) => Self::Unroutable,
            Err(E::AmbiguousDestination(_)) => Self::AmbiguousDestination,
            Err(E::Backpressure(_)) => Self::Backpressure,
            Err(E::Closed) => Self::Closed,
            Err(E::NotReady) => Self::NotReady,
            Err(E::AdmissionTimeout) => Self::AdmissionTimeout,
            Err(E::CircularWait(_)) => Self::CircularWait,
            Err(_) => Self::OtherFailure,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Bounded message metric identity; overflow aggregates excess label sets.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MessageMetricLabels {
    /// None for routing failures before a target is known, or for overflow.
    pub component: Option<modorch_core::ComponentRef>,
    /// None only for overflow.
    pub kind: Option<crate::MessageKind>,
    /// None only for overflow. Empty Topics are distinct from overflow.
    pub topic: Option<String>,
}
#[derive(Clone, Debug)]
/// Per-label routing attempts and handler execution snapshot.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MessageMetrics {
    /// Isolated application identity attached to this snapshot.
    pub application_id: modorch_core::ApplicationId,
    /// Bounded message metric identity.
    pub labels: MessageMetricLabels,
    /// True for aggregate counters beyond exact-label capacity.
    pub overflow: bool,
    /// Admission attempts, including retries by *_wait; not final business results.
    pub routing: std::collections::BTreeMap<RouteOutcome, u64>,
    /// Execution policy or sampled execution counters, as indicated by the type.
    pub execution: ExecutionMetrics,
}
#[derive(Default)]
pub(crate) struct MessageStats {
    routes: [AtomicU64; 9],
    pub execution: Arc<Metrics>,
}
impl MessageStats {
    pub fn record(&self, outcome: RouteOutcome) {
        self.routes[outcome as usize].fetch_add(1, Ordering::Relaxed);
    }
}
type Topics = std::collections::BTreeMap<String, Arc<MessageStats>>;
type Kinds = std::collections::BTreeMap<crate::MessageKind, Topics>;
#[derive(Default)]
struct RegistryState {
    targeted: std::collections::BTreeMap<modorch_core::ComponentRef, Kinds>,
    untargeted: Kinds,
    rows: usize,
    bytes: usize,
    overflow: Option<Arc<MessageStats>>,
}
#[derive(Default)]
pub(crate) struct MessageMetricsRegistry {
    entries: std::sync::Mutex<RegistryState>,
}
impl MessageMetricsRegistry {
    pub fn get(
        &self,
        component: Option<&modorch_core::ComponentRef>,
        kind: crate::MessageKind,
        topic: &str,
    ) -> Arc<MessageStats> {
        let mut state = self.entries.lock().unwrap();
        let kinds = match component {
            Some(c) => state.targeted.get(c),
            None => Some(&state.untargeted),
        };
        if let Some(stats) = kinds
            .and_then(|k| k.get(&kind))
            .and_then(|topics| topics.get(topic))
        {
            return stats.clone();
        }
        let bytes = topic
            .len()
            .saturating_add(component.map_or(0, |c| c.id.len()));
        // Independent byte budget also bounds diagnostic labels from rejected input.
        if state.rows >= MESSAGE_METRIC_LABEL_LIMIT || bytes > 1024 * 1024 - state.bytes {
            return state.overflow.get_or_insert_with(Default::default).clone();
        }
        state.rows += 1;
        state.bytes += bytes;
        let kinds = match component {
            Some(c) => state.targeted.entry(c.clone()).or_default(),
            None => &mut state.untargeted,
        };
        kinds
            .entry(kind)
            .or_default()
            .entry(topic.into())
            .or_default()
            .clone()
    }
    pub fn snapshot(&self, application_id: modorch_core::ApplicationId) -> Vec<MessageMetrics> {
        let rows = {
            let state = self.entries.lock().unwrap();
            let mut rows = Vec::with_capacity(state.rows + usize::from(state.overflow.is_some()));
            let mut append = |component: Option<&modorch_core::ComponentRef>, kinds: &Kinds| {
                for (kind, topics) in kinds {
                    for (topic, stats) in topics {
                        rows.push((
                            MessageMetricLabels {
                                component: component.cloned(),
                                kind: Some(*kind),
                                topic: Some(topic.clone()),
                            },
                            stats.clone(),
                        ));
                    }
                }
            };
            append(None, &state.untargeted);
            for (component, kinds) in &state.targeted {
                append(Some(component), kinds);
            }
            if let Some(stats) = &state.overflow {
                rows.push((
                    MessageMetricLabels {
                        component: None,
                        kind: None,
                        topic: None,
                    },
                    stats.clone(),
                ));
            }
            rows
        };
        rows.into_iter()
            .map(|(labels, stats)| MessageMetrics {
                application_id,
                overflow: labels.kind.is_none(),
                labels,
                routing: RouteOutcome::ALL
                    .into_iter()
                    .map(|outcome| {
                        (
                            outcome,
                            stats.routes[outcome as usize].load(Ordering::Relaxed),
                        )
                    })
                    .collect(),
                execution: stats.execution.snapshot(),
            })
            .collect()
    }
}
#[derive(Clone)]
pub(crate) struct MetricSet(pub [Arc<Metrics>; 2]);
impl MetricSet {
    pub fn canceled(&self) {
        for m in &self.0 {
            m.canceled.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn caller_canceled(&self) {
        for m in &self.0 {
            m.caller_cancellations.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn timed_out(&self) {
        for m in &self.0 {
            m.timeouts.fetch_add(1, Ordering::Relaxed);
        }
    }
}
