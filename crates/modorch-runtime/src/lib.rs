//! Application ownership, lifecycle transactions and retryable checked shutdown.
#![forbid(unsafe_code)]
mod component;
mod metrics;
pub use metrics::{DrainMetrics, RuntimeMetrics};
mod supervision;
pub use component::*;
use futures_util::FutureExt;
pub use modorch_core::*;
pub use modorch_eventhub::*;
pub use modorch_registry::*;
pub use modorch_scheduler::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
    time::Duration,
};
pub use supervision::{ComponentHealth, ComponentState};
use tokio::sync::{Mutex as AsyncMutex, watch};

pub struct ApplicationBuilder {
    name: String,
    factories: BTreeMap<ComponentRef, Factory>,
    selection: ComponentSelection,
    resolution: DependencyResolution,
    max_tasks: usize,
    max_timers: usize,
    message_limits: HubLimits,
    execution_resources: ExecutionResources,
    journal_capacity: usize,
    queue_capacities: Option<(usize, usize)>,
}
impl ApplicationBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            factories: BTreeMap::new(),
            selection: ComponentSelection::new(),
            resolution: DependencyResolution::RequireSelected,
            max_tasks: 1024,
            max_timers: 256,
            message_limits: HubLimits::default(),
            execution_resources: ExecutionResources::default(),
            journal_capacity: 1024,
            queue_capacities: None,
        }
    }
    pub fn register_initiator(
        &mut self,
        factory: impl InitiatorFactory + 'static,
    ) -> FrameworkResult<&mut Self> {
        self.register(Factory::Initiator(Box::new(factory)))
    }
    pub fn register_module(
        &mut self,
        factory: impl ModuleFactory + 'static,
    ) -> FrameworkResult<&mut Self> {
        self.register(Factory::Module(Box::new(factory)))
    }
    fn register(&mut self, factory: Factory) -> FrameworkResult<&mut Self> {
        let descriptor = factory.descriptor();
        let component = descriptor.component();
        let valid = matches!(
            (&factory, component.kind),
            (Factory::Initiator(_), ComponentKind::Initiator)
                | (Factory::Module(_), ComponentKind::Module)
        );
        if !valid {
            return Err(FrameworkError::InvalidConfig(format!(
                "factory kind does not match {component}"
            )));
        }
        if self.factories.contains_key(&component) {
            return Err(FrameworkError::DuplicateComponent(component));
        }
        self.factories.insert(component, factory);
        Ok(self)
    }
    pub fn selection(mut self, selection: ComponentSelection) -> Self {
        self.selection = selection;
        self
    }
    pub fn resolve_dependencies(mut self, resolution: DependencyResolution) -> Self {
        self.resolution = resolution;
        self
    }
    pub fn task_limits(mut self, tasks: usize, timers: usize) -> Self {
        self.max_tasks = tasks;
        self.max_timers = timers;
        self
    }
    pub fn message_limits(mut self, limits: HubLimits) -> Self {
        self.message_limits = limits;
        self
    }
    pub fn execution_resources(mut self, resources: ExecutionResources) -> Self {
        self.execution_resources = resources;
        self
    }
    /// Retain the most recent lifecycle records; health keeps the latest failure.
    pub fn journal_capacity(mut self, capacity: usize) -> Self {
        self.journal_capacity = capacity;
        self
    }
    /// Application-wide overrides, applied at mailbox installation. Partitioned
    /// components use per_shard; other policies use per_component.
    pub fn queue_capacities(mut self, per_component: usize, per_shard: usize) -> Self {
        self.queue_capacities = Some((per_component, per_shard));
        self
    }
    pub fn build(self) -> FrameworkResult<Application> {
        self.message_limits.validate()?;
        self.execution_resources.validate()?;
        if self.journal_capacity == 0 {
            return Err(FrameworkError::InvalidConfig(
                "zero journal capacity".into(),
            ));
        }
        if let Some((component, shard)) = self.queue_capacities {
            if [component, shard]
                .iter()
                .any(|c| *c == 0 || *c > tokio::sync::Semaphore::MAX_PERMITS)
            {
                return Err(FrameworkError::InvalidConfig(
                    "invalid mailbox capacities".into(),
                ));
            }
        }
        if self.name.trim().is_empty() {
            return Err(FrameworkError::InvalidConfig(
                "empty application name".into(),
            ));
        }
        let descriptors: BTreeMap<_, _> = self
            .factories
            .iter()
            .map(|(c, f)| (c.clone(), f.descriptor()))
            .collect();
        for (key, descriptor) in &descriptors {
            if key != &descriptor.component() {
                return Err(FrameworkError::InvalidConfig(
                    "factory descriptor changed after registration".into(),
                ));
            }
        }
        let plan = resolve(&descriptors, &self.selection, self.resolution)?;
        for c in &plan {
            self.factories[c].validate(&self.selection.config(c))?;
        }
        let tracker = OperationTracker::new();
        let failures = FailureReporter::new();
        let services = ServiceRegistry::new(tracker.clone());
        let hub = EventHub::with_limits(tracker.clone(), failures.clone(), self.message_limits)?;
        let scheduler = SchedulerRuntime::with_resources(
            tracker.clone(),
            failures.clone(),
            self.max_tasks,
            self.max_timers,
            self.execution_resources,
        )?;
        let (state, _) = watch::channel(ApplicationState::New);
        let inner = Arc::new(Inner {
            name: self.name,
            descriptors: descriptors.clone(),
            factories: self
                .factories
                .into_iter()
                .map(|(c, f)| (c, Arc::new(f)))
                .collect(),
            selection: self.selection.clone(),
            queue_capacities: self.queue_capacities,
            component_health: Mutex::new(
                plan.iter()
                    .map(|c| {
                        (
                            c.clone(),
                            ComponentHealth {
                                state: ComponentState::Starting,
                                generation: 0,
                                restarts: 0,
                                last_failure: None,
                            },
                        )
                    })
                    .collect(),
            ),
            terminal_failure: watch::channel(None).0,
            supervisor_abort: Mutex::new(None),
            state,
            phase: Mutex::new(Phase::Setup),
            tracker,
            failures,
            services,
            hub,
            scheduler,
            entries: AsyncMutex::new(vec![]),
            journal: Mutex::new(VecDeque::new()),
            journal_capacity: self.journal_capacity,
            journal_dropped: std::sync::atomic::AtomicU64::new(0),
            drain_metrics: Default::default(),
            shutdown: AsyncMutex::new(()),
            shutdown_root: Mutex::new(None),
            shutdown_job: Mutex::new(None),
        });
        Ok(Application {
            inner,
            descriptors,
            plan,
            selection: self.selection,
            queue_capacities: self.queue_capacities,
        })
    }
}
pub struct Application {
    inner: Arc<Inner>,
    descriptors: BTreeMap<ComponentRef, ComponentDescriptor>,
    plan: Vec<ComponentRef>,
    selection: ComponentSelection,
    queue_capacities: Option<(usize, usize)>,
}
struct Entry {
    descriptor: ComponentDescriptor,
    instance: Instance,
    begin_done: bool,
    quiesce_done: bool,
    teardown_done: bool,
}
struct ShutdownJob {
    result: watch::Receiver<Option<FrameworkResult<ShutdownReceipt>>>,
    abort: tokio::task::AbortHandle,
}
struct Inner {
    descriptors: BTreeMap<ComponentRef, ComponentDescriptor>,
    factories: BTreeMap<ComponentRef, Arc<Factory>>,
    selection: ComponentSelection,
    queue_capacities: Option<(usize, usize)>,
    component_health: Mutex<BTreeMap<ComponentRef, ComponentHealth>>,
    terminal_failure: watch::Sender<Option<FrameworkError>>,
    supervisor_abort: Mutex<Option<tokio::task::AbortHandle>>,
    name: String,
    state: watch::Sender<ApplicationState>,
    phase: Mutex<Phase>,
    tracker: OperationTracker,
    failures: FailureReporter,
    services: ServiceRegistry,
    hub: EventHub,
    scheduler: SchedulerRuntime,
    entries: AsyncMutex<Vec<Entry>>,
    journal: Mutex<VecDeque<LifecycleRecord>>,
    journal_capacity: usize,
    journal_dropped: std::sync::atomic::AtomicU64,
    drain_metrics: metrics::DrainTracker,
    shutdown: AsyncMutex<()>,
    shutdown_root: Mutex<Option<OperationLease>>,
    shutdown_job: Mutex<Option<ShutdownJob>>,
}
#[derive(Clone)]
pub struct ApplicationHandle {
    inner: Arc<Inner>,
}
#[derive(Clone, Debug)]
pub struct LifecycleRecord {
    /// State captured before rollback/teardown can change component health.
    pub state: ComponentState,
    pub component: ComponentRef,
    pub phase: Phase,
    pub outcome: FrameworkResult<()>,
}
#[derive(Clone, Copy, Debug)]
pub struct ShutdownOptions {
    pub timeout: Duration,
}
impl ShutdownOptions {
    pub fn graceful() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }
}
impl Default for ShutdownOptions {
    fn default() -> Self {
        Self::graceful()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShutdownReceipt {
    GracefullyStopped,
    Forced { pending: PendingWork },
}
#[derive(Clone, Debug)]
pub struct Health {
    pub application_id: ApplicationId,
    pub name: String,
    pub state: ApplicationState,
    pub live: bool,
    pub ready: bool,
    pub failure: Option<FrameworkError>,
    pub pending: PendingWork,
    pub components: BTreeMap<ComponentRef, ComponentHealth>,
}
pub struct StartupError {
    /// Box the primary error to keep startup results small while retaining cleanup ownership.
    pub source: Box<FrameworkError>,
    pub cleanup: FrameworkResult<ShutdownReceipt>,
    pub handle: ApplicationHandle,
}
impl std::fmt::Debug for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartupError")
            .field("source", &self.source)
            .field("cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "startup failed: {}; cleanup: {:?}",
            self.source, self.cleanup
        )
    }
}
impl std::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl Inner {
    fn set_state(&self, state: ApplicationState) {
        self.state.send_if_modified(|current| {
            if matches!(
                *current,
                ApplicationState::Stopped | ApplicationState::Forced
            ) {
                return false;
            }
            *current = state;
            true
        });
    }
    fn record(&self, c: ComponentRef, phase: Phase, outcome: FrameworkResult<()>) {
        let state = {
            let mut health = self.component_health.lock().unwrap();
            let status = health.get_mut(&c).expect("selected component");
            if let Err(error) = &outcome {
                status.state = if phase == Phase::Setup {
                    ComponentState::PartiallyInitialized
                } else {
                    ComponentState::Failed
                };
                status.last_failure = Some(error.clone());
            }
            status.state
        };
        tracing::info!(application_id=self.tracker.application_id(), component=%c, ?phase, ?state, success=outcome.is_ok(), error=?outcome.as_ref().err(), "lifecycle hook completed");
        let mut journal = self.journal.lock().unwrap();
        if journal.len() == self.journal_capacity {
            journal.pop_front();
            self.journal_dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        journal.push_back(LifecycleRecord {
            state,
            component: c,
            phase,
            outcome,
        });
    }
    fn context(&self, d: &ComponentDescriptor) -> ComponentContext {
        ComponentContext {
            component: d.component(),
            services: self
                .services
                .reader(d.component(), d.requires.iter().chain(&d.optional).cloned()),
            event_hub: self.hub.clone(),
            tasks: self.scheduler.for_owner(d.component()),
        }
    }
    fn stop_context(&self, d: &ComponentDescriptor, phase: Phase) -> ShutdownContext {
        ShutdownContext {
            component: d.component(),
            phase,
            services: self.context(d).services,
        }
    }
    fn incomplete(&self, phase: Phase, errors: Vec<FrameworkError>) -> FrameworkError {
        FrameworkError::StopIncomplete {
            phase,
            pending: self.tracker.pending(),
            errors,
        }
    }
}
async fn checked_hook<F: std::future::Future<Output = FrameworkResult<()>>>(
    tracker: &OperationTracker,
    component: &ComponentRef,
    phase: Phase,
    future: F,
) -> FrameworkResult<()> {
    match AssertUnwindSafe(future).catch_unwind().await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.in_execution(
            component.clone(),
            tracker.current().map(|lease| lease.id()),
            phase,
        )),
        Err(_) => Err(FrameworkError::ExecutionFailed {
            component: component.clone(),
            operation_id: tracker.current().map(|lease| lease.id()),
            phase,
            detail: "lifecycle hook panicked".into(),
            source: None,
        }),
    }
}
impl Application {
    pub fn startup_plan(&self) -> &[ComponentRef] {
        &self.plan
    }
    pub async fn start(self) -> Result<ApplicationHandle, StartupError> {
        // Continue startup ownership and rollback even if the caller cancels this await.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = self.start_owned().await;
            if let Err(Ok(handle)) = sender.send(result) {
                let _ = handle.shutdown_checked(ShutdownOptions::graceful()).await;
            }
        });
        receiver
            .await
            .expect("startup coordinator unexpectedly panicked")
    }
    async fn start_owned(self) -> Result<ApplicationHandle, StartupError> {
        let handle = ApplicationHandle {
            inner: self.inner.clone(),
        };
        let result = match AssertUnwindSafe(self.initialize()).catch_unwind().await {
            Ok(result) => result,
            Err(_) => Err(FrameworkError::InvalidConfig(
                "startup coordinator panicked; initiating owned rollback".into(),
            )),
        };
        if let Err(source) = result {
            handle.inner.set_state(ApplicationState::Failed);
            let cleanup = handle.shutdown_checked(ShutdownOptions::graceful()).await;
            if cleanup.is_err() {
                let retained = handle.clone();
                tokio::spawn(async move {
                    retained.retain_until_stopped().await;
                });
            }
            return Err(StartupError {
                source: Box::new(source),
                cleanup,
                handle,
            });
        }
        let supervisor = handle.clone();
        let job = tokio::spawn(async move {
            supervisor.supervise().await;
        });
        *handle.inner.supervisor_abort.lock().unwrap() = Some(job.abort_handle());
        Ok(handle)
    }
    async fn initialize(self) -> FrameworkResult<()> {
        self.inner.set_state(ApplicationState::Initializing);
        let mut entries = self.inner.entries.lock().await;
        for c in &self.plan {
            let descriptor = &self.descriptors[c];
            let factory = &self.inner.factories[c];
            let instance = std::panic::catch_unwind(AssertUnwindSafe(|| {
                factory.create(&self.selection.config(c))
            }))
            .map_err(|_| FrameworkError::ExecutionFailed {
                component: c.clone(),
                operation_id: None,
                phase: Phase::Setup,
                detail: "factory panicked".into(),
                source: None,
            })?
            .map_err(|error| error.in_execution(c.clone(), None, Phase::Setup))?;
            // Register ownership before setup, including partially initialized failures.
            entries.push(Entry {
                descriptor: descriptor.clone(),
                instance,
                begin_done: false,
                quiesce_done: false,
                teardown_done: false,
            });
            let entry = entries.last_mut().unwrap();
            if entry.instance.descriptor() != *descriptor {
                return Err(FrameworkError::InvalidConfig(format!(
                    "instance descriptor differs from factory for {c}"
                )));
            }
            let ctx = self.inner.context(descriptor);
            let result = checked_hook(
                &self.inner.tracker,
                c,
                Phase::Setup,
                entry.instance.lifecycle().setup(&ctx),
            )
            .await;
            self.inner.record(c.clone(), Phase::Setup, result.clone());
            result?;
            let extra = std::panic::catch_unwind(AssertUnwindSafe(|| -> FrameworkResult<()> {
                if let Instance::Initiator(i) = &entry.instance {
                    i.export_services(&mut self.inner.services.publisher(c.clone())?)?;
                }
                if let Some(registration) = entry.instance.handlers()? {
                    let mut mailbox = descriptor.clone();
                    if let Some((component, shard)) = self.queue_capacities {
                        mailbox.queue_capacity =
                            if matches!(mailbox.execution, ExecutionPolicy::Partitioned { .. }) {
                                shard
                            } else {
                                component
                            };
                    }
                    self.inner.hub.install(&mailbox, registration)?;
                }
                Ok(())
            }))
            .map_err(|_| FrameworkError::ExecutionFailed {
                component: c.clone(),
                operation_id: None,
                phase: Phase::Setup,
                detail: "service export or handler factory panicked".into(),
                source: None,
            })?;
            extra.map_err(|error| error.in_execution(c.clone(), None, Phase::Setup))?;
        }
        self.inner.set_state(ApplicationState::Starting);
        for entry in entries.iter_mut() {
            let c = entry.descriptor.component();
            let ctx = self.inner.context(&entry.descriptor);
            let result = checked_hook(
                &self.inner.tracker,
                &c,
                Phase::Start,
                entry.instance.lifecycle().start(&ctx),
            )
            .await;
            self.inner.record(c.clone(), Phase::Start, result.clone());
            result?;
        }
        for entry in entries.iter_mut() {
            let c = entry.descriptor.component();
            let ctx = self.inner.context(&entry.descriptor);
            let result = checked_hook(
                &self.inner.tracker,
                &c,
                Phase::Ready,
                entry.instance.lifecycle().ready(&ctx),
            )
            .await;
            self.inner.record(c, Phase::Ready, result.clone());
            result?;
        }
        if let Some(error) = self.inner.failures.current() {
            return Err(error);
        }
        // All readiness barriers passed before any ingress is activated.
        self.inner.tracker.open()?;
        for entry in entries.iter_mut() {
            let c = entry.descriptor.component();
            let ctx = self.inner.context(&entry.descriptor);
            if let Instance::Initiator(i) = &mut entry.instance {
                let result = checked_hook(
                    &self.inner.tracker,
                    &c,
                    Phase::ActivateIngress,
                    i.activate_ingress(&ctx),
                )
                .await;
                self.inner.record(c, Phase::ActivateIngress, result.clone());
                result?;
            }
        }
        for status in self.inner.component_health.lock().unwrap().values_mut() {
            status.state = ComponentState::Ready;
        }
        self.inner.set_state(ApplicationState::Ready);
        Ok(())
    }
}
impl ApplicationHandle {
    pub fn id(&self) -> ApplicationId {
        self.inner.tracker.application_id()
    }
    pub fn task_metrics(&self) -> TaskMetrics {
        self.inner.scheduler.metrics()
    }
    pub fn event_hub(&self) -> EventHub {
        self.inner.hub.clone()
    }
    pub fn health(&self) -> Health {
        let state = *self.inner.state.borrow();
        let failure = self
            .inner
            .terminal_failure
            .borrow()
            .clone()
            .or_else(|| self.inner.failures.current());
        Health {
            application_id: self.id(),
            name: self.inner.name.clone(),
            state,
            live: !matches!(state, ApplicationState::Stopped | ApplicationState::Forced),
            ready: state == ApplicationState::Ready && failure.is_none(),
            failure,
            pending: self.inner.tracker.pending(),
            components: self.inner.component_health.lock().unwrap().clone(),
        }
    }
    pub fn lifecycle_journal(&self) -> Vec<LifecycleRecord> {
        self.inner.journal.lock().unwrap().iter().cloned().collect()
    }
    pub fn begin_shutdown(&self) {
        self.close_admission();
        self.ensure_shutdown_job();
    }
    fn close_admission(&self) {
        if *self.inner.state.borrow() == ApplicationState::Forced {
            return;
        }
        {
            let mut root = self.inner.shutdown_root.lock().unwrap();
            if root.is_none() {
                *root = self
                    .inner
                    .tracker
                    .close_with_lease(ComponentRef::initiator("__modorch_lifecycle"));
            }
        }
        self.inner.scheduler.begin_shutdown();
        if !matches!(
            *self.inner.state.borrow(),
            ApplicationState::Stopped | ApplicationState::Forced
        ) {
            self.inner.set_state(ApplicationState::Quiescing);
        }
    }
    fn ensure_shutdown_job(&self) -> watch::Receiver<Option<FrameworkResult<ShutdownReceipt>>> {
        let mut job = self.inner.shutdown_job.lock().unwrap();
        if let Some(existing) = &*job {
            if existing.result.borrow().as_ref().is_none_or(|r| r.is_ok()) {
                return existing.result.clone();
            }
        }
        let (sender, receiver) = watch::channel(None);
        let handle = self.clone();
        let join = tokio::spawn(async move {
            let result = handle.shutdown_transaction().await;
            sender.send_replace(Some(result));
        });
        *job = Some(ShutdownJob {
            result: receiver.clone(),
            abort: join.abort_handle(),
        });
        receiver
    }
    async fn retain_until_stopped(&self) {
        let mut state = self.inner.state.subscribe();
        loop {
            if matches!(
                *state.borrow(),
                ApplicationState::Stopped | ApplicationState::Forced
            ) {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }
    pub async fn wait(&self) -> FrameworkResult<()> {
        let mut state = self.inner.state.subscribe();
        let mut faults = self.inner.terminal_failure.subscribe();
        loop {
            if let Some(error) = faults.borrow().clone() {
                return Err(error);
            }
            if matches!(
                *state.borrow(),
                ApplicationState::Stopped | ApplicationState::Forced
            ) {
                return Ok(());
            }
            tokio::select! { result = state.changed() => result.map_err(|_| FrameworkError::Closed)?, result = faults.changed() => result.map_err(|_| FrameworkError::Closed)? }
        }
    }
    pub async fn shutdown_checked(
        &self,
        options: ShutdownOptions,
    ) -> FrameworkResult<ShutdownReceipt> {
        if *self.inner.state.borrow() == ApplicationState::Forced {
            return Ok(ShutdownReceipt::Forced {
                pending: self.inner.tracker.pending(),
            });
        }
        self.close_admission();
        let mut job = self.ensure_shutdown_job();
        let wait = async {
            loop {
                if let Some(result) = job.borrow().clone() {
                    return result;
                }
                if job.changed().await.is_err() {
                    if *self.inner.state.borrow() == ApplicationState::Forced {
                        return Ok(ShutdownReceipt::Forced {
                            pending: self.inner.tracker.pending(),
                        });
                    }
                    return Err(FrameworkError::Closed);
                }
            }
        };
        match tokio::time::timeout(options.timeout, wait).await {
            Ok(result) => result,
            Err(_) => {
                if let Some(result) = job.borrow().clone() {
                    return result;
                }
                match *self.inner.state.borrow() {
                    ApplicationState::Stopped => return Ok(ShutdownReceipt::GracefullyStopped),
                    ApplicationState::Forced => {
                        return Ok(ShutdownReceipt::Forced {
                            pending: self.inner.tracker.pending(),
                        });
                    }
                    _ => {}
                }
                self.inner.set_state(ApplicationState::StopIncomplete);
                tracing::warn!(application_id=self.id(), phase=?*self.inner.phase.lock().unwrap(), pending=?self.inner.tracker.pending(), outcome="incomplete", "shutdown caller deadline elapsed; transaction retained");
                Err(self
                    .inner
                    .incomplete(*self.inner.phase.lock().unwrap(), vec![]))
            }
        }
    }
    async fn shutdown_transaction(&self) -> FrameworkResult<ShutdownReceipt> {
        let _transaction = self.inner.shutdown.lock().await;
        match *self.inner.state.borrow() {
            ApplicationState::Stopped => return Ok(ShutdownReceipt::GracefullyStopped),
            ApplicationState::Forced => {
                return Ok(ShutdownReceipt::Forced {
                    pending: self.inner.tracker.pending(),
                });
            }
            _ => {}
        }
        let mut entries = self.inner.entries.lock().await;
        for phase in [Phase::BeginShutdown, Phase::Quiesce] {
            *self.inner.phase.lock().unwrap() = phase;
            let mut errors = vec![];
            for entry in entries.iter_mut() {
                if entry.teardown_done
                    || (phase == Phase::BeginShutdown && entry.begin_done)
                    || (phase == Phase::Quiesce && entry.quiesce_done)
                {
                    continue;
                }
                let c = entry.descriptor.component();
                let ctx = self.inner.stop_context(&entry.descriptor, phase);
                let hook = async {
                    match phase {
                        Phase::BeginShutdown => {
                            entry.instance.lifecycle().begin_shutdown(&ctx).await
                        }
                        _ => entry.instance.lifecycle().quiesce(&ctx).await,
                    }
                };
                let operation = self.inner.shutdown_root.lock().unwrap().clone();
                let result = if let Some(operation) = operation {
                    self.inner
                        .tracker
                        .scope(
                            operation,
                            checked_hook(&self.inner.tracker, &c, phase, hook),
                        )
                        .await
                } else {
                    checked_hook(&self.inner.tracker, &c, phase, hook).await
                };
                self.inner.record(c, phase, result.clone());
                match result {
                    Ok(()) => {
                        if phase == Phase::BeginShutdown {
                            entry.begin_done = true
                        } else {
                            entry.quiesce_done = true
                        }
                    }
                    Err(error) => errors.push(error),
                }
            }
            if !errors.is_empty() {
                self.inner.set_state(ApplicationState::StopIncomplete);
                return Err(self.inner.incomplete(phase, errors));
            }
        }
        self.inner.shutdown_root.lock().unwrap().take();
        *self.inner.phase.lock().unwrap() = Phase::Drain;
        self.inner.set_state(ApplicationState::Draining);
        let drain = self.inner.drain_metrics.begin();
        self.inner.tracker.drain().await;
        drain.complete();
        *self.inner.phase.lock().unwrap() = Phase::Teardown;
        self.inner.set_state(ApplicationState::Stopping);
        for entry in entries.iter_mut().rev() {
            if entry.teardown_done {
                continue;
            }
            let c = entry.descriptor.component();
            self.inner.services.retire(&c);
            let ctx = self.inner.stop_context(&entry.descriptor, Phase::Teardown);
            let result = checked_hook(
                &self.inner.tracker,
                &c,
                Phase::Teardown,
                entry.instance.lifecycle().teardown(&ctx),
            )
            .await;
            self.inner
                .record(c.clone(), Phase::Teardown, result.clone());
            if let Err(error) = result {
                self.inner.set_state(ApplicationState::StopIncomplete);
                return Err(self.inner.incomplete(Phase::Teardown, vec![error]));
            }
            entry.teardown_done = true;
            self.inner.services.remove(&c);
            if let Some(status) = self.inner.component_health.lock().unwrap().get_mut(&c) {
                status.state = ComponentState::Stopped;
            }
        }
        *self.inner.phase.lock().unwrap() = Phase::Terminate;
        self.inner.hub.terminate(false).await;
        for status in self.inner.component_health.lock().unwrap().values_mut() {
            status.state = ComponentState::Stopped;
        }
        self.inner.tracker.terminate();
        self.inner.set_state(ApplicationState::Stopped);
        if *self.inner.state.borrow() == ApplicationState::Forced {
            Ok(ShutdownReceipt::Forced {
                pending: self.inner.tracker.pending(),
            })
        } else {
            tracing::info!(
                application_id = self.id(),
                outcome = "graceful",
                "application shutdown completed"
            );
            Ok(ShutdownReceipt::GracefullyStopped)
        }
    }
    /// Aborts framework futures. External side effects and retained resources may remain.
    pub async fn force_shutdown(&self) -> ShutdownReceipt {
        if *self.inner.state.borrow() == ApplicationState::Stopped {
            return ShutdownReceipt::GracefullyStopped;
        }
        self.inner.set_state(ApplicationState::Forced);
        if let Some(abort) = self.inner.supervisor_abort.lock().unwrap().as_ref() {
            abort.abort();
        }
        if let Some(job) = self.inner.shutdown_job.lock().unwrap().as_ref() {
            job.abort.abort();
        }
        self.inner.tracker.terminate();
        self.inner.shutdown_root.lock().unwrap().take();
        self.inner.scheduler.force_cancel();
        self.inner.hub.terminate(true).await;
        self.inner.set_state(ApplicationState::Forced);
        let pending = self.inner.tracker.pending();
        tracing::warn!(
            application_id = self.id(),
            outcome = "forced",
            ?pending,
            "application executors aborted"
        );
        ShutdownReceipt::Forced { pending }
    }
}
