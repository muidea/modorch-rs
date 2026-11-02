//! Bounded in-process routing, serial domains and typed request outcomes.
//!
//! # Ownership and lifecycle
//! One component owns one execution domain and may have many subscriptions. Serial,
//! Partitioned and Dynamic preserve ordering within their serial domains; Concurrent
//! does not promise order. Dynamic lane state is discarded on retirement. Shared
//! payloads must be Send + Sync. RequestObservation never retains reply ownership.
//! Contract: MSG-EVT-001, MSG-CMD-001, MSG-REQ-004, MSG-LANE-001, MSG-REPLY-001.
//!
//! # Errors and timeouts
//! Command/publish report queue admission. RequestHandle::wait separately waits for
//! completion: queued cancellation prevents execution; running timeout is Indeterminate.
//! Immediate capacity failure is Backpressure; *_wait has an admission deadline and
//! waiter/cost bounds. Handler-local *_wait is rejected. Host ingress uses external().
//!
//! # Examples
//!
//! ```
//! use modorch_eventhub::topic_matches;
//! assert!(topic_matches("orders/:id/changed", "orders/42/changed"));
//! assert!(!topic_matches("orders/42/changed", "orders/+/changed"));
//! ```
//!
//! # Stability
//! Development API 0.0.1. No durable delivery, automatic retry or cross-process transport.
//! Install workers only inside a host Tokio runtime; application shutdown owns drain.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
mod admission;
mod lanes;
mod limits;
mod routing;
pub use limits::{AdmissionMetrics, HubLimits};
mod metrics;
mod request;
pub use metrics::{
    ExecutionMetrics, MESSAGE_METRIC_LABEL_LIMIT, MessageMetricLabels, MessageMetrics, RouteOutcome,
};
mod types;
mod waitgraph;
mod workers;
use modorch_core::*;
use request::{RequestGuard, RequestShared};
pub use request::{RequestHandle, RequestObservation};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::mpsc;
pub use types::*;
use waitgraph::{EXECUTION, Node, WaitGraph};
pub use workers::*;

