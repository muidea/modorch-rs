use modorch::{prelude::*, *};
use std::sync::{Arc, Mutex};
type Log = Arc<Mutex<Vec<&'static str>>>;
struct Factory(Log);
struct Broken(Log);
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("broken"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Broken(self.0.clone())))
    }
}
#[async_trait]
impl Lifecycle for Broken {
    async fn setup(&mut self, _: &SetupContext) -> FrameworkResult<()> {
        self.0.lock().unwrap().push("setup");
        Err(FrameworkError::InvalidConfig(
            "injected setup failure".into(),
        ))
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.0.lock().unwrap().push("teardown");
        Ok(())
    }
}
impl Module for Broken {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("broken"))
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let log = Log::default();
    let mut builder = ApplicationBuilder::new("lifecycle");
    builder.register_module(Factory(log.clone()))?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("broken");
    let error = builder
        .selection(selection)
        .build()?
        .start()
        .await
        .err()
        .expect("injected setup must fail");
    assert!(
        matches!(error.source.root_cause(),FrameworkError::InvalidConfig(message) if message=="injected setup failure")
    );
    assert!(matches!(
        error.cleanup,
        Ok(ShutdownReceipt::GracefullyStopped)
    ));
    assert_eq!(*log.lock().unwrap(), ["setup", "teardown"]);
    assert_eq!(error.handle.health().pending.operations, 0);
    println!("lifecycle: setup failed, compensated, primary error retained");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn setup_failure_runs_compensation() {
    run().await.unwrap();
}
