use crate::*;
use async_trait::async_trait;
use futures_util::FutureExt;
use std::{num::NonZeroUsize, panic::AssertUnwindSafe};

#[derive(Clone)]
/// Trusted handler identity and causal EventHub capability.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct HandlerContext {
    /// Causal message capability for trusted component child operations.
    pub event_hub: EventHub,
    /// Component associated with this boundary; absent before routing when optional.
    pub component: ComponentRef,
    /// Fixed shard index or dynamic lane instance identity.
    pub shard: usize,
    /// Tracked operation identity for diagnostics and causal accounting.
    pub operation_id: OperationId,
}
// async-trait adds a bare #[must_use] to the already must-use boxed Future.
#[allow(clippy::double_must_use)]
#[async_trait]
/// Exclusive mutable handler within one serial execution domain.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait SerialHandler: Send {
    /// Execute one message within the framework-provided causal context; return a shared reply or business error.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply>;
}
// Keep this workaround scoped to the macro-generated trait methods.
#[allow(clippy::double_must_use)]
#[async_trait]
/// Send + Sync handler shared by a bounded set of workers; no ordering promise.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait ConcurrentHandler: Send + Sync {
    /// Execute one message within the framework-provided causal context; return a shared reply or business error.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    async fn handle(
        &self,
        ctx: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply>;
}
/// Creates independent serial handlers for fixed shards or dynamic lane instances.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait HandlerFactory: Send + Sync {
    /// Create a fresh serial handler for a fixed shard or monotonically assigned dynamic lane instance ID.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    fn create(&self, shard: usize) -> FrameworkResult<Box<dyn SerialHandler>>;
}
impl<F> HandlerFactory for F
where
    F: Fn(usize) -> FrameworkResult<Box<dyn SerialHandler>> + Send + Sync,
{
    fn create(&self, shard: usize) -> FrameworkResult<Box<dyn SerialHandler>> {
        self(shard)
    }
}
/// One component-owned execution domain with any number of subscriptions.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum HandlerRegistration {
    /// One mutable handler and bounded FIFO mailbox.
    Serial {
        /// Owned mutable serial handler or shared thread-safe concurrent handler.
        handler: Box<dyn SerialHandler>,
        /// Multiple subscriptions sharing this component execution domain.
        subscriptions: Vec<SubscriptionSpec>,
    },
    /// Independent serial handlers on fixed shards.
    Partitioned {
        /// Nonzero fixed shard count; at most 4096.
        shards: NonZeroUsize,
        /// Creates an independent serial handler per shard/lane.
        factory: Box<dyn HandlerFactory>,
        /// Multiple subscriptions sharing this component execution domain.
        subscriptions: Vec<SubscriptionSpec>,
    },
    /// Bounded per-key serial lanes retired after idle timeout.
    Dynamic {
        /// Maximum combined active and preparing dynamic lanes; at most 4096.
        max_lanes: NonZeroUsize,
        /// Positive empty-lane retirement interval; does not preserve handler private state.
        idle_timeout: std::time::Duration,
        /// Creates an independent serial handler per shard/lane.
        factory: Box<dyn HandlerFactory>,
        /// Multiple subscriptions sharing this component execution domain.
        subscriptions: Vec<SubscriptionSpec>,
    },
    /// Shared handler with a strict concurrency bound and no ordering guarantee.
    Concurrent {
        /// Maximum simultaneous work in this execution domain.
        max_in_flight: NonZeroUsize,
        /// Owned mutable serial handler or shared thread-safe concurrent handler.
        handler: Arc<dyn ConcurrentHandler>,
        /// Multiple subscriptions sharing this component execution domain.
        subscriptions: Vec<SubscriptionSpec>,
    },
}
enum Workers {
    Serial(Vec<Box<dyn SerialHandler>>),
    Concurrent(usize, Arc<dyn ConcurrentHandler>),
    Dynamic(DynamicEndpoint),
}
impl EventHub {
    /// Install one validated execution domain and subscriptions; requires a current host Tokio runtime.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn install(
        &self,
        descriptor: &ComponentDescriptor,
        registration: HandlerRegistration,
    ) -> FrameworkResult<()> {
        tokio::runtime::Handle::try_current().map_err(|_| {
            FrameworkError::InvalidConfig("EventHub workers require a host Tokio runtime".into())
        })?;
        let target = descriptor.component();
        if descriptor.queue_capacity == 0
            || descriptor.queue_capacity > tokio::sync::Semaphore::MAX_PERMITS
        {
            return Err(FrameworkError::InvalidConfig(
                "invalid queue capacity".into(),
            ));
        }
        let (policy, subscriptions, workers) = match registration {
            HandlerRegistration::Serial {
                handler,
                subscriptions,
            } => (
                ExecutionPolicy::Serial,
                subscriptions,
                Workers::Serial(vec![handler]),
            ),
            HandlerRegistration::Partitioned {
                shards,
                factory,
                subscriptions,
            } => {
                if shards.get() > 4096 {
                    return Err(FrameworkError::InvalidConfig("shards exceed 4096".into()));
                }
                let handlers = (0..shards.get())
                    .map(|shard| factory.create(shard))
                    .collect::<FrameworkResult<Vec<_>>>()?;
                (
                    ExecutionPolicy::Partitioned { shards },
                    subscriptions,
                    Workers::Serial(handlers),
                )
            }
            HandlerRegistration::Dynamic {
                max_lanes,
                idle_timeout,
                factory,
                subscriptions,
            } => {
                if max_lanes.get() > 4096
                    || idle_timeout.is_zero()
                    || std::time::Instant::now()
                        .checked_add(idle_timeout)
                        .is_none()
                {
                    return Err(FrameworkError::InvalidConfig(
                        "invalid dynamic lane bounds".into(),
                    ));
                }
                (
                    ExecutionPolicy::Dynamic {
                        max_lanes,
                        idle_timeout,
                    },
                    subscriptions,
                    Workers::Dynamic(DynamicEndpoint {
                        lanes: BTreeMap::new(),
                        creating: BTreeMap::new(),
                        runtime: tokio::runtime::Handle::current(),
                        factory: Arc::from(factory),
                        max_lanes: max_lanes.get(),
                        idle_timeout,
                        capacity: descriptor.queue_capacity,
                        next: 0,
                    }),
                )
            }
            HandlerRegistration::Concurrent {
                max_in_flight,
                handler,
                subscriptions,
            } => {
                if max_in_flight.get() > 4096 {
                    return Err(FrameworkError::InvalidConfig(
                        "concurrent workers exceed 4096".into(),
                    ));
                }
                (
                    ExecutionPolicy::Concurrent { max_in_flight },
                    subscriptions,
                    Workers::Concurrent(max_in_flight.get(), handler),
                )
            }
        };
        if policy != descriptor.execution {
            return Err(FrameworkError::InvalidConfig(format!(
                "handler policy does not match {target}"
            )));
        }
        let mut r = self.0.routing.lock().unwrap();
        if r.terminated || self.0.tracker.is_closed() {
            return Err(FrameworkError::Closed);
        }
        if r.endpoints.contains_key(&target) {
            return Err(FrameworkError::DuplicateComponent(target));
        }
        if r.subscriptions.len().saturating_add(subscriptions.len())
            > self.0.budget.limits.max_subscriptions
        {
            return Err(FrameworkError::ResourceLimit("subscriptions".into()));
        }
        let capacity_changed = Arc::new(tokio::sync::Notify::new());
        let mut ids = BTreeSet::new();
        for spec in &subscriptions {
            validate_pattern(&spec.topic_pattern)?;
            if spec.topic_pattern.len() > self.0.budget.limits.max_topic_bytes {
                return Err(FrameworkError::InvalidConfig(
                    "subscription topic length".into(),
                ));
            }
            if spec.target != target
                || r.subscriptions.contains_key(&spec.id)
                || !ids.insert(spec.id)
            {
                return Err(FrameworkError::InvalidConfig(
                    "invalid subscription target or duplicate ID".into(),
                ));
            }
        }
        let count = match &workers {
            Workers::Serial(handlers) => handlers.len(),
            _ => 1,
        };
        let (senders, receivers): (Vec<_>, Vec<_>) = (0..count)
            .map(|_| mpsc::channel(descriptor.queue_capacity))
            .unzip();
        let dynamic = match workers {
            Workers::Dynamic(dynamic) => {
                r.endpoints.insert(
                    target.clone(),
                    Endpoint {
                        capacity_changed: capacity_changed.clone(),
                        senders: vec![],
                        dynamic: Some(dynamic),
                    },
                );
                for spec in subscriptions {
                    r.subscriptions.insert(spec.id, spec);
                }
                r.generation += 1;
                return Ok(());
            }
            workers => workers,
        };
        let workers = dynamic;
        r.endpoints.insert(
            target.clone(),
            Endpoint {
                capacity_changed: capacity_changed.clone(),
                senders,
                dynamic: None,
            },
        );
        for spec in subscriptions {
            r.subscriptions.insert(spec.id, spec);
        }
        r.generation += 1;
        let mut all_handles = self.0.workers.lock().unwrap();
        let handles = all_handles.entry(target.clone()).or_default();
        match workers {
            Workers::Dynamic(_) => unreachable!(),
            Workers::Serial(handlers) => {
                for (shard, (mut handler, mut receiver)) in
                    handlers.into_iter().zip(receivers).enumerate()
                {
                    let hub = self.clone();
                    let component = target.clone();
                    let capacity_changed = capacity_changed.clone();
                    handles.push(tokio::spawn(async move {
                        while let Some(dispatch) = receiver.recv().await {
                            capacity_changed.notify_waiters();
                            let context = HandlerContext {
                                event_hub: hub.clone(),
                                component: component.clone(),
                                shard,
                                operation_id: dispatch.lease.id(),
                            };
                            hub.execute(&context, dispatch, |message| {
                                handler.handle(&context, message)
                            })
                            .await;
                        }
                    }));
                }
            }
            Workers::Concurrent(limit, handler) => {
                let mut receiver = receivers.into_iter().next().unwrap();
                let (available, mut ready) = mpsc::channel(limit);
                let mut executors = Vec::with_capacity(limit);
                for index in 0..limit {
                    let (sender, mut input) = mpsc::channel::<Dispatch>(1);
                    executors.push(sender);
                    available.try_send(index).expect("initial worker capacity");
                    let available = available.clone();
                    let hub = self.clone();
                    let handler = handler.clone();
                    let component = target.clone();
                    handles.push(tokio::spawn(async move {
                        while let Some(dispatch) = input.recv().await {
                            let context = HandlerContext {
                                event_hub: hub.clone(),
                                component: component.clone(),
                                shard: 0,
                                operation_id: dispatch.lease.id(),
                            };
                            hub.execute(&context, dispatch, |message| {
                                handler.handle(&context, message)
                            })
                            .await;
                            if available.send(index).await.is_err() {
                                break;
                            }
                        }
                    }));
                }
                drop(available);
                let changed = capacity_changed.clone();
                // A separate dispatcher keeps receiver progress independent of user
                // polling. Reserve an idle worker before dequeueing, so internal
                // handoff cannot increase capacity + max_in_flight residency.
                handles.push(tokio::spawn(async move {
                    while let Some(index) = ready.recv().await {
                        let Some(dispatch) = receiver.recv().await else {
                            break;
                        };
                        changed.notify_waiters();
                        if executors[index].send(dispatch).await.is_err() {
                            break;
                        }
                    }
                }));
            }
        }
        Ok(())
    }
    async fn execute<F, Fut>(&self, context: &HandlerContext, dispatch: Dispatch, handler: F)
    where
        F: FnOnce(MessageEnvelope) -> Fut,
        Fut: std::future::Future<Output = FrameworkResult<HandlerReply>>,
    {
        let Dispatch {
            envelope,
            lease,
            request,
            span,
            metrics,
            _budget,
        } = dispatch;
        if let Some(shared) = &request {
            if envelope
                .deadline
                .is_some_and(|deadline| deadline <= std::time::Instant::now())
            {
                shared.cancel_wait(lease.id(), true);
                return;
            }
            if !shared.begin() {
                return;
            }
        }
        let mut metrics = crate::metrics::ExecutionGuard::new(metrics);
        use tracing::Instrument;
        let generation = self.0.tracker.generation(&context.component);
        let outcome = self
            .0
            .tracker
            .scope(
                lease.clone(),
                EXECUTION.scope(
                    Node(context.component.clone(), context.shard),
                    AssertUnwindSafe(async { handler(envelope).await }).catch_unwind(),
                ),
            )
            .instrument(span)
            .await;
        let result = match outcome {
            Ok(result) => result.map_err(|error| {
                error.in_execution(
                    context.component.clone(),
                    Some(context.operation_id),
                    Phase::Handle,
                )
            }),
            Err(_) => {
                let error = FrameworkError::ExecutionFailed {
                    component: context.component.clone(),
                    operation_id: Some(context.operation_id),
                    phase: Phase::Handle,
                    detail: "handler panicked".into(),
                    source: None,
                };
                self.0
                    .failures
                    .report_for(context.component.clone(), generation, error.clone());
                Err(error)
            }
        };
        metrics.success = result.is_ok();
        if let Some(shared) = request {
            shared.complete(result);
        } else if let Err(error) = result {
            self.0
                .failures
                .report_for(context.component.clone(), generation, error);
        }
    }
}

