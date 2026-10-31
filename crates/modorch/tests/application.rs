#![cfg(feature = "runtime")]
use modorch::{prelude::*, *};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Semaphore;

type Log = Arc<Mutex<Vec<String>>>;
#[derive(Clone)]
struct Factory {
    d: ComponentDescriptor,
    log: Log,
    creates: Arc<AtomicUsize>,
    fail_setup: bool,
    fail_start: bool,
    fail_teardown_once: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}
impl Factory {
    fn new(c: ComponentRef, log: &Log) -> Self {
        Self {
            d: ComponentDescriptor::new(c),
            log: log.clone(),
            creates: Arc::default(),
            fail_setup: false,
            fail_start: false,
            fail_teardown_once: Arc::default(),
            closed: Arc::default(),
        }
    }
    fn make(&self) -> Box<Component> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Box::new(Component(self.clone()))
    }
}
impl InitiatorFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(self.make())
    }
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(self.make())
    }
}
struct Component(Factory);
impl Component {
    fn record(&self, phase: &str) {
        self.0
            .log
            .lock()
            .unwrap()
            .push(format!("{}:{phase}", self.0.d.id));
    }
}
#[async_trait]
impl Lifecycle for Component {
    async fn setup(&mut self, _: &SetupContext) -> FrameworkResult<()> {
        self.record("setup");
        if self.0.fail_setup {
            return Err(FrameworkError::InvalidConfig("partial setup".into()));
        }
        Ok(())
    }
    async fn start(&mut self, _: &RuntimeContext) -> FrameworkResult<()> {
        self.record("start");
        if self.0.fail_start {
            return Err(FrameworkError::InvalidConfig("start".into()));
        }
        Ok(())
    }
    async fn begin_shutdown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.record("begin");
        Ok(())
    }
    async fn quiesce(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.record("quiesce");
        Ok(())
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.record("teardown");
        if self.0.fail_teardown_once.swap(false, Ordering::SeqCst) {
            return Err(FrameworkError::InvalidConfig("retry teardown".into()));
        }
        self.0.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
#[async_trait]
impl Initiator for Component {
    fn descriptor(&self) -> ComponentDescriptor {
        self.0.d.clone()
    }
    fn export_services(&self, p: &mut ServicePublisher) -> FrameworkResult<()> {
        p.publish("resource", self.0.closed.clone())
    }
    fn event_endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        if self.0.d.id != "events" {
            return Ok(None);
        }
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(InitiatorEcho),
            subscriptions: vec![SubscriptionSpec::new(
                77,
                self.0.d.component(),
                "init/echo/v1",
            )],
        }))
    }
}
impl Module for Component {
    fn descriptor(&self) -> ComponentDescriptor {
        self.0.d.clone()
    }
}
#[tokio::test]
async fn only_enabled_factories_create_and_dag_orders_lifecycle() {
    let log = Log::default();
    let db = Factory::new(ComponentRef::initiator("db"), &log);
    let a = Factory::new(ComponentRef::module("a"), &log);
    let mut b = Factory::new(ComponentRef::module("b"), &log);
    b.d.requires = vec![a.d.component(), db.d.component()];
    let disabled = Factory::new(ComponentRef::module("disabled"), &log);
    let count = disabled.creates.clone();
    let mut builder = ApplicationBuilder::new("dag");
    builder
        .register_module(b)
        .unwrap()
        .register_module(disabled)
        .unwrap()
        .register_module(a)
        .unwrap();
    builder.register_initiator(db).unwrap();
    let mut selection = ComponentSelection::new();
    selection
        .enable_initiator("db")
        .enable_module("b")
        .enable_module("a");
    let application = builder.selection(selection).build().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        application.startup_plan(),
        &[
            ComponentRef::initiator("db"),
            ComponentRef::module("a"),
            ComponentRef::module("b")
        ]
    );
    let handle = application.start().await.unwrap();
    assert!(handle.health().ready);
    handle
        .shutdown_checked(ShutdownOptions::graceful())
        .await
        .unwrap();
    handle
        .shutdown_checked(ShutdownOptions::graceful())
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let logs = log.lock().unwrap();
    assert_eq!(
        &logs[..6],
        [
            "db:setup", "a:setup", "b:setup", "db:start", "a:start", "b:start"
        ]
    );
    let teardown: Vec<_> = logs
        .iter()
        .filter(|s| s.ends_with(":teardown"))
        .map(String::as_str)
        .collect();
    assert_eq!(teardown, vec!["b:teardown", "a:teardown", "db:teardown"]);
}
#[tokio::test]
async fn default_selection_is_empty_duplicates_and_unavailable_are_errors() {
    let log = Log::default();
    let f = Factory::new(ComponentRef::module("a"), &log);
    let creates = f.creates.clone();
    let mut b = ApplicationBuilder::new("empty");
    b.register_module(f.clone()).unwrap();
    assert!(matches!(
        b.register_module(f),
        Err(FrameworkError::DuplicateComponent(_))
    ));
    let handle = b.build().unwrap().start().await.unwrap();
    assert_eq!(creates.load(Ordering::SeqCst), 0);
    handle.shutdown_checked(Default::default()).await.unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_module("not-compiled");
    assert!(matches!(
        ApplicationBuilder::new("unavailable")
            .selection(selection)
            .build(),
        Err(FrameworkError::ComponentUnavailable(_))
    ));
}
#[tokio::test]
async fn partial_setup_rolls_back_and_preserves_primary_error() {
    let log = Log::default();
    let db = Factory::new(ComponentRef::initiator("db"), &log);
    let mut broken = Factory::new(ComponentRef::module("broken"), &log);
    broken.fail_setup = true;
    let mut builder = ApplicationBuilder::new("rollback");
    builder.register_initiator(db).unwrap();
    builder.register_module(broken).unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("db").enable_module("broken");
    let error = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .err()
        .unwrap();
    assert!(error.source.to_string().contains("partial setup"));
    assert!(error.cleanup.is_ok());
    let setup = error
        .handle
        .lifecycle_journal()
        .into_iter()
        .find(|record| {
            record.component == ComponentRef::module("broken") && record.phase == Phase::Setup
        })
        .unwrap();
    assert_eq!(setup.state, ComponentState::PartiallyInitialized);
    assert_eq!(
        setup.outcome.unwrap_err().component(),
        Some(&ComponentRef::module("broken"))
    );
    assert_eq!(error.handle.health().state, ApplicationState::Stopped);
    assert!(log.lock().unwrap().contains(&"broken:teardown".into()));
    assert!(log.lock().unwrap().contains(&"db:teardown".into()));
}
#[tokio::test]
async fn startup_error_is_bounded_and_preserves_failed_cleanup_for_retry() {
    assert!(std::mem::size_of::<StartupError>() <= 128);
    let log = Log::default();
    let mut broken = Factory::new(ComponentRef::module("broken"), &log);
    broken.fail_setup = true;
    broken.fail_teardown_once.store(true, Ordering::SeqCst);
    let closed = broken.closed.clone();
    let mut builder = ApplicationBuilder::new("startup-cleanup-retry");
    builder.register_module(broken).unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_module("broken");
    let error = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .err()
        .expect("setup must fail");
    let source = std::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<FrameworkError>()
        .expect("source must expose the original framework error");
    assert_eq!(source.phase(), Some(Phase::Setup));
    assert!(
        matches!(source.root_cause(), FrameworkError::InvalidConfig(detail) if detail == "partial setup")
    );
    assert!(matches!(
        &error.cleanup,
        Err(FrameworkError::StopIncomplete { phase: Phase::Teardown, errors, .. })
            if errors.iter().any(|error| matches!(error.root_cause(), FrameworkError::InvalidConfig(detail) if detail == "retry teardown"))
    ));
    assert!(!closed.load(Ordering::SeqCst));
    assert_eq!(
        error
            .handle
            .shutdown_checked(Default::default())
            .await
            .unwrap(),
        ShutdownReceipt::GracefullyStopped
    );
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(error.handle.health().pending.operations, 0);
    assert!(
        matches!(error.source.root_cause(), FrameworkError::InvalidConfig(detail) if detail == "partial setup")
    );
}

