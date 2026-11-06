#![cfg(feature = "runtime")]
//! Real TCP tests for both component roles and checked ingress ownership.
use modorch::*;
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
    sync::Semaphore,
};

#[derive(Default)]
struct State {
    creates: AtomicUsize,
    addresses: Mutex<Vec<SocketAddr>>,
    log: Mutex<Vec<(usize, &'static str)>>,
    tasks: Mutex<Vec<TaskScheduler>>,
    hubs: Mutex<Vec<ExternalEventHub>>,
    // Test-only gates deliberately preserve ownership across cancelled callers.
    ready_gate: Mutex<Option<(usize, Arc<Semaphore>)>>,
    activate_gate: Mutex<Option<Arc<Semaphore>>>,
    connection_gate: Mutex<Option<Arc<Semaphore>>>,
    fail_activate: Mutex<Vec<usize>>,
    panic_activate: bool,
    bind_on_activate: Option<SocketAddr>,
    connections: AtomicUsize,
    rejected: AtomicUsize,
}
impl State {
    fn address(&self) -> SocketAddr {
        *self.addresses.lock().unwrap().last().unwrap()
    }
    fn has(&self, generation: usize, phase: &'static str) -> bool {
        self.log.lock().unwrap().contains(&(generation, phase))
    }
}
#[derive(Clone)]
struct Factory {
    descriptor: ComponentDescriptor,
    state: Arc<State>,
    subscription: u64,
}
impl Factory {
    fn module(id: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            subscription: NEXT.fetch_add(1, Ordering::Relaxed),
            descriptor: ComponentDescriptor::new(ComponentRef::module(id)),
            state: Arc::default(),
        }
    }
    fn make(&self) -> Box<Ingress> {
        Box::new(Ingress {
            factory: self.clone(),
            generation: self.state.creates.fetch_add(1, Ordering::SeqCst),
            socket: None,
            listener: None,
        })
    }
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor.clone()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(self.make())
    }
}
impl InitiatorFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor.clone()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(self.make())
    }
}
struct Ingress {
    factory: Factory,
    generation: usize,
    socket: Option<TcpSocket>,
    listener: Option<TaskHandle>,
}
fn io_error(error: std::io::Error) -> FrameworkError {
    FrameworkError::InvalidConfig(error.to_string())
}
impl Ingress {
    fn log(&self, phase: &'static str) {
        self.factory
            .state
            .log
            .lock()
            .unwrap()
            .push((self.generation, phase));
    }
    fn endpoint(&self) -> HandlerRegistration {
        HandlerRegistration::Serial {
            handler: Box::new(Echo(self.generation)),
            subscriptions: vec![SubscriptionSpec::new(
                self.factory.subscription,
                self.factory.descriptor.component(),
                "ingress/echo",
            )],
        }
    }
    async fn activate(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        self.log("activate");
        let gate = self.factory.state.activate_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        if self
            .factory
            .state
            .fail_activate
            .lock()
            .unwrap()
            .contains(&self.generation)
        {
            assert!(
                !self.factory.state.panic_activate,
                "injected activation panic"
            );
            return Err(FrameworkError::InvalidConfig(
                "injected activation failure".into(),
            ));
        }
        if let Some(address) = self.factory.state.bind_on_activate {
            self.socket
                .as_ref()
                .unwrap()
                .bind(address)
                .map_err(io_error)?;
        }
        let listener = self.socket.take().unwrap().listen(16).map_err(io_error)?;
        let tasks = ctx.tasks.clone();
        let external = ctx.event_hub.external();
        let destination = Destination::Component(ctx.component.clone());
        let state = self.factory.state.clone();
        self.listener = Some(ctx.tasks.spawn(move |cancel| async move {
            loop {
                let (mut stream, _) = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    result = listener.accept() => result.map_err(io_error)?,
                };
                let external = external.clone();
                let destination = destination.clone();
                let connection_state = state.clone();
                // A rejected spawn drops the unadmitted stream; the listener stays alive.
                if let Err(error) = tasks.spawn(move |cancel| async move {
                    connection_state.connections.fetch_add(1, Ordering::SeqCst);
                    let gate = connection_state.connection_gate.lock().unwrap().clone();
                    if let Some(gate) = gate {
                        gate.acquire().await.unwrap().forget();
                    }
                    let work = async {
                        stream.read_u8().await.map_err(io_error)?;
                        let reply = external
                            .request::<_, usize>(
                                "ingress/echo",
                                destination,
                                (),
                                Default::default(),
                            )?
                            .wait(Duration::from_secs(1))
                            .await?;
                        stream.write_u8(*reply as u8).await.map_err(io_error)?;
                        Ok(())
                    };
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => Ok(()),
                        result = work => result,
                    }
                }) {
                    assert!(matches!(
                        error,
                        FrameworkError::Backpressure(_)
                            | FrameworkError::ResourceLimit(_)
                            | FrameworkError::Closed
                    ));
                    state.rejected.fetch_add(1, Ordering::SeqCst);
                }
            }
            Ok(())
        })?);
        self.log("activated");
        Ok(())
    }
}
#[async_trait]
impl Lifecycle for Ingress {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        let socket = TcpSocket::new_v4().map_err(io_error)?;
        socket.set_reuseaddr(true).map_err(io_error)?;
        let address = self
            .factory
            .state
            .addresses
            .lock()
            .unwrap()
            .first()
            .copied()
            .unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
        let address = if let Some(address) = self.factory.state.bind_on_activate {
            address
        } else {
            socket.bind(address).map_err(io_error)?;
            socket.local_addr().map_err(io_error)?
        };
        self.factory.state.addresses.lock().unwrap().push(address);
        self.socket = Some(socket);
        self.factory
            .state
            .tasks
            .lock()
            .unwrap()
            .push(ctx.tasks.clone());
        self.factory
            .state
            .hubs
            .lock()
            .unwrap()
            .push(ctx.event_hub.external());
        self.log("setup");
        Ok(())
    }
    async fn ready(&mut self, _: &RuntimeContext) -> FrameworkResult<()> {
        self.log("ready");
        let gate = self.factory.state.ready_gate.lock().unwrap().clone();
        if let Some((generation, gate)) = gate {
            if generation == self.generation {
                gate.acquire().await.unwrap().forget();
            }
        }
        Ok(())
    }
    async fn begin_shutdown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        if let Some(listener) = &self.listener {
            listener.cancel();
        }
        self.log("begin");
        Ok(())
    }
    async fn quiesce(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        if let Some(listener) = &self.listener {
            listener.wait_finished().await?;
        }
        self.log("quiesce");
        Ok(())
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.socket.take();
        self.listener.take();
        self.log("teardown");
        Ok(())
    }
}
#[async_trait]
impl Module for Ingress {
    fn descriptor(&self) -> ComponentDescriptor {
        self.factory.descriptor.clone()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(self.endpoint()))
    }
    async fn activate_ingress(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        self.activate(ctx).await
    }
}
#[async_trait]
impl Initiator for Ingress {
    fn descriptor(&self) -> ComponentDescriptor {
        self.factory.descriptor.clone()
    }
    fn event_endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(self.endpoint()))
    }
    async fn activate_ingress(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        self.activate(ctx).await
    }
}
struct Echo(usize);
#[async_trait]
impl SerialHandler for Echo {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        Ok(Arc::new(self.0))
    }
}
fn build(factories: &[Factory], task_limit: usize) -> Application {
    let mut builder = ApplicationBuilder::new("ingress").task_limits(task_limit, 16);
    let mut selection = ComponentSelection::new();
    for factory in factories {
        match factory.descriptor.kind {
            ComponentKind::Initiator => {
                builder.register_initiator(factory.clone()).unwrap();
                selection.enable_initiator(&factory.descriptor.id);
            }
            ComponentKind::Module => {
                builder.register_module(factory.clone()).unwrap();
                selection.enable_module(&factory.descriptor.id);
            }
        }
    }
    builder.selection(selection).build().unwrap()
}
async fn until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition did not become true");
}
async fn refuses(address: SocketAddr) {
    assert!(
        tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(address))
            .await
            .unwrap()
            .is_err()
    );
}
async fn echo(address: SocketAddr) -> u8 {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_u8(0).await.unwrap();
        stream.read_u8().await.unwrap()
    })
    .await
    .unwrap()
}
async fn fail(state: &State) {
    let task = state
        .tasks
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone()
        .spawn(|_| async {
            Err(FrameworkError::InvalidConfig(
                "injected listener failure".into(),
            ))
        })
        .unwrap();
    assert!(task.wait_finished().await.is_err());
}
fn restart_pair(max_attempts: u32) -> (Factory, Factory) {
    let mut first = Factory::module("a");
    first.descriptor.failure = FailurePolicy::Restart {
        max_attempts,
        backoff: Duration::ZERO,
    };
    let mut second = Factory::module("b");
    second
        .descriptor
        .requires
        .push(first.descriptor.component());
    (first, second)
}

