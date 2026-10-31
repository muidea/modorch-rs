#![cfg(feature = "runtime")]
use modorch::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
#[derive(Default)]
struct Shared {
    generations: AtomicUsize,
    log: Mutex<Vec<String>>,
    tasks: Mutex<Vec<TaskScheduler>>,
    services: Mutex<Vec<ServiceHandle<AtomicBool>>>,
    old_closed: Mutex<Vec<Arc<AtomicBool>>>,
    handler_gate: Mutex<Option<Arc<Semaphore>>>,
    fail_setup_until: AtomicUsize,
}
#[derive(Clone)]
struct Factory {
    id: &'static str,
    kind: ComponentKind,
    policy: FailurePolicy,
    depends: Vec<ComponentRef>,
    shared: Arc<Shared>,
}
impl Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef {
            kind: self.kind,
            id: self.id.into(),
        });
        d.requires = self.depends.clone();
        d.failure = self.policy.clone();
        d
    }
    fn create(&self) -> Box<Component> {
        let index = if self.id == "db" {
            self.shared.generations.fetch_add(1, Ordering::SeqCst)
        } else {
            self.shared
                .generations
                .load(Ordering::SeqCst)
                .saturating_sub(1)
        };
        Box::new(Component {
            factory: self.clone(),
            index,
            closed: Arc::new(AtomicBool::new(false)),
        })
    }
}
impl InitiatorFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(self.create())
    }
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(self.create())
    }
}
struct Component {
    factory: Factory,
    index: usize,
    closed: Arc<AtomicBool>,
}
impl Component {
    fn log(&self, phase: &str) {
        self.factory
            .shared
            .log
            .lock()
            .unwrap()
            .push(format!("{}:{}:{}", self.factory.id, self.index, phase));
    }
}
#[async_trait]
impl Lifecycle for Component {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        self.log("setup");
        self.factory
            .shared
            .tasks
            .lock()
            .unwrap()
            .push(ctx.tasks.clone());
        if self.factory.id == "db" {
            self.factory
                .shared
                .old_closed
                .lock()
                .unwrap()
                .push(self.closed.clone());
            if self.index > 0
                && self.index <= self.factory.shared.fail_setup_until.load(Ordering::SeqCst)
            {
                return Err(FrameworkError::InvalidConfig(
                    "injected partial replacement setup".into(),
                ));
            }
        }
        if self.factory.id == "worker" {
            self.factory
                .shared
                .services
                .lock()
                .unwrap()
                .push(ctx.services.get(&ComponentRef::initiator("db"), "db")?);
        }
        Ok(())
    }
    async fn begin_shutdown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.log("begin");
        Ok(())
    }
    async fn quiesce(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.log("quiesce");
        Ok(())
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.log("teardown");
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
#[async_trait]
impl Initiator for Component {
    fn descriptor(&self) -> ComponentDescriptor {
        self.factory.descriptor()
    }
    fn export_services(&self, p: &mut ServicePublisher) -> FrameworkResult<()> {
        p.publish("db", self.closed.clone())
    }
    fn event_endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        self.endpoint()
    }
}
impl Module for Component {
    fn descriptor(&self) -> ComponentDescriptor {
        self.factory.descriptor()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        self.endpoint()
    }
}
impl Component {
    fn endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(Handler {
                generation: self.index,
                shared: self.factory.shared.clone(),
            }),
            subscriptions: vec![SubscriptionSpec::new(
                match self.factory.id {
                    "db" => 1,
                    "worker" => 2,
                    _ => 3,
                },
                self.factory.descriptor().component(),
                format!("{}/v1", self.factory.id),
            )],
        }))
    }
}
#[derive(Clone, Copy)]
enum Work {
    Panic,
    Echo,
    Hold,
}
struct Handler {
    generation: usize,
    shared: Arc<Shared>,
}
#[async_trait]
impl SerialHandler for Handler {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        msg: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        match *msg.payload::<Work>()? {
            Work::Panic => panic!("injected component fault"),
            Work::Echo => {}
            Work::Hold => {
                let gate = self.shared.handler_gate.lock().unwrap().clone().unwrap();
                gate.acquire().await.unwrap().forget();
            }
        }
        Ok(Arc::new(self.generation))
    }
}
async fn start(policy: FailurePolicy, shared: Arc<Shared>) -> ApplicationHandle {
    let mut builder = ApplicationBuilder::new("recovery");
    builder
        .register_initiator(Factory {
            id: "db",
            kind: ComponentKind::Initiator,
            policy,
            depends: vec![],
            shared: shared.clone(),
        })
        .unwrap();
    builder
        .register_module(Factory {
            id: "worker",
            kind: ComponentKind::Module,
            policy: FailurePolicy::FailFast,
            depends: vec![ComponentRef::initiator("db")],
            shared: shared.clone(),
        })
        .unwrap();
    builder
        .register_module(Factory {
            id: "independent",
            kind: ComponentKind::Module,
            policy: FailurePolicy::FailFast,
            depends: vec![],
            shared,
        })
        .unwrap();
    let mut selection = ComponentSelection::new();
    selection
        .enable_initiator("db")
        .enable_module("worker")
        .enable_module("independent");
    builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .unwrap()
}
fn destination(id: &str) -> Destination {
    Destination::Component(if id == "db" {
        ComponentRef::initiator(id)
    } else {
        ComponentRef::module(id)
    })
}
async fn fault(handle: &ApplicationHandle) {
    let result = handle
        .event_hub()
        .request::<_, usize>("db/v1", destination("db"), Work::Panic, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await;
    assert!(matches!(
        result,
        Err(FrameworkError::ExecutionFailed { .. })
    ));
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn echo(handle: &ApplicationHandle, id: &str) -> usize {
    *handle
        .event_hub()
        .request::<_, usize>(
            &format!("{id}/v1"),
            destination(id),
            Work::Echo,
            Default::default(),
        )
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap()
}
#[tokio::test]
async fn stop_dependents_removes_only_the_dependency_closure() {
    let shared = Arc::new(Shared::default());
    let handle = start(FailurePolicy::StopDependents, shared.clone()).await;
    fault(&handle).await;
    until(|| handle.health().state == ApplicationState::Degraded).await;
    assert_eq!(echo(&handle, "independent").await, 0);
    assert!(!handle.health().ready);
    assert!(matches!(
        handle.event_hub().command(
            "worker/v1",
            destination("worker"),
            Work::Echo,
            Default::default()
        ),
        Err(FrameworkError::Unroutable(_))
    ));
    let log = shared.log.lock().unwrap().clone();
    let stops = log
        .iter()
        .filter(|s| s.ends_with("teardown"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(stops, vec!["worker:0:teardown", "db:0:teardown"]);
    assert_eq!(
        handle.health().components[&ComponentRef::module("worker")].state,
        ComponentState::Stopped
    );
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert_eq!(
        shared
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_str() == "db:0:teardown")
            .count(),
        1
    );
}
#[tokio::test]
async fn restart_waits_for_old_handlers_and_leases_and_invalidates_old_handles() {
    let shared = Arc::new(Shared::default());
    let gate = Arc::new(Semaphore::new(0));
    *shared.handler_gate.lock().unwrap() = Some(gate.clone());
    let handle = start(
        FailurePolicy::Restart {
            max_attempts: 2,
            backoff: Duration::from_millis(1),
        },
        shared.clone(),
    )
    .await;
    let old_task = shared.tasks.lock().unwrap()[0].clone();
    let service = shared.services.lock().unwrap()[0].clone();
    let lease = service.acquire().unwrap();
    let hold = handle
        .event_hub()
        .request::<_, usize>(
            "worker/v1",
            destination("worker"),
            Work::Hold,
            Default::default(),
        )
        .unwrap();
    tokio::task::yield_now().await;
    fault(&handle).await;
    until(|| handle.health().state == ApplicationState::Recovering).await;
    assert_eq!(shared.generations.load(Ordering::SeqCst), 1);
    assert!(!shared.old_closed.lock().unwrap()[0].load(Ordering::SeqCst));
    assert!(service.acquire().is_err());
    assert_eq!(echo(&handle, "independent").await, 0);
    gate.add_permits(1);
    assert_eq!(*hold.wait(Duration::from_secs(1)).await.unwrap(), 0);
    assert_eq!(shared.generations.load(Ordering::SeqCst), 1);
    drop(lease);
    until(|| {
        handle.health().ready
            && handle.health().components[&ComponentRef::initiator("db")].generation == 1
    })
    .await;
    assert_eq!(echo(&handle, "worker").await, 1);
    assert_eq!(echo(&handle, "independent").await, 0);
    assert!(matches!(
        old_task.spawn(|_| async { Ok(()) }),
        Err(FrameworkError::Closed)
    ));
    assert!(service.acquire().is_err());
    assert!(
        shared
            .services
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .acquire()
            .is_ok()
    );
    let log = shared.log.lock().unwrap().clone();
    assert!(
        log.iter().position(|s| s == "db:0:teardown").unwrap()
            < log.iter().position(|s| s == "db:1:setup").unwrap()
    );
    handle.shutdown_checked(Default::default()).await.unwrap();
}
#[tokio::test]
async fn replacement_setup_failure_is_cleaned_before_retry_and_budget_is_bounded() {
    let shared = Arc::new(Shared::default());
    shared.fail_setup_until.store(1, Ordering::SeqCst);
    let handle = start(
        FailurePolicy::Restart {
            max_attempts: 2,
            backoff: Duration::from_millis(1),
        },
        shared.clone(),
    )
    .await;
    fault(&handle).await;
    until(|| handle.health().ready && shared.generations.load(Ordering::SeqCst) == 3).await;
    assert_eq!(echo(&handle, "db").await, 2);
    let log = shared.log.lock().unwrap().clone();
    assert!(
        log.iter().position(|s| s == "db:1:teardown").unwrap()
            < log.iter().position(|s| s == "db:2:setup").unwrap()
    );
    fault(&handle).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), handle.wait())
            .await
            .unwrap()
            .is_err()
    );
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert_eq!(shared.generations.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn shutdown_interrupts_restart_backoff_without_spawning_a_new_generation() {
    let shared = Arc::new(Shared::default());
    let handle = start(
        FailurePolicy::Restart {
            max_attempts: 2,
            backoff: Duration::from_secs(3600),
        },
        shared.clone(),
    )
    .await;
    fault(&handle).await;
    until(|| handle.health().components[&ComponentRef::initiator("db")].restarts == 1).await;
    handle
        .shutdown_checked(ShutdownOptions {
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(shared.generations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn old_generation_faults_are_not_replayed_against_the_replacement() {
    let shared = Arc::new(Shared::default());
    let handle = start(
        FailurePolicy::Restart {
            max_attempts: 3,
            backoff: Duration::from_millis(1),
        },
        shared.clone(),
    )
    .await;
    for _ in 0..3 {
        handle
            .event_hub()
            .command("db/v1", destination("db"), Work::Panic, Default::default())
            .unwrap();
    }
    until(|| {
        handle.health().ready
            && handle.health().components[&ComponentRef::initiator("db")].generation == 1
    })
    .await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(shared.generations.load(Ordering::SeqCst), 2);
    assert_eq!(
        handle.health().components[&ComponentRef::initiator("db")].restarts,
        1
    );
    handle.shutdown_checked(Default::default()).await.unwrap();
}
#[tokio::test]
async fn a_noncooperative_old_task_keeps_resources_alive_until_it_really_finishes() {
    let shared = Arc::new(Shared::default());
    let handle = start(
        FailurePolicy::Restart {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
        shared.clone(),
    )
    .await;
    let gate = Arc::new(Semaphore::new(0));
    let wait = gate.clone();
    let task = shared.tasks.lock().unwrap()[0]
        .spawn(move |_| async move {
            wait.acquire().await.unwrap().forget();
            Ok(())
        })
        .unwrap();
    fault(&handle).await;
    until(|| handle.health().state == ApplicationState::Recovering).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(shared.generations.load(Ordering::SeqCst), 1);
    assert!(!shared.old_closed.lock().unwrap()[0].load(Ordering::SeqCst));
    gate.add_permits(1);
    task.wait().await.unwrap();
    until(|| handle.health().ready && shared.generations.load(Ordering::SeqCst) == 2).await;
    handle.shutdown_checked(Default::default()).await.unwrap();
}