#[tokio::test]
async fn retries_skip_successful_teardown_checkpoints() {
    let log = Log::default();
    let a = Factory::new(ComponentRef::module("a"), &log);
    let b = Factory::new(ComponentRef::module("b"), &log);
    a.fail_teardown_once.store(true, Ordering::SeqCst);
    let mut builder = ApplicationBuilder::new("retry");
    builder
        .register_module(a)
        .unwrap()
        .register_module(b)
        .unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_module("a").enable_module("b");
    let handle = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    assert!(matches!(
        handle.shutdown_checked(Default::default()).await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Teardown,
            ..
        })
    ));
    handle.shutdown_checked(Default::default()).await.unwrap();
    let logs = log.lock().unwrap();
    assert_eq!(
        logs.iter().filter(|s| s.as_str() == "b:teardown").count(),
        1
    );
    assert_eq!(
        logs.iter().filter(|s| s.as_str() == "a:teardown").count(),
        2
    );
}
struct ChildModule {
    d: ComponentDescriptor,
    gate: Arc<Semaphore>,
    resource_closed: Arc<AtomicBool>,
    child: bool,
}
#[async_trait]
impl Lifecycle for ChildModule {}
impl Module for ChildModule {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(ChildHandler {
                gate: self.gate.clone(),
                closed: self.resource_closed.clone(),
                child: self.child,
            }),
            subscriptions: vec![SubscriptionSpec::new(
                if self.child { 2 } else { 1 },
                self.d.component(),
                "work/v1",
            )],
        }))
    }
}
struct ChildFactory {
    child: bool,
    gate: Arc<Semaphore>,
    closed: Arc<AtomicBool>,
}
impl ModuleFactory for ChildFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d =
            ComponentDescriptor::new(ComponentRef::module(if self.child { "b" } else { "a" }));
        d.requires.push(ComponentRef::initiator("db"));
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(ChildModule {
            d: self.descriptor(),
            gate: self.gate.clone(),
            resource_closed: self.closed.clone(),
            child: self.child,
        }))
    }
}
struct ChildHandler {
    gate: Arc<Semaphore>,
    closed: Arc<AtomicBool>,
    child: bool,
}
#[async_trait]
impl SerialHandler for ChildHandler {
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        if !self.child {
            self.gate.acquire().await.unwrap().forget();
            ctx.event_hub
                .request::<_, usize>(
                    "work/v1",
                    Destination::Component(ComponentRef::module("b")),
                    (),
                    Default::default(),
                )?
                .wait(Duration::from_secs(1))
                .await?;
        }
        assert!(!self.closed.load(Ordering::SeqCst));
        Ok(Arc::new(42usize))
    }
}
#[tokio::test]
async fn shutdown_timeout_preserves_resources_and_accepts_only_causal_children() {
    let gate = Arc::new(Semaphore::new(0));
    let log = Log::default();
    let db = Factory::new(ComponentRef::initiator("db"), &log);
    let closed = db.closed.clone();
    let mut builder = ApplicationBuilder::new("drain");
    builder.register_initiator(db).unwrap();
    for child in [false, true] {
        builder
            .register_module(ChildFactory {
                child,
                gate: gate.clone(),
                closed: closed.clone(),
            })
            .unwrap();
    }
    let mut selection = ComponentSelection::new();
    selection
        .enable_initiator("db")
        .enable_module("a")
        .enable_module("b");
    let handle = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let hub = handle.event_hub();
    let request = hub
        .request::<_, usize>(
            "work/v1",
            Destination::Component(ComponentRef::module("a")),
            (),
            Default::default(),
        )
        .unwrap();
    tokio::task::yield_now().await;
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(1)
            })
            .await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Drain,
            ..
        })
    ));
    assert!(!closed.load(Ordering::SeqCst));
    let draining = handle.metrics();
    assert_eq!(draining.application_id, handle.id());
    assert_eq!(draining.drain.started, 1);
    assert_eq!(draining.drain.active, 1);
    assert_eq!(draining.drain.completed, 0);
    assert!(draining.drain.active_elapsed > Duration::ZERO);
    assert!(matches!(
        hub.command(
            "work/v1",
            Destination::Component(ComponentRef::module("b")),
            (),
            Default::default()
        ),
        Err(FrameworkError::Closed)
    ));
    gate.add_permits(1);
    assert_eq!(*request.wait(Duration::from_secs(1)).await.unwrap(), 42);
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(handle.health().pending.operations, 0);
    let drained = handle.metrics();
    assert_eq!(drained.drain.started, 1);
    assert_eq!(drained.drain.completed, 1);
    assert_eq!(drained.drain.active, 0);
    assert_eq!(drained.drain.interrupted, 0);
    assert!(drained.drain.completed_elapsed >= draining.drain.active_elapsed);
    assert!(drained.drain.last_completed.is_some());
    #[cfg(feature = "observability")]
    assert_eq!(
        modorch::observe::ApplicationMetrics::capture(&handle)
            .0
            .application_id,
        handle.id()
    );
}
#[tokio::test]
async fn applications_are_isolated_in_one_runtime() {
    let one = ApplicationBuilder::new("one")
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let two = ApplicationBuilder::new("two")
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    assert_ne!(one.id(), two.id());
    one.shutdown_checked(Default::default()).await.unwrap();
    assert!(two.health().ready);
    two.shutdown_checked(Default::default()).await.unwrap();
}

