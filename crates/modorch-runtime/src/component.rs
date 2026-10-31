use crate::*;
use async_trait::async_trait;

#[derive(Clone)]
pub struct ComponentContext {
    pub component: ComponentRef,
    pub services: ServiceReader,
    pub event_hub: EventHub,
    pub tasks: TaskScheduler,
}
pub type SetupContext = ComponentContext;
pub type RuntimeContext = ComponentContext;
pub type ModuleContext = ComponentContext;
pub struct ShutdownContext {
    pub component: ComponentRef,
    pub phase: Phase,
    pub services: ServiceReader,
}

#[async_trait]
pub trait Lifecycle: Send {
    async fn setup(&mut self, _ctx: &SetupContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// Must return promptly. Submit long-running work to ctx.tasks.
    async fn start(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
    async fn ready(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
    async fn begin_shutdown(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
    async fn quiesce(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
    /// On error the resource remains owned; retries call only unfinished hooks.
    async fn teardown(&mut self, _ctx: &ShutdownContext) -> FrameworkResult<()> {
        Ok(())
    }
}
#[async_trait]
pub trait Initiator: Lifecycle {
    fn descriptor(&self) -> ComponentDescriptor;
    fn export_services(&self, _publisher: &mut ServicePublisher) -> FrameworkResult<()> {
        Ok(())
    }
    fn event_endpoint(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(None)
    }
    async fn activate_ingress(&mut self, _ctx: &RuntimeContext) -> FrameworkResult<()> {
        Ok(())
    }
}
pub trait Module: Lifecycle {
    fn descriptor(&self) -> ComponentDescriptor;
    /// One component-owned execution policy, with any number of subscriptions.
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(None)
    }
}
pub trait InitiatorFactory: Send + Sync {
    fn descriptor(&self) -> ComponentDescriptor;
    fn validate_config(&self, _config: &ComponentConfig) -> FrameworkResult<()> {
        Ok(())
    }
    fn create(&self, config: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>>;
}
pub trait ModuleFactory: Send + Sync {
    fn descriptor(&self) -> ComponentDescriptor;
    fn validate_config(&self, _config: &ComponentConfig) -> FrameworkResult<()> {
        Ok(())
    }
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
