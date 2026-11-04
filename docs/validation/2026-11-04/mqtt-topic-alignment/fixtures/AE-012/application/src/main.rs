use modorch::{prelude::*, *};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
struct State {
    started: Notify,
    release: Notify,
    closed: AtomicBool,
}
struct Factory(Arc<State>);
struct Worker(Arc<State>);
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("worker"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Worker(self.0.clone())))
    }
}
#[async_trait]
impl Lifecycle for Worker {
    async fn start(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        let state = self.0.clone();
        ctx.tasks.spawn(move |_| async move {
            state.started.notify_one();
            state.release.notified().await;
            Ok(())
        })?;
        Ok(())
    }
    async fn teardown(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        self.0.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
impl Module for Worker {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("worker"))
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(State {
        started: Notify::new(),
        release: Notify::new(),
        closed: AtomicBool::new(false),
    });
    let mut builder = ApplicationBuilder::new("shutdown-timeout");
    builder.register_module(Factory(state.clone()))?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("worker");
    let handle = builder.selection(selection).build()?.start().await?;
    tokio::time::timeout(Duration::from_secs(5), state.started.notified()).await?;
    assert!(matches!(
        handle
            .shutdown_checked(ShutdownOptions {
                timeout: Duration::from_millis(1)
            })
            .await,
        Err(FrameworkError::StopIncomplete { .. })
    ));
    assert!(!state.closed.load(Ordering::SeqCst));
    assert!(handle.health().pending.operations > 0);
    state.release.notify_one();
    assert_eq!(
        handle.shutdown_checked(ShutdownOptions::graceful()).await?,
        ShutdownReceipt::GracefullyStopped
    );
    assert!(state.closed.load(Ordering::SeqCst));
    assert_eq!(handle.health().pending.operations, 0);
    println!("shutdown: timeout retained resources, retry drained");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn shutdown_timeout_keeps_the_owner_until_actual_exit() {
    run().await.unwrap();
}