struct ProbeData {
    setup_entered: Semaphore,
    setup_gate: Option<Arc<Semaphore>>,
    quiesce_gate: Option<Arc<Semaphore>>,
    fault_gate: Option<Arc<Semaphore>>,
    quiesces: AtomicUsize,
    torn_down: AtomicBool,
    service: Mutex<Option<ServiceHandle<AtomicBool>>>,
    tasks: Mutex<Option<TaskScheduler>>,
}
impl Default for ProbeData {
    fn default() -> Self {
        Self {
            setup_entered: Semaphore::new(0),
            setup_gate: None,
            quiesce_gate: None,
            fault_gate: None,
            quiesces: AtomicUsize::new(0),
            torn_down: AtomicBool::new(false),
            service: Mutex::new(None),
            tasks: Mutex::new(None),
        }
    }
}
struct ProbeFactory(Arc<ProbeData>);
impl ModuleFactory for ProbeFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module("probe"));
        d.requires.push(ComponentRef::initiator("db"));
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Probe {
            d: self.descriptor(),
            data: self.0.clone(),
        }))
    }
}
struct Probe {
    d: ComponentDescriptor,
    data: Arc<ProbeData>,
}
#[async_trait]
impl Lifecycle for Probe {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        self.data.setup_entered.add_permits(1);
        if let Some(gate) = &self.data.setup_gate {
            gate.acquire().await.unwrap().forget();
        }
        *self.data.service.lock().unwrap() = Some(
            ctx.services
                .get(&ComponentRef::initiator("db"), "resource")?,
        );
        Ok(())
    }
    async fn start(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        *self.data.tasks.lock().unwrap() = Some(ctx.tasks.clone());
        if let Some(gate) = self.data.fault_gate.clone() {
            ctx.tasks.spawn(move |_| async move {
                gate.acquire().await.unwrap().forget();
                panic!("supervised task fault");
                #[allow(unreachable_code)]
                Ok(())
            })?;
        }
        Ok(())
    }
    async fn quiesce(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.data.quiesces.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.data.quiesce_gate {
            gate.acquire().await.unwrap().forget();
        }
        // Lifecycle flushes may still acquire dependencies after roots close.
        let lease = self
            .data
            .service
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .acquire()?;
        assert!(!lease.load(Ordering::SeqCst));
        Ok(())
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.data.torn_down.store(true, Ordering::SeqCst);
        Ok(())
    }
}
impl Module for Probe {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
}
fn probe_application(data: Arc<ProbeData>) -> (Application, Arc<AtomicBool>) {
    let log = Log::default();
    let db = Factory::new(ComponentRef::initiator("db"), &log);
    let closed = db.closed.clone();
    let mut builder = ApplicationBuilder::new("probe");
    builder.register_initiator(db).unwrap();
    builder.register_module(ProbeFactory(data)).unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("db").enable_module("probe");
    (builder.selection(selection).build().unwrap(), closed)
}
#[tokio::test]
async fn externally_held_service_leases_block_owner_teardown() {
    let data = Arc::new(ProbeData::default());
    let (app, closed) = probe_application(data.clone());
    let handle = app.start().await.unwrap();
    let service = data.service.lock().unwrap().as_ref().unwrap().clone();
    let lease = service.acquire().unwrap();
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(1)
            })
            .await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Drain,
            ..
        })
    ));
    assert!(!closed.load(Ordering::SeqCst));
    assert!(matches!(
        service.acquire(),
        Err(FrameworkError::ServiceUnavailable(_))
    ));
    drop(lease);
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(closed.load(Ordering::SeqCst));
    assert!(service.acquire().is_err());
}
#[tokio::test]
async fn timed_out_quiesce_continues_once_and_can_flush_dependencies() {
    let gate = Arc::new(Semaphore::new(0));
    let data = Arc::new(ProbeData {
        quiesce_gate: Some(gate.clone()),
        ..Default::default()
    });
    let (app, closed) = probe_application(data.clone());
    let handle = app.start().await.unwrap();
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(1)
            })
            .await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Quiesce,
            ..
        })
    ));
    assert!(!closed.load(Ordering::SeqCst));
    gate.add_permits(1);
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert_eq!(data.quiesces.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancellation_of_startup_does_not_abandon_created_resources() {
    let gate = Arc::new(Semaphore::new(0));
    let data = Arc::new(ProbeData {
        setup_gate: Some(gate.clone()),
        ..Default::default()
    });
    let (app, closed) = probe_application(data.clone());
    let start = tokio::spawn(async move { app.start().await });
    data.setup_entered.acquire().await.unwrap().forget();
    start.abort();
    let _ = start.await;
    gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !closed.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(data.torn_down.load(Ordering::SeqCst));
}
#[tokio::test]
async fn supervised_panic_triggers_failfast_shutdown_with_a_failure_receipt() {
    let gate = Arc::new(Semaphore::new(0));
    let data = Arc::new(ProbeData {
        fault_gate: Some(gate.clone()),
        ..Default::default()
    });
    let (app, closed) = probe_application(data);
    let handle = app.start().await.unwrap();
    gate.add_permits(1);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), handle.wait())
            .await
            .unwrap(),
        Err(FrameworkError::ExecutionFailed { .. })
    ));
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(closed.load(Ordering::SeqCst));
    assert!(!handle.health().ready);
}
#[tokio::test]
async fn force_shutdown_never_reports_a_graceful_stop_or_releases_busy_owners() {
    let gate = Arc::new(Semaphore::new(0));
    let data = Arc::new(ProbeData {
        quiesce_gate: Some(gate),
        ..Default::default()
    });
    let (app, closed) = probe_application(data);
    let handle = app.start().await.unwrap();
    let _ = handle
        .shutdown_checked(ShutdownOptions {
            timeout: Duration::from_millis(1),
        })
        .await;
    assert!(matches!(
        handle.force_shutdown().await,
        ShutdownReceipt::Forced { .. }
    ));
    assert_eq!(handle.health().state, ApplicationState::Forced);
    assert!(!closed.load(Ordering::SeqCst));
    assert!(matches!(
        handle.shutdown_checked(Default::default()).await.unwrap(),
        ShutdownReceipt::Forced { .. }
    ));
    assert!(!closed.load(Ordering::SeqCst));
}

