use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Supervised component lifecycle state within an application generation.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum ComponentState {
    /// Running start and readiness hooks.
    Starting,
    /// Setup created partial state before failing; compensation is required.
    PartiallyInitialized,
    /// All relevant readiness barriers passed.
    Ready,
    /// Releasing resources in reverse dependency order.
    Stopping,
    /// Irreversible graceful terminal state.
    Stopped,
    /// Old generation drained; rebuilding under the cumulative attempt budget.
    Restarting,
    /// A supervised operation or lifecycle boundary failed.
    Failed,
}
#[derive(Clone, Debug)]
/// Latest component state, generation, cumulative restart count and failure.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentHealth {
    /// Current lifecycle state, as indicated by the enclosing type.
    pub state: ComponentState,
    /// Component generation used to reject stale capabilities.
    pub generation: u64,
    /// Cumulative rebuild attempts for this component.
    pub restarts: u32,
    /// Latest failure retained independently of journal eviction.
    pub last_failure: Option<FrameworkError>,
}
impl ApplicationHandle {
    pub(crate) async fn supervise(&self) {
        let mut state = self.inner.state.subscribe();
        loop {
            if matches!(
                *state.borrow(),
                ApplicationState::Stopped | ApplicationState::Forced
            ) {
                return;
            }
            let event = tokio::select! { event=self.inner.failures.next()=>event, _=state.changed()=>continue };
            let Some(owner) = event.component.as_ref() else {
                self.fail_fast(event.error).await;
                return;
            };
            if !self.inner.tracker.generation_valid(owner, event.generation) {
                self.inner.failures.acknowledge();
                continue;
            }
            let status = self
                .inner
                .component_health
                .lock()
                .unwrap()
                .get(owner)
                .cloned();
            if status.is_none_or(|s| s.state == ComponentState::Stopped) {
                self.inner.failures.acknowledge();
                continue;
            }
            let policy = self
                .inner
                .descriptors
                .get(owner)
                .map(|d| d.failure.clone())
                .unwrap_or_default();
            if self.inner.tracker.is_closed() {
                self.retain_until_stopped().await;
                return;
            }
            if policy == FailurePolicy::FailFast {
                self.fail_fast(event.error).await;
                return;
            }
            let result = match AssertUnwindSafe(self.recover(owner, &policy, event.error.clone()))
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(_) => Err(FrameworkError::ExecutionFailed {
                    component: owner.clone(),
                    operation_id: self.inner.tracker.current().map(|lease| lease.id()),
                    phase: Phase::Setup,
                    detail: "recovery hook panicked".into(),
                    source: None,
                }),
            };
            if let Err(error) = result {
                if self.inner.tracker.is_closed() {
                    self.retain_until_stopped().await;
                    return;
                }
                self.fail_fast(error).await;
                return;
            }
            self.inner.failures.acknowledge();
        }
    }
    async fn fail_fast(&self, error: FrameworkError) {
        self.inner.terminal_failure.send_replace(Some(error));
        self.begin_shutdown();
        let _ = self.shutdown_checked(ShutdownOptions::graceful()).await;
        self.retain_until_stopped().await;
    }
    fn closure(&self, owner: &ComponentRef) -> BTreeSet<ComponentRef> {
        let mut owners = BTreeSet::from([owner.clone()]);
        loop {
            let before = owners.len();
            for (component, descriptor) in &self.inner.descriptors {
                if !self
                    .inner
                    .component_health
                    .lock()
                    .unwrap()
                    .contains_key(component)
                {
                    continue;
                }
                if descriptor
                    .requires
                    .iter()
                    .chain(&descriptor.optional)
                    .any(|dependency| owners.contains(dependency))
                {
                    owners.insert(component.clone());
                }
            }
            if before == owners.len() {
                return owners;
            }
        }
    }
    async fn recover(
        &self,
        owner: &ComponentRef,
        policy: &FailurePolicy,
        failure: FrameworkError,
    ) -> FrameworkResult<()> {
        // This lock is shared with full shutdown, so recovery and teardown never overlap.
        let _transaction = self.inner.shutdown.lock().await;
        if self.inner.tracker.is_closed() {
            return Err(FrameworkError::Closed);
        }
        let owners = self.closure(owner);
        {
            let mut health = self.inner.component_health.lock().unwrap();
            for component in &owners {
                let status = health.get_mut(component).unwrap();
                status.state = ComponentState::Stopping;
                status.last_failure = Some(failure.clone());
            }
        }
        self.inner.set_state(ApplicationState::Recovering);
        let operation = self.inner.tracker.close_owners(&owners);
        self.inner.scheduler.stop_owners(&owners);
        let mut entries = self.inner.entries.lock().await;
        for phase in [Phase::BeginShutdown, Phase::Quiesce] {
            for entry in entries
                .iter_mut()
                .filter(|e| owners.contains(&e.descriptor.component()) && !e.teardown_done)
            {
                let component = entry.descriptor.component();
                let ctx = self.inner.stop_context(&entry.descriptor, phase);
                let hook = async {
                    match phase {
                        Phase::BeginShutdown => {
                            entry.instance.lifecycle().begin_shutdown(&ctx).await
                        }
                        _ => entry.instance.lifecycle().quiesce(&ctx).await,
                    }
                };
                let result = self
                    .inner
                    .tracker
                    .scope(
                        operation.clone(),
                        checked_hook(&self.inner.tracker, &component, phase, hook),
                    )
                    .await;
                self.inner.record(component, phase, result.clone());
                result?;
                if phase == Phase::BeginShutdown {
                    entry.begin_done = true;
                } else {
                    entry.quiesce_done = true;
                }
            }
        }
        drop(operation);
        let drain = self.inner.drain_metrics.begin();
        self.inner.tracker.drain_owners(&owners).await;
        drain.complete();
        // Remove and join old handlers before releasing any owner resources.
        self.inner.hub.remove_owners(&owners).await;
        for entry in entries
            .iter_mut()
            .rev()
            .filter(|e| owners.contains(&e.descriptor.component()) && !e.teardown_done)
        {
            let component = entry.descriptor.component();
            self.inner.services.retire(&component);
            let ctx = self.inner.stop_context(&entry.descriptor, Phase::Teardown);
            let result = checked_hook(
                &self.inner.tracker,
                &component,
                Phase::Teardown,
                entry.instance.lifecycle().teardown(&ctx),
            )
            .await;
            self.inner
                .record(component.clone(), Phase::Teardown, result.clone());
            result?;
            entry.teardown_done = true;
            self.inner.services.remove(&component);
            self.inner
                .component_health
                .lock()
                .unwrap()
                .get_mut(&component)
                .unwrap()
                .state = ComponentState::Stopped;
        }
        let FailurePolicy::Restart {
            max_attempts,
            backoff,
        } = policy
        else {
            self.inner.set_state(ApplicationState::Degraded);
            return Ok(());
        };
        let mut last_error = failure;
        loop {
            let attempts = self.inner.component_health.lock().unwrap()[owner].restarts;
            if attempts >= *max_attempts {
                return Err(last_error);
            }
            {
                let mut health = self.inner.component_health.lock().unwrap();
                health.get_mut(owner).unwrap().restarts += 1;
                for component in &owners {
                    health.get_mut(component).unwrap().state = ComponentState::Restarting;
                }
            }
            self.backoff(*backoff).await?;
            self.inner.tracker.advance_owners(&owners)?;
            let boot = self.inner.tracker.close_owners(&owners);
            let result = self
                .inner
                .tracker
                .scope(boot.clone(), self.rebuild(&owners, &mut entries))
                .await;
            drop(boot);
            match result {
                Ok(()) => {
                    self.inner.tracker.open_owners(&owners)?;
                    for entry in entries
                        .iter_mut()
                        .filter(|e| owners.contains(&e.descriptor.component()))
                    {
                        let component = entry.descriptor.component();
                        let ctx = self.inner.context(&entry.descriptor);
                        if let Instance::Initiator(initiator) = &mut entry.instance {
                            checked_hook(
                                &self.inner.tracker,
                                &component,
                                Phase::ActivateIngress,
                                initiator.activate_ingress(&ctx),
                            )
                            .await?;
                        }
                        let mut health = self.inner.component_health.lock().unwrap();
                        let status = health.get_mut(&component).unwrap();
                        status.state = ComponentState::Ready;
                        status.generation = self.inner.tracker.generation(&component);
                    }
                    let degraded = self
                        .inner
                        .component_health
                        .lock()
                        .unwrap()
                        .values()
                        .any(|s| s.state == ComponentState::Stopped);
                    self.inner.set_state(if degraded {
                        ApplicationState::Degraded
                    } else {
                        ApplicationState::Ready
                    });
                    return Ok(());
                }
                Err(error) => {
                    // Partial reconstruction goes through the same checked barriers
                    // as the original generation before another attempt is allowed.
                    last_error = error;
                    let cleanup = self.inner.tracker.close_owners(&owners);
                    self.inner.scheduler.stop_owners(&owners);
                    for phase in [Phase::BeginShutdown, Phase::Quiesce] {
                        for entry in entries.iter_mut().filter(|e| {
                            owners.contains(&e.descriptor.component()) && !e.teardown_done
                        }) {
                            let c = entry.descriptor.component();
                            let ctx = self.inner.stop_context(&entry.descriptor, phase);
                            let result = self
                                .inner
                                .tracker
                                .scope(
                                    cleanup.clone(),
                                    checked_hook(&self.inner.tracker, &c, phase, async {
                                        if phase == Phase::BeginShutdown {
                                            entry.instance.lifecycle().begin_shutdown(&ctx).await
                                        } else {
                                            entry.instance.lifecycle().quiesce(&ctx).await
                                        }
                                    }),
                                )
                                .await;
                            self.inner.record(c, phase, result.clone());
                            result?;
                            if phase == Phase::BeginShutdown {
                                entry.begin_done = true
                            } else {
                                entry.quiesce_done = true
                            }
                        }
                    }
                    drop(cleanup);
                    let drain = self.inner.drain_metrics.begin();
                    self.inner.tracker.drain_owners(&owners).await;
                    drain.complete();
                    self.inner.hub.remove_owners(&owners).await;
                    for entry in entries
                        .iter_mut()
                        .rev()
                        .filter(|e| owners.contains(&e.descriptor.component()) && !e.teardown_done)
                    {
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
                        result?;
                        entry.teardown_done = true;
                        self.inner.services.remove(&c);
                    }
                }
            }
        }
    }
    async fn backoff(&self, delay: Duration) -> FrameworkResult<()> {
        let mut state = self.inner.state.subscribe();
        let stop = async {
            loop {
                if self.inner.tracker.is_closed() {
                    return;
                }
                let _ = state.changed().await;
            }
        };
        tokio::select! {_=tokio::time::sleep(delay)=>Ok(()),_=stop=>Err(FrameworkError::Closed)}
    }
    async fn rebuild(
        &self,
        owners: &BTreeSet<ComponentRef>,
        entries: &mut [Entry],
    ) -> FrameworkResult<()> {
        for entry in entries
            .iter_mut()
            .filter(|e| owners.contains(&e.descriptor.component()))
        {
            let c = entry.descriptor.component();
            let config = self.inner.selection.config(&c);
            let instance = std::panic::catch_unwind(AssertUnwindSafe(|| {
                self.inner.factories[&c].create(&config)
            }))
            .map_err(|_| FrameworkError::ExecutionFailed {
                component: c.clone(),
                operation_id: self.inner.tracker.current().map(|lease| lease.id()),
                phase: Phase::Setup,
                detail: "restart factory panicked".into(),
                source: None,
            })?
            .map_err(|error| {
                error.in_execution(
                    c.clone(),
                    self.inner.tracker.current().map(|lease| lease.id()),
                    Phase::Setup,
                )
            })?;
            entry.instance = instance;
            entry.begin_done = false;
            entry.quiesce_done = false;
            entry.teardown_done = false;
            if entry.instance.descriptor() != entry.descriptor {
                return Err(FrameworkError::InvalidConfig(
                    "replacement descriptor changed".into(),
                ));
            }
            let ctx = self.inner.context(&entry.descriptor);
            let result = checked_hook(
                &self.inner.tracker,
                &c,
                Phase::Setup,
                entry.instance.lifecycle().setup(&ctx),
            )
            .await;
            self.inner.record(c.clone(), Phase::Setup, result.clone());
            result?;
            let extra = std::panic::catch_unwind(AssertUnwindSafe(|| -> FrameworkResult<()> {
                if let Instance::Initiator(initiator) = &entry.instance {
                    initiator.export_services(&mut self.inner.services.publisher(c.clone())?)?;
                }
                if let Some(registration) = entry.instance.handlers()? {
                    let mut descriptor = entry.descriptor.clone();
                    if let Some((component, shard)) = self.inner.queue_capacities {
                        descriptor.queue_capacity = if matches!(
                            descriptor.execution,
                            ExecutionPolicy::Partitioned { .. }
                        ) {
                            shard
                        } else {
                            component
                        };
                    }
                    self.inner.hub.install(&descriptor, registration)?;
                }
                Ok(())
            }))
            .map_err(|_| FrameworkError::ExecutionFailed {
                component: c.clone(),
                operation_id: self.inner.tracker.current().map(|lease| lease.id()),
                phase: Phase::Setup,
                detail: "restart endpoint/export panicked".into(),
                source: None,
            })?;
            extra.map_err(|error| {
                error.in_execution(
                    c.clone(),
                    self.inner.tracker.current().map(|lease| lease.id()),
                    Phase::Setup,
                )
            })?;
        }
        for phase in [Phase::Start, Phase::Ready] {
            for entry in entries
                .iter_mut()
                .filter(|e| owners.contains(&e.descriptor.component()))
            {
                let c = entry.descriptor.component();
                let ctx = self.inner.context(&entry.descriptor);
                let result = checked_hook(&self.inner.tracker, &c, phase, async {
                    if phase == Phase::Start {
                        entry.instance.lifecycle().start(&ctx).await
                    } else {
                        entry.instance.lifecycle().ready(&ctx).await
                    }
                })
                .await;
                self.inner.record(c, phase, result.clone());
                result?;
            }
        }
        Ok(())
    }
}