#[tokio::test]
async fn both_roles_wait_for_all_ready_before_tcp_listen_and_build_has_no_side_effects() {
    let mut first = Factory::module("init");
    first.descriptor = ComponentDescriptor::new(ComponentRef::initiator("init"));
    let second = Factory::module("mqtt");
    let blocker = Factory::module("z_blocker");
    let gate = Arc::new(Semaphore::new(0));
    *blocker.state.ready_gate.lock().unwrap() = Some((0, gate.clone()));
    let app = build(&[first.clone(), second.clone(), blocker.clone()], 16);
    for factory in [&first, &second, &blocker] {
        assert_eq!(factory.state.creates.load(Ordering::SeqCst), 0);
        assert!(factory.state.addresses.lock().unwrap().is_empty());
    }
    let start = tokio::spawn(app.start());
    until(|| blocker.state.has(0, "ready")).await;
    for factory in [&first, &second, &blocker] {
        assert!(!factory.state.has(0, "activate"));
        refuses(factory.state.address()).await;
    }
    gate.add_permits(1);
    let handle = start.await.unwrap().unwrap();
    assert!(handle.health().ready);
    assert_eq!(echo(first.state.address()).await, 0);
    assert_eq!(echo(second.state.address()).await, 0);
    handle.shutdown_checked(Default::default()).await.unwrap();
}

