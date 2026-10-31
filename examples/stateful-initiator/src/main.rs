use modorch::{prelude::*, *};
use std::sync::Arc;

struct SharedConfig {
    greeting: String,
}
impl SharedConfig {
    fn greeting(&self) -> &str {
        &self.greeting
    }
}
struct ConfigInitiator;
#[async_trait]
impl Lifecycle for ConfigInitiator {}
#[async_trait]
impl Initiator for ConfigInitiator {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::initiator("config"))
    }
    fn export_services(&self, publisher: &mut ServicePublisher) -> FrameworkResult<()> {
        publisher.publish(
            "snapshot",
            Arc::new(SharedConfig {
                greeting: "hello from a leased service".into(),
            }),
        )
    }
}
struct ConfigFactory;
impl InitiatorFactory for ConfigFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        ConfigInitiator.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(Box::new(ConfigInitiator))
    }
}
struct Reader {
    config: Option<ServiceHandle<SharedConfig>>,
}
#[async_trait]
impl Lifecycle for Reader {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        self.config = Some(
            ctx.services
                .get(&ComponentRef::initiator("config"), "snapshot")?,
        );
        Ok(())
    }
    async fn start(&mut self, _: &RuntimeContext) -> FrameworkResult<()> {
        let lease = self.config.as_ref().unwrap().acquire()?;
        println!("{}", lease.greeting());
        Ok(())
    }
}
impl Module for Reader {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module("reader"));
        d.requires.push(ComponentRef::initiator("config"));
        d
    }
}
struct ReaderFactory;
impl ModuleFactory for ReaderFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        Reader { config: None }.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Reader { config: None }))
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = ApplicationBuilder::new("shared-config");
    builder.register_initiator(ConfigFactory)?;
    builder.register_module(ReaderFactory)?;
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("config").enable_module("reader");
    let handle = builder.selection(selection).build()?.start().await?;
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    Ok(())
}