impl EventHub {
    pub(crate) fn sender(
        &self,
        routing: &mut Routing,
        target: &ComponentRef,
        key: &Option<String>,
    ) -> FrameworkResult<(mpsc::Sender<Dispatch>, usize)> {
        let endpoint = routing
            .endpoints
            .get_mut(target)
            .ok_or_else(|| FrameworkError::Unroutable(target.to_string()))?;
        if endpoint.dynamic.is_none() {
            let shard = Self::shard(endpoint, key);
            return Ok((endpoint.senders[shard].clone(), shard));
        }
        let dynamic = endpoint.dynamic.as_ref().unwrap();
        let key = key.as_deref().unwrap_or("");
        dynamic
            .lanes
            .get(key)
            .map(|lane| (lane.sender.clone(), lane.node))
            .ok_or_else(|| FrameworkError::Backpressure(target.clone()))
    }
    pub(crate) fn start_lane(
        &self,
        routing: &mut Routing,
        plan: &crate::lanes::LanePreparation,
        mut handler: Box<dyn SerialHandler>,
    ) {
        let target = &plan.target;
        let node = plan.node;
        let key = plan.key.clone();
        let capacity_changed = plan.changed.clone();
        let dynamic = routing
            .endpoints
            .get_mut(target)
            .unwrap()
            .dynamic
            .as_mut()
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(plan.capacity);
        dynamic.lanes.insert(
            key.clone(),
            Lane {
                sender: sender.clone(),
                node,
            },
        );
        let idle = plan.idle;
        let hub = self.clone();
        let component = target.clone();
        let join = plan.runtime.spawn(async move {
            loop {
                let dispatch = match tokio::time::timeout(idle, receiver.recv()).await {
                    Ok(message) => message,
                    Err(_) => {
                        let mut routing = hub.0.routing.lock().unwrap();
                        match receiver.try_recv() {
                            Ok(message) => Some(message),
                            Err(_) => {
                                if let Some(dynamic) = routing
                                    .endpoints
                                    .get_mut(&component)
                                    .and_then(|e| e.dynamic.as_mut())
                                {
                                    if dynamic
                                        .lanes
                                        .get(&key)
                                        .is_some_and(|lane| lane.node == node)
                                    {
                                        dynamic.lanes.remove(&key);
                                    }
                                }
                                capacity_changed.notify_waiters();
                                None
                            }
                        }
                    }
                };
                let Some(dispatch) = dispatch else {
                    break;
                };
                capacity_changed.notify_waiters();
                let context = HandlerContext {
                    event_hub: hub.clone(),
                    component: component.clone(),
                    shard: node,
                    operation_id: dispatch.lease.id(),
                };
                hub.execute(&context, dispatch, |message| {
                    handler.handle(&context, message)
                })
                .await;
            }
        });
        let mut workers = self.0.workers.lock().unwrap();
        let handles = workers.entry(target.clone()).or_default();
        handles.retain(|h| !h.is_finished());
        handles.push(join);
    }
}