#[tokio::test]
async fn partial_activation_error_and_panic_preserve_cause_and_release_all_ports() {
    for panic_activate in [false, true] {
        let first = Factory::module("a");
        let mut second = Factory::module("b");
        second.state = Arc::new(State {
            panic_activate,
            ..Default::default()
        });
        second.state.fail_activate.lock().unwrap().push(0);
        let error = build(&[first.clone(), second.clone()], 16)
            .start()
            .await
            .err()
            .unwrap();
        assert_eq!(error.source.phase(), Some(Phase::ActivateIngress));
        assert_eq!(
            error.source.component(),
            Some(&second.descriptor.component())
        );
        if !panic_activate {
            assert!(
                matches!(error.source.root_cause(), FrameworkError::InvalidConfig(message) if message == "injected activation failure")
            );
            assert!(std::error::Error::source(error.source.as_ref()).is_some());
        }
        assert!(error.cleanup.is_ok());
        assert!(first.state.has(0, "activated"));
        for factory in [&first, &second] {
            assert!(factory.state.has(0, "teardown"));
            refuses(factory.state.address()).await;
        }
    }
}

#[tokio::test]
async fn cancelled_start_wait_still_compensates_activated_module() {
    let first = Factory::module("a");
    let second = Factory::module("b");
    let gate = Arc::new(Semaphore::new(0));
    *second.state.activate_gate.lock().unwrap() = Some(gate.clone());
    let start = tokio::spawn(build(&[first.clone(), second.clone()], 16).start());
    until(|| second.state.has(0, "activate")).await;
    assert_eq!(echo(first.state.address()).await, 0);
    start.abort();
    assert!(start.await.err().unwrap().is_cancelled());
    gate.add_permits(1);
    until(|| first.state.has(0, "teardown") && second.state.has(0, "teardown")).await;
    refuses(first.state.address()).await;
    refuses(second.state.address()).await;
}