struct InitiatorEcho;
#[async_trait]
impl SerialHandler for InitiatorEcho {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        Ok(message.payload)
    }
}
#[tokio::test]
async fn initiator_can_register_an_optional_event_endpoint() {
    let log = Log::default();
    let mut builder = ApplicationBuilder::new("initiator-endpoint");
    builder
        .register_initiator(Factory::new(ComponentRef::initiator("events"), &log))
        .unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("events");
    let handle = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let reply = handle
        .event_hub()
        .request::<_, usize>(
            "init/echo/v1",
            Destination::Component(ComponentRef::initiator("events")),
            7usize,
            Default::default(),
        )
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(*reply, 7);
    handle.shutdown_checked(Default::default()).await.unwrap();
}

#[tokio::test]
async fn forced_drain_is_interrupted_and_never_counted_as_completed() {
    let data = Arc::new(ProbeData::default());
    let (app, closed) = probe_application(data.clone());
    let handle = app.start().await.unwrap();
    let lease = data
        .service
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .acquire()
        .unwrap();
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(1)
            })
            .await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Drain,
            ..
        })
    ));
    assert_eq!(handle.metrics().drain.active, 1);
    assert!(matches!(
        handle.force_shutdown().await,
        ShutdownReceipt::Forced { .. }
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.metrics().drain.active > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let metrics = handle.metrics();
    assert_eq!(metrics.drain.started, 1);
    assert_eq!(metrics.drain.completed, 0);
    assert_eq!(metrics.drain.interrupted, 1);
    assert!(metrics.drain.interrupted_elapsed > Duration::ZERO);
    assert!(metrics.drain.last_completed.is_none());
    assert!(!closed.load(Ordering::SeqCst));
    drop(lease);
}

#[tokio::test]
async fn lifecycle_journal_is_bounded_and_startup_errors_survive_eviction() {
    let log = Log::default();
    let mut builder = ApplicationBuilder::new("journal").journal_capacity(2);
    let mut broken = Factory::new(ComponentRef::module("broken"), &log);
    broken.fail_start = true;
    builder.register_module(broken).unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_module("broken");
    let error = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .err()
        .expect("startup must fail");
    assert!(
        matches!(error.source.root_cause(), FrameworkError::InvalidConfig(detail) if detail == "start")
    );
    assert_eq!(error.handle.lifecycle_journal().len(), 2);
    assert!(error.handle.metrics().journal_dropped > 0);
    assert!(
        error.handle.health().components[&ComponentRef::module("broken")]
            .last_failure
            .is_some()
    );
}

#[tokio::test]
async fn blocking_work_keeps_its_service_and_owner_alive_after_await_is_abandoned() {
    let data = Arc::new(ProbeData::default());
    let (app, closed) = probe_application(data.clone());
    let handle = app.start().await.unwrap();
    let service = data
        .service
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .acquire()
        .unwrap();
    let scheduler = data.tasks.lock().unwrap().as_ref().unwrap().clone();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let task = scheduler
        .spawn_blocking_io(move |_| {
            started_tx.send(()).unwrap();
            released.recv().unwrap();
            assert!(!service.load(Ordering::SeqCst));
            Ok(())
        })
        .unwrap();
    started.await.unwrap();
    drop(task);
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(10)
            })
            .await,
        Err(FrameworkError::StopIncomplete {
            phase: Phase::Drain,
            ..
        })
    ));
    assert!(!closed.load(Ordering::SeqCst));
    assert!(!data.torn_down.load(Ordering::SeqCst));
    release.send(()).unwrap();
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(closed.load(Ordering::SeqCst));
}