struct Dispatch {
    envelope: MessageEnvelope,
    lease: OperationLease,
    request: Option<RequestGuard>,
    span: tracing::Span,
    metrics: metrics::MetricSet,
    _budget: limits::BudgetGuard,
}
struct Lane {
    sender: mpsc::Sender<Dispatch>,
    node: usize,
}
struct DynamicEndpoint {
    lanes: BTreeMap<String, Lane>,
    creating: BTreeMap<String, usize>,
    runtime: tokio::runtime::Handle,
    factory: Arc<dyn HandlerFactory>,
    max_lanes: usize,
    idle_timeout: std::time::Duration,
    capacity: usize,
    next: usize,
}
struct Endpoint {
    capacity_changed: Arc<tokio::sync::Notify>,
    senders: Vec<mpsc::Sender<Dispatch>>,
    dynamic: Option<DynamicEndpoint>,
}
impl Endpoint {
    fn senders(&self) -> Vec<&mpsc::Sender<Dispatch>> {
        if let Some(dynamic) = &self.dynamic {
            dynamic.lanes.values().map(|lane| &lane.sender).collect()
        } else {
            self.senders.iter().collect()
        }
    }
}
struct Routing {
    endpoints: BTreeMap<ComponentRef, Endpoint>,
    subscriptions: routing::RouteIndex,
    generation: u64,
    terminated: bool,
}
struct Inner {
    routing: Mutex<Routing>,
    tracker: OperationTracker,
    failures: FailureReporter,
    graph: WaitGraph,
    workers: Mutex<BTreeMap<ComponentRef, Vec<tokio::task::JoinHandle<()>>>>,
    capacity_changed: Arc<tokio::sync::Notify>,
    budget: Arc<limits::Budget>,
    accepted: AtomicU64,
    rejected: AtomicU64,
    metrics: Arc<metrics::Metrics>,
    message_metrics: metrics::MessageMetricsRegistry,
}
#[derive(Clone)]
/// Application-local bounded message routing and component execution domains.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct EventHub(Arc<Inner>);
#[derive(Clone, Copy, Debug)]
/// Message admission and execution snapshot; accepted is not business completion.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct HubMetrics {
    /// Isolated application identity attached to this snapshot.
    pub application_id: ApplicationId,
    /// Cumulative accepted admissions; does not mean handler completion.
    pub accepted: u64,
    /// Cumulative rejected routing/admission attempts, including retries.
    pub rejected: u64,
    /// Accepted work awaiting execution.
    pub queued: usize,
    /// Execution policy or sampled execution counters, as indicated by the type.
    pub execution: ExecutionMetrics,
    /// Current admission-wait and declared-cost snapshot.
    pub admission: AdmissionMetrics,
    /// Retained worker controls after collecting completed dynamic workers.
    pub worker_tasks: usize,
}
impl EventHub {
    /// Create a hub with default validated admission limits.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn new(tracker: OperationTracker, failures: FailureReporter) -> Self {
        Self::with_limits(tracker, failures, HubLimits::default()).expect("valid default limits")
    }
    /// Construct an application-scoped hub after validating all admission budgets.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn with_limits(
        tracker: OperationTracker,
        failures: FailureReporter,
        limits: HubLimits,
    ) -> FrameworkResult<Self> {
        limits.validate()?;
        let changed = Arc::new(tokio::sync::Notify::new());
        Ok(Self(Arc::new(Inner {
            routing: Mutex::new(Routing {
                endpoints: BTreeMap::new(),
                subscriptions: Default::default(),
                generation: 0,
                terminated: false,
            }),
            tracker,
            failures,
            graph: WaitGraph::default(),
            workers: Mutex::new(BTreeMap::new()),
            capacity_changed: changed.clone(),
            budget: Arc::new(limits::Budget {
                limits,
                state: Default::default(),
                changed,
            }),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            metrics: Arc::default(),
            message_metrics: Default::default(),
        })))
    }
    /// Install a subscription and return its already-effective generation receipt; duplicate IDs reject it.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn subscribe(&self, spec: SubscriptionSpec) -> FrameworkResult<SubscriptionReceipt> {
        validate_pattern(&spec.topic_pattern)?;
        if spec.topic_pattern.len() > self.0.budget.limits.max_topic_bytes {
            return Err(FrameworkError::InvalidConfig(
                "subscription topic length".into(),
            ));
        }
        let mut r = self.0.routing.lock().unwrap();
        if r.terminated || self.0.tracker.is_closed() {
            return Err(FrameworkError::Closed);
        }
        if !r.endpoints.contains_key(&spec.target) {
            return Err(FrameworkError::ComponentUnavailable(spec.target));
        }
        if r.subscriptions.contains_key(&spec.id) {
            return Err(FrameworkError::InvalidConfig(format!(
                "duplicate subscription {}",
                spec.id
            )));
        }
        if r.subscriptions.len() >= self.0.budget.limits.max_subscriptions {
            return Err(FrameworkError::ResourceLimit("subscriptions".into()));
        }
        let id = spec.id;
        r.subscriptions.insert(id, spec);
        r.generation += 1;
        self.0.capacity_changed.notify_waiters();
        Ok(SubscriptionReceipt {
            id,
            generation: r.generation,
        })
    }
    /// Remove a subscription and return an already-effective generation receipt; accepted work is unaffected.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn unsubscribe(&self, id: SubscriptionId) -> FrameworkResult<SubscriptionReceipt> {
        let mut r = self.0.routing.lock().unwrap();
        if r.terminated {
            return Err(FrameworkError::Closed);
        }
        if r.subscriptions.remove(&id).is_none() {
            return Err(FrameworkError::InvalidConfig(format!(
                "unknown subscription {id}"
            )));
        }
        r.generation += 1;
        self.0.capacity_changed.notify_waiters();
        Ok(SubscriptionReceipt {
            id,
            generation: r.generation,
        })
    }
    fn targets(r: &Routing, topic: &str, destination: &Destination) -> Vec<ComponentRef> {
        r.subscriptions
            .matching(topic, destination)
            .map(|s| s.target.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn shard(endpoint: &Endpoint, lane: &Option<String>) -> usize {
        // Fixed FNV-1a rather than randomized hashing; Lane never influences routing.
        let hash = lane
            .as_deref()
            .unwrap_or("")
            .bytes()
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ b as u64).wrapping_mul(0x100000001b3)
            });
        (hash % endpoint.senders.len() as u64) as usize
    }
    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        r: &mut Routing,
        target: &ComponentRef,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        payload: MessagePayload,
        options: MessageOptions,
        request: bool,
        admission_deadline: Option<tokio::time::Instant>,
    ) -> FrameworkResult<(AdmissionReceipt, Option<Arc<RequestShared>>)> {
        let stats = self.0.message_metrics.get(Some(target), kind, topic);
        let result = self.send_inner(
            r,
            target,
            kind,
            topic,
            destination,
            payload,
            options,
            request,
            admission_deadline,
            metrics::MetricSet([self.0.metrics.clone(), stats.execution.clone()]),
        );
        stats.record(RouteOutcome::from_result(&result));
        if result.is_ok() {
            self.0.accepted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.0.rejected.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
    #[allow(clippy::too_many_arguments)]
    fn send_inner(
        &self,
        r: &mut Routing,
        target: &ComponentRef,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        payload: MessagePayload,
        options: MessageOptions,
        request: bool,
        admission_deadline: Option<tokio::time::Instant>,
        metrics: metrics::MetricSet,
    ) -> FrameworkResult<(AdmissionReceipt, Option<Arc<RequestShared>>)> {
        if r.terminated {
            return Err(FrameworkError::Closed);
        }
        Self::check_admission_deadline(admission_deadline)?;
        if options
            .deadline
            .is_some_and(|d| d <= std::time::Instant::now())
        {
            return Err(FrameworkError::AdmissionTimeout);
        }
        let bytes = self.0.budget.limits.message(topic, destination, &options)?;
        let lease = self.0.tracker.admit(target.clone())?;
        let (sender, _) = self.sender(r, target, &options.lane_key)?;
        let permit = sender.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => FrameworkError::Backpressure(target.clone()),
            mpsc::error::TrySendError::Closed(_) => FrameworkError::Closed,
        })?;
        let budget = self.0.budget.acquire(target, bytes, false)?;
        let request = request.then(|| RequestShared::new(metrics.clone()));
        let id = lease.id();
        let source = self
            .0
            .tracker
            .current()
            .and_then(|_| EXECUTION.try_with(|n| n.0.clone()).ok());
        let causation = self.0.tracker.current().map(|p| p.id());
        let envelope = MessageEnvelope {
            message_id: id,
            kind,
            topic: topic.into(),
            source,
            destination: destination.clone(),
            lane_key: options.lane_key,
            correlation_id: if kind == MessageKind::Request {
                Some(id)
            } else {
                None
            },
            causation_id: causation,
            deadline: options.deadline,
            headers: options.headers,
            payload,
        };
        // The admission limit is separate from the message execution deadline.
        // Check again after lock contention, lane construction and reservation.
        Self::check_admission_deadline(admission_deadline)?;
        if envelope
            .deadline
            .is_some_and(|d| d <= std::time::Instant::now())
        {
            return Err(FrameworkError::AdmissionTimeout);
        }
        permit.send(Dispatch {
                envelope,
                lease,
                request: request.clone().map(|shared| RequestGuard { shared, id }),
                span: if causation.is_some() {
                    tracing::info_span!("message", application_id=self.0.tracker.application_id(), operation_id=id, topic, target=%target, kind=?kind, causation_id=?causation)
                } else {
                    tracing::info_span!(parent: None, "message", application_id=self.0.tracker.application_id(), operation_id=id, topic, target=%target, kind=?kind)
                },
                metrics,
                _budget: budget,
            });
        Ok((
            AdmissionReceipt {
                operation_id: id,
                target: target.clone(),
            },
            request,
        ))
    }
    /// Fan out an event and report each target admission; accepted does not mean completed.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn publish<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        event: T,
        options: MessageOptions,
        _policy: PublishPolicy,
    ) -> FrameworkResult<PublishReport> {
        self.validate_message(MessageKind::Event, topic, &destination, &options)?;
        let r = self.0.routing.lock().unwrap();
        if r.terminated || (self.0.tracker.is_closed() && self.0.tracker.current().is_none()) {
            let result = Err(FrameworkError::Closed);
            self.record_route::<()>(None, MessageKind::Event, topic, &result);
            return Err(FrameworkError::Closed);
        }
        let targets = Self::targets(&r, topic, &destination);
        if targets.is_empty() {
            let result = Err(FrameworkError::Unroutable(topic.into()));
            self.record_route::<()>(None, MessageKind::Event, topic, &result);
            return Err(FrameworkError::Unroutable(topic.into()));
        }
        drop(r);
        let payload: MessagePayload = Arc::new(event);
        Ok(PublishReport {
            deliveries: targets
                .into_iter()
                .map(|target| {
                    let result = self
                        .prepare_target(
                            MessageKind::Event,
                            topic,
                            &destination,
                            &options.lane_key,
                            Some(&target),
                            None,
                        )
                        .and_then(|(mut r, _)| {
                            self.send(
                                &mut r,
                                &target,
                                MessageKind::Event,
                                topic,
                                &destination,
                                payload.clone(),
                                options.clone(),
                                false,
                                None,
                            )
                            .map(|(receipt, _)| receipt)
                        });
                    Delivery { target, result }
                })
                .collect(),
        })
    }
    /// Route to exactly one component and acknowledge admission only; routing or capacity errors reject it.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn command<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        command: T,
        options: MessageOptions,
    ) -> FrameworkResult<AdmissionReceipt> {
        self.validate_message(MessageKind::Command, topic, &destination, &options)?;
        let (mut r, target) = self.prepare_target(
            MessageKind::Command,
            topic,
            &destination,
            &options.lane_key,
            None,
            None,
        )?;
        self.send(
            &mut r,
            &target,
            MessageKind::Command,
            topic,
            &destination,
            Arc::new(command),
            options,
            false,
            None,
        )
        .map(|(receipt, _)| receipt)
    }
    /// Admit one typed request; await its RequestHandle for business completion, separately from admission.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn request<T: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        request: T,
        options: MessageOptions,
    ) -> FrameworkResult<RequestHandle<R>> {
        self.validate_message(MessageKind::Request, topic, &destination, &options)?;
        let (mut r, target) = self.prepare_target(
            MessageKind::Request,
            topic,
            &destination,
            &options.lane_key,
            None,
            None,
        )?;
        let (_, shard) = self
            .sender(&mut r, &target, &options.lane_key)
            .inspect_err(|error| {
                self.record_route::<()>(
                    Some(&target),
                    MessageKind::Request,
                    topic,
                    &Err(error.clone()),
                );
            })?;
        let edge = if let Some(from) = self
            .0
            .tracker
            .current()
            .and_then(|_| EXECUTION.try_with(Clone::clone).ok())
        {
            Some(
                self.0
                    .graph
                    .insert(from, Node(target.clone(), shard))
                    .inspect_err(|error| {
                        self.record_route::<()>(
                            Some(&target),
                            MessageKind::Request,
                            topic,
                            &Err(error.clone()),
                        );
                    })?,
            )
        } else {
            None
        };
        let (receipt, shared) = self.send(
            &mut r,
            &target,
            MessageKind::Request,
            topic,
            &destination,
            Arc::new(request),
            options,
            true,
            None,
        )?;
        let shared = shared.expect("request state");
        Ok(RequestHandle {
            target: target.clone(),
            id: receipt.operation_id,
            shared,
            edge,
            marker: std::marker::PhantomData,
        })
    }
    fn unique_target(
        &self,
        r: &Routing,
        _kind: MessageKind,
        topic: &str,
        destination: &Destination,
    ) -> FrameworkResult<ComponentRef> {
        if r.terminated {
            Err(FrameworkError::Closed)
        } else {
            let mut target = None;
            for spec in r.subscriptions.matching(topic, destination) {
                match target {
                    Some(previous) if previous != &spec.target => {
                        return Err(FrameworkError::AmbiguousDestination(topic.into()));
                    }
                    _ => target = Some(&spec.target),
                }
            }
            target
                .cloned()
                .ok_or_else(|| FrameworkError::Unroutable(topic.into()))
        }
    }
    fn check_admission_deadline(deadline: Option<tokio::time::Instant>) -> FrameworkResult<()> {
        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            Err(FrameworkError::AdmissionTimeout)
        } else {
            Ok(())
        }
    }
    fn validate_message(
        &self,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        options: &MessageOptions,
    ) -> FrameworkResult<usize> {
        self.0
            .budget
            .limits
            .message(topic, destination, options)
            .inspect_err(|error| {
                self.record_route::<()>(None, kind, topic, &Err(error.clone()));
            })
    }
    fn record_route<T>(
        &self,
        target: Option<&ComponentRef>,
        kind: MessageKind,
        topic: &str,
        result: &FrameworkResult<T>,
    ) {
        self.0
            .message_metrics
            .get(target, kind, topic)
            .record(RouteOutcome::from_result(result));
        if result.is_ok() {
            self.0.accepted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.0.rejected.fetch_add(1, Ordering::Relaxed);
        }
    }
    /// Bounded label sets; snapshots are independently sampled, not transactional.
    pub fn message_metrics(&self) -> Vec<MessageMetrics> {
        self.0
            .message_metrics
            .snapshot(self.0.tracker.application_id())
    }
    /// Capture independently sampled counters; this snapshot is not a transactional view.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn metrics(&self) -> HubMetrics {
        let r = self.0.routing.lock().unwrap();
        let mut workers = self.0.workers.lock().unwrap();
        for handles in workers.values_mut() {
            handles.retain(|h| !h.is_finished());
        }
        let worker_tasks = workers.values().map(Vec::len).sum();
        HubMetrics {
            worker_tasks,
            application_id: self.0.tracker.application_id(),
            accepted: self.0.accepted.load(Ordering::Relaxed),
            execution: self.0.metrics.snapshot(),
            admission: self.0.budget.state.lock().unwrap().metrics,
            rejected: self.0.rejected.load(Ordering::Relaxed),
            queued: r
                .endpoints
                .values()
                .flat_map(Endpoint::senders)
                .map(|s| s.max_capacity() - s.capacity())
                .sum(),
        }
    }
    /// Must be called after the shared tracker reaches zero. Force only aborts executors.
    pub async fn terminate(&self, force: bool) {
        {
            let mut r = self.0.routing.lock().unwrap();
            r.terminated = true;
            r.subscriptions.clear();
            r.endpoints.clear();
            self.0.capacity_changed.notify_waiters();
        }
        let workers = std::mem::take(&mut *self.0.workers.lock().unwrap());
        for worker in workers.into_values().flatten() {
            if force {
                worker.abort();
            }
            let _ = worker.await;
        }
    }
}