#[tokio::test]
async fn slow_connection_retains_resources_until_checked_shutdown_can_finish() {
    let factory = Factory::module("mqtt");
    let gate = Arc::new(Semaphore::new(0));
    *factory.state.connection_gate.lock().unwrap() = Some(gate.clone());
    let handle = build(std::slice::from_ref(&factory), 16)
        .start()
        .await
        .unwrap();
    let _connection = TcpStream::connect(factory.state.address()).await.unwrap();
    until(|| factory.state.connections.load(Ordering::SeqCst) == 1).await;
    let result = handle
        .shutdown_checked(ShutdownOptions {
            timeout: Duration::from_millis(10),
        })
        .await;
    assert!(matches!(result, Err(FrameworkError::StopIncomplete { .. })));
    assert!(!factory.state.has(0, "teardown"));
    refuses(factory.state.address()).await;
    gate.add_permits(1);
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(factory.state.has(0, "teardown"));
}

#[tokio::test]
async fn connection_task_limit_rejects_without_losing_listener_or_leaking_work() {
    let factory = Factory::module("mqtt");
    let handle = build(std::slice::from_ref(&factory), 1)
        .start()
        .await
        .unwrap();
    let mut connection = TcpStream::connect(factory.state.address()).await.unwrap();
    until(|| factory.state.rejected.load(Ordering::SeqCst) == 1).await;
    assert_eq!(factory.state.connections.load(Ordering::SeqCst), 0);
    let read = tokio::time::timeout(Duration::from_secs(1), connection.read_u8())
        .await
        .unwrap();
    assert!(read.is_err());
    assert!(handle.health().ready);
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert!(factory.state.has(0, "teardown"));
    refuses(factory.state.address()).await;
}

