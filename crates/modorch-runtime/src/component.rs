use crate::*;
use async_trait::async_trait;

#[derive(Clone)]
/// Owner-bound runtime capabilities supplied to component lifecycle hooks.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentContext {
    /// Component associated with this boundary; absent before routing when optional.
    pub component: ComponentRef,
    /// Generation- and dependency-scoped service lookup capability.
    pub services: ServiceReader,
    /// Causal message capability for trusted component child operations.
    pub event_hub: EventHub,
    /// Current admitted tasks or their snapshot, including jobs still releasing resources.
    pub tasks: TaskScheduler,
}
/// ComponentContext used during resource initialization.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type SetupContext = ComponentContext;
/// ComponentContext used during start, readiness and ingress activation.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type RuntimeContext = ComponentContext;
/// ComponentContext alias for module lifecycle integration.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type ModuleContext = ComponentContext;
/// Services and phase available to an owned component shutdown hook.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ShutdownContext {
    /// Component associated with this boundary; absent before routing when optional.
    pub component: ComponentRef,
    /// Lifecycle/execution phase associated with this boundary.
    pub phase: Phase,
    /// Generation- and dependency-scoped service lookup capability.
    pub services: ServiceReader,
}

// async-trait adds a bare #[must_use] to the already must-use boxed Future.
#[allow(clippy::double_must_use)]
#[async_trait]
/// Fallible, retry-aware component hooks supervised by the application.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait Lifecycle: Send {
    /// Initialize owned resources; partial failure still enters compensation (LIFE-002).
    async fn setup(&mut self, _ctx: &SetupContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// Must return promptly. Submit long-running work to ctx.tasks.
    async fn start(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// Await the component readiness barrier before any ingress activates (LIFE-001).
    async fn ready(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// Stop new production without releasing dependencies prematurely (LIFE-003).
    async fn begin_shutdown(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// Await producer quiescence; timeout does not cancel the owned hook (LIFE-003).
    async fn quiesce(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// On error the resource remains owned; retries call only unfinished hooks.
    async fn teardown(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
}
// Keep this workaround scoped to the macro-generated trait methods.
#[allow(clippy::double_must_use)]
#[async_trait]
/// Lifecycle component providing narrow shared services and optional ingress or handlers.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait Initiator: Lifecycle {
    /// Return stable metadata; created instances must match their registered factory descriptor.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn descriptor(&self) -> ComponentDescriptor;
    /// Publish narrow services after successful setup; duplicate typed keys reject startup.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn export_services(&self, _publisher: &mut ServicePublisher) -> FrameworkResult<()> {
        Ok(())
    }
    /// Optionally return one Initiator-owned handler registration with multiple subscriptions.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn event_endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(None)
    }
    /// Activate ingress after all ready hooks passed (LIFE-INGRESS-001).
    ///
    /// Return promptly; run listeners through `ctx.tasks` and submit external
    /// messages through `ctx.event_hub.external()`. Do not listen before this
    /// hook. Partial activation must support checked shutdown compensation.
    async fn activate_ingress(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
}
#[allow(clippy::double_must_use)]
#[async_trait]
/// Lifecycle component encapsulating business behavior, private state and optional ingress.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait Module: Lifecycle {
    /// Return stable metadata; created instances must match their registered factory descriptor.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn descriptor(&self) -> ComponentDescriptor;
    /// One component-owned execution policy, with any number of subscriptions.
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(None)
    }
    /// Activate ingress after all ready hooks passed (LIFE-INGRESS-001).
    ///
    /// Defaults to no action. Return promptly; use `ctx.tasks` for listeners
    /// and `ctx.event_hub.external()` for incoming messages. Bind an unlistened
    /// socket during setup if needed, and listen only here. Partial activation
    /// is compensated through begin_shutdown, quiesce, drain and teardown.
    /// The same barrier and compensation apply during supervised reconstruction.
    async fn activate_ingress(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
}
/// Side-effect-free metadata/config validation and creation of selected Initiators.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait InitiatorFactory: Send + Sync {
    /// Return stable metadata; created instances must match their registered factory descriptor.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn descriptor(&self) -> ComponentDescriptor;
    /// Validate owner-defined configuration before any selected factory is invoked.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn validate_config(&self, _config: &ComponentConfig) -> FrameworkResult<()> {
        Ok(())
    }
    /// Create a fresh instance without starting background work; resource setup belongs to Lifecycle::setup.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn create(&self, config: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>>;
}
/// Side-effect-free metadata/config validation and creation of selected Modules.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub trait ModuleFactory: Send + Sync {
    /// Return stable metadata; created instances must match their registered factory descriptor.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn descriptor(&self) -> ComponentDescriptor;
    /// Validate owner-defined configuration before any selected factory is invoked.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn validate_config(&self, _config: &ComponentConfig) -> FrameworkResult<()> {
        Ok(())
    }
    /// Create a fresh instance without starting background work; resource setup belongs to Lifecycle::setup.
    ///
    /// Contract: LIFE-002 / LIFE-003. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    fn create(&self, config: &ComponentConfig) -> FrameworkResult<Box<dyn Module>>;
}
pub(crate) enum Factory {
    Initiator(Box<dyn InitiatorFactory>),
    Module(Box<dyn ModuleFactory>),
}
impl Factory {
    pub fn descriptor(&self) -> ComponentDescriptor {
        match self {
            Self::Initiator(f) => f.descriptor(),
            Self::Module(f) => f.descriptor(),
        }
    }
    pub fn validate(&self, config: &ComponentConfig) -> FrameworkResult<()> {
        match self {
            Self::Initiator(f) => f.validate_config(config),
            Self::Module(f) => f.validate_config(config),
        }
    }
    pub fn create(&self, config: &ComponentConfig) -> FrameworkResult<Instance> {
        match self {
            Self::Initiator(f) => f.create(config).map(Instance::Initiator),
            Self::Module(f) => f.create(config).map(Instance::Module),
        }
    }
}
pub(crate) enum Instance {
    Initiator(Box<dyn Initiator>),
    Module(Box<dyn Module>),
}
impl Instance {
    pub async fn activate_ingress(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.activate_ingress(ctx).await,
            Self::Module(m) => m.activate_ingress(ctx).await,
        }
    }
    pub fn lifecycle(&mut self) -> &mut dyn Lifecycle {
        self
    }
    pub fn descriptor(&self) -> ComponentDescriptor {
        match self {
            Self::Initiator(i) => i.descriptor(),
            Self::Module(m) => m.descriptor(),
        }
    }
    pub fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        match self {
            Self::Initiator(i) => i.event_endpoint(),
            Self::Module(m) => m.handlers(),
        }
    }
}

#[async_trait]
impl Lifecycle for Instance {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.setup(ctx).await,
            Self::Module(m) => m.setup(ctx).await,
        }
    }
    async fn start(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.start(ctx).await,
            Self::Module(m) => m.start(ctx).await,
        }
    }
    async fn ready(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.ready(ctx).await,
            Self::Module(m) => m.ready(ctx).await,
        }
    }
    async fn begin_shutdown(&mut self, ctx: &ShutdownContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.begin_shutdown(ctx).await,
            Self::Module(m) => m.begin_shutdown(ctx).await,
        }
    }
    async fn quiesce(&mut self, ctx: &ShutdownContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.quiesce(ctx).await,
            Self::Module(m) => m.quiesce(ctx).await,
        }
    }
    async fn teardown(&mut self, ctx: &ShutdownContext) -> FrameworkResult<()> {
        match self {
            Self::Initiator(i) => i.teardown(ctx).await,
            Self::Module(m) => m.teardown(ctx).await,
        }
    }
}