/// Adapter for external ingress running inside framework-owned producer tasks.
/// Every submission establishes a new root, so shutdown cannot be bypassed by
/// inheriting the producer task's pre-existing operation.
#[derive(Clone)]
pub struct ExternalEventHub(EventHub);
impl EventHub {
    /// Create a capability that clears inherited causal privileges for each external submission.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn external(&self) -> ExternalEventHub {
        ExternalEventHub(self.clone())
    }
}
impl ExternalEventHub {
    /// Route to exactly one component and acknowledge admission only; routing or capacity errors reject it.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn command<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        command: T,
        options: MessageOptions,
    ) -> FrameworkResult<AdmissionReceipt> {
        self.0
            .0
            .tracker
            .as_root(|| self.0.command(topic, destination, command, options))
    }
    /// Admit one typed request; await its RequestHandle for business completion, separately from admission.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn request<T: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        request: T,
        options: MessageOptions,
    ) -> FrameworkResult<RequestHandle<R>> {
        self.0
            .0
            .tracker
            .as_root(|| self.0.request(topic, destination, request, options))
    }
    /// Fan out an event and report each target admission; accepted does not mean completed.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn publish<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        event: T,
        options: MessageOptions,
        policy: PublishPolicy,
    ) -> FrameworkResult<PublishReport> {
        self.0
            .0
            .tracker
            .as_root(|| self.0.publish(topic, destination, event, options, policy))
    }
}

