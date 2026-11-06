use modorch::{prelude::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Config {
    released: Arc<AtomicBool>,
}
impl Config {
    fn greeting(&self) -> &str {
        "hello from a leased service"
    }
}
impl Drop for Config {
    fn drop(&mut self) {
        self.released.store(true, Ordering::SeqCst);
    }
}
struct ConfigOwner {
    config: Option<Arc<Config>>,
}
struct ConfigFactory {
    released: Arc<AtomicBool>,
}
impl InitiatorFactory for ConfigFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::initiator("config"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(Box::new(ConfigOwner {
            config: Some(Arc::new(Config {
                released: self.released.clone(),
            })),
        }))
    }
}
#[async_trait]
impl Lifecycle for ConfigOwner {
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.config.take();
        Ok(())
    }
}
impl Initiator for ConfigOwner {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::initiator("config"))
    }
    fn export_services(&self, publisher: &mut ServicePublisher) -> FrameworkResult<()> {
        publisher.publish("snapshot", self.config.as_ref().unwrap().clone())
    }
}
struct Reader;
struct ReaderFactory;
impl ModuleFactory for ReaderFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        Reader.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Reader))
    }
}
#[async_trait]
impl Lifecycle for Reader {
    async fn start(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        let handle = ctx
            .services
            .get::<Config>(&ComponentRef::initiator("config"), "snapshot")?;
        let lease = handle.acquire()?;
        assert_eq!(lease.greeting(), "hello from a leased service");
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
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let released = Arc::new(AtomicBool::new(false));
    let mut builder = ApplicationBuilder::new("shared-service");
    builder
        .register_initiator(ConfigFactory {
            released: released.clone(),
        })?
        .register_module(ReaderFactory)?;
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("config").enable_module("reader");
    let handle = builder.selection(selection).build()?.start().await?;
    assert!(!released.load(Ordering::SeqCst));
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    assert!(released.load(Ordering::SeqCst));
    println!("shared-service: released=true");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn service_is_read_and_released_after_dependents() {
    run().await.unwrap();
}