#[tokio::test]
async fn context_hubs_share_routing_and_different_applications_remain_isolated() {
    let first = Factory::module("mqtt");
    let second = Factory::module("business");
    let other = Factory::module("other_app_only");
    let handle = build(&[first.clone(), second.clone()], 16)
        .start()
        .await
        .unwrap();
    let other_handle = build(std::slice::from_ref(&other), 16)
        .start()
        .await
        .unwrap();
    for (source, target) in [(&first, &second), (&second, &first)] {
        let hub = source.state.hubs.lock().unwrap()[0].clone();
        let response = hub
            .request::<_, usize>(
                "ingress/echo",
                Destination::Component(target.descriptor.component()),
                (),
                Default::default(),
            )
            .unwrap();
        assert_eq!(*response.wait(Duration::from_secs(1)).await.unwrap(), 0);
        assert!(
            hub.request::<_, usize>(
                "ingress/echo",
                Destination::Component(other.descriptor.component()),
                (),
                Default::default()
            )
            .is_err()
        );
    }
    assert_eq!(echo(first.state.address()).await, 0);
    handle.shutdown_checked(Default::default()).await.unwrap();
    other_handle
        .shutdown_checked(Default::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn restart_activation_failure_compensates_before_retry_and_preserves_application_hub() {
    let (first, second) = restart_pair(2);
    second.state.fail_activate.lock().unwrap().push(1);
    let gate = Arc::new(Semaphore::new(0));
    *second.state.ready_gate.lock().unwrap() = Some((1, gate.clone()));
    let handle = build(&[first.clone(), second.clone()], 16)
        .start()
        .await
        .unwrap();
    let old_task = first.state.tasks.lock().unwrap()[0].clone();
    let old_hub = first.state.hubs.lock().unwrap()[0].clone();
    let address = first.state.address();
    let activation_gate = Arc::new(Semaphore::new(0));
    *second.state.activate_gate.lock().unwrap() = Some(activation_gate.clone());
    let connection_gate = Arc::new(Semaphore::new(0));
    *first.state.connection_gate.lock().unwrap() = Some(connection_gate.clone());
    fail(&first.state).await;
    until(|| second.state.has(1, "ready")).await;
    refuses(address).await;
    refuses(second.state.address()).await;
    assert!(!first.state.has(1, "activate"));
    assert!(!handle.health().ready);
    gate.add_permits(1);
    until(|| second.state.has(1, "activate")).await;
    assert_ne!(
        handle.health().components[&first.descriptor.component()].state,
        ComponentState::Ready
    );
    let _connection = TcpStream::connect(address).await.unwrap();
    until(|| first.state.connections.load(Ordering::SeqCst) == 1).await;
    activation_gate.add_permits(1);
    until(|| first.state.has(1, "quiesce")).await;
    assert!(!first.state.has(1, "teardown"));
    assert_eq!(first.state.creates.load(Ordering::SeqCst), 2);
    first.state.connection_gate.lock().unwrap().take();
    connection_gate.add_permits(1);
    activation_gate.add_permits(1);
    until(|| handle.health().ready && first.state.has(2, "activated")).await;
    assert_eq!(echo(address).await, 2);
    assert_eq!(echo(second.state.address()).await, 2);
    assert!(matches!(
        old_task.spawn(|_| async { Ok(()) }),
        Err(FrameworkError::Closed)
    ));
    let response = old_hub
        .request::<_, usize>(
            "ingress/echo",
            Destination::Component(second.descriptor.component()),
            (),
            Default::default(),
        )
        .unwrap();
    assert_eq!(*response.wait(Duration::from_secs(1)).await.unwrap(), 2);
    for factory in [&first, &second] {
        let log = factory.state.log.lock().unwrap();
        assert!(
            log.iter()
                .position(|entry| *entry == (1, "teardown"))
                .unwrap()
                < log.iter().position(|entry| *entry == (2, "setup")).unwrap()
        );
    }
    let failure = handle.health().components[&second.descriptor.component()]
        .last_failure
        .clone()
        .unwrap();
    assert_eq!(failure.phase(), Some(Phase::ActivateIngress));
    assert_eq!(
        handle.health().components[&first.descriptor.component()].restarts,
        2
    );
    handle.shutdown_checked(Default::default()).await.unwrap();
    refuses(address).await;
}

#[tokio::test]
async fn activation_retry_budget_exhaustion_closes_all_ingress_and_reports_activation_phase() {
    let (first, second) = restart_pair(2);
    second.state.fail_activate.lock().unwrap().extend([1, 2]);
    let handle = build(&[first.clone(), second.clone()], 16)
        .start()
        .await
        .unwrap();
    fail(&first.state).await;
    let result = tokio::time::timeout(Duration::from_secs(3), handle.wait())
        .await
        .unwrap();
    let error = result.unwrap_err();
    assert_eq!(error.phase(), Some(Phase::ActivateIngress));
    handle.shutdown_checked(Default::default()).await.unwrap();
    assert_eq!(first.state.creates.load(Ordering::SeqCst), 3);
    for factory in [&first, &second] {
        assert!(factory.state.has(2, "teardown"));
        refuses(factory.state.address()).await;
    }
}

#[tokio::test]
async fn real_bind_failure_in_activation_retains_source_and_compensates_previous_ingress() {
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first = Factory::module("a");
    let mut second = Factory::module("b");
    second.state = Arc::new(State {
        bind_on_activate: Some(occupied.local_addr().unwrap()),
        ..Default::default()
    });
    let error = build(&[first.clone(), second.clone()], 16)
        .start()
        .await
        .err()
        .unwrap();
    assert_eq!(error.source.phase(), Some(Phase::ActivateIngress));
    assert!(std::error::Error::source(error.source.as_ref()).is_some());
    assert!(
        matches!(error.source.root_cause(), FrameworkError::InvalidConfig(message) if !message.is_empty())
    );
    assert!(first.state.has(0, "activated"));
    assert!(first.state.has(0, "teardown"));
    assert!(second.state.has(0, "teardown"));
    assert!(error.cleanup.is_ok());
    refuses(first.state.address()).await;
    // Only the independent test-owned listener remains; cleanup must not touch it.
    let _connection = TcpStream::connect(occupied.local_addr().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn listener_task_quota_failure_compensates_all_partially_activated_sockets() {
    let first = Factory::module("a");
    let second = Factory::module("b");
    let error = build(&[first.clone(), second.clone()], 1)
        .start()
        .await
        .err()
        .unwrap();
    assert_eq!(error.source.phase(), Some(Phase::ActivateIngress));
    assert!(matches!(
        error.source.root_cause(),
        FrameworkError::Backpressure(_)
    ));
    assert!(error.cleanup.is_ok());
    assert!(first.state.has(0, "activated"));
    for factory in [&first, &second] {
        assert!(factory.state.has(0, "teardown"));
        refuses(factory.state.address()).await;
    }
}