#[derive(Clone, Debug)]
/// Per-component mailbox depths, capacities and active or preparing lanes.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MailboxMetrics {
    /// Resolved destination component.
    pub target: ComponentRef,
    /// Isolated application identity attached to this snapshot.
    pub application_id: ApplicationId,
    /// Live dynamic lane instances.
    pub active_lanes: usize,
    /// Lane reservations whose synchronous factory has not installed yet.
    pub preparing_lanes: usize,
    /// Configured capacity of each mailbox in this execution domain.
    pub shard_capacity: usize,
    /// Independently sampled queue depths for each shard or active lane.
    pub shard_queue_depths: Vec<usize>,
}
impl EventHub {
    /// Snapshot mailbox depths and active/preparing lane counts.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn mailbox_metrics(&self) -> Vec<MailboxMetrics> {
        let routing = self.0.routing.lock().unwrap();
        routing
            .endpoints
            .iter()
            .map(|(target, endpoint)| MailboxMetrics {
                target: target.clone(),
                application_id: self.0.tracker.application_id(),
                active_lanes: endpoint.senders().len(),
                preparing_lanes: endpoint.dynamic.as_ref().map_or(0, |d| d.creating.len()),
                shard_capacity: endpoint
                    .dynamic
                    .as_ref()
                    .map(|d| d.capacity)
                    .unwrap_or_else(|| endpoint.senders[0].max_capacity()),
                shard_queue_depths: endpoint
                    .senders()
                    .iter()
                    .map(|sender| sender.max_capacity() - sender.capacity())
                    .collect(),
            })
            .collect()
    }
    /// Called only after the selected closure's operations and leases have drained.
    pub async fn remove_owners(&self, owners: &BTreeSet<ComponentRef>) {
        {
            let mut routing = self.0.routing.lock().unwrap();
            routing
                .subscriptions
                .retain(|_, spec| !owners.contains(&spec.target));
            routing.endpoints.retain(|owner, _| !owners.contains(owner));
            routing.generation += 1;
            self.0.capacity_changed.notify_waiters();
        }
        let workers = {
            let mut workers = self.0.workers.lock().unwrap();
            owners
                .iter()
                .flat_map(|owner| workers.remove(owner).unwrap_or_default())
                .collect::<Vec<_>>()
        };
        for worker in workers {
            let _ = worker.await;
        }
    }
}
