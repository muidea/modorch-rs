use modorch::{prelude::*, *};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;
const TOPIC: &str = "calculator/double/v1";
struct Double {
    value: usize,
    slow: bool,
}
struct Factory {
    started: Arc<Notify>,
    release: Arc<Notify>,
}
struct Calculator {
    started: Arc<Notify>,
    release: Arc<Notify>,
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("calculator"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Calculator {
            started: self.started.clone(),
            release: self.release.clone(),
        }))
    }
}
#[async_trait]
impl Lifecycle for Calculator {}
impl Module for Calculator {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("calculator"))
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(Calculator {
                started: self.started.clone(),
                release: self.release.clone(),
            }),
            subscriptions: vec![SubscriptionSpec::new(
                1,
                self.descriptor().component(),
                TOPIC,
            )],
        }))
    }
}
#[async_trait]
impl SerialHandler for Calculator {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        let input = message.payload::<Double>()?;
        if input.slow {
            self.started.notify_one();
            self.release.notified().await;
        }
        Ok(Arc::new(input.value * 2))
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut builder = ApplicationBuilder::new("request-response");
    builder.register_module(Factory {
        started: started.clone(),
        release: release.clone(),
    })?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("calculator");
    let handle = builder.selection(selection).build()?.start().await?;
    let hub = handle.event_hub().external();
    let destination = Destination::Component(ComponentRef::module("calculator"));
    let answer = hub
        .request::<_, usize>(
            TOPIC,
            destination.clone(),
            Double {
                value: 21,
                slow: false,
            },
            Default::default(),
        )?
        .wait(Duration::from_secs(1))
        .await?;
    assert_eq!(*answer, 42);
    let slow = hub.request::<_, usize>(
        TOPIC,
        destination,
        Double {
            value: 1,
            slow: true,
        },
        Default::default(),
    )?;
    tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
    let observation = slow.observation();
    assert!(matches!(
        slow.wait(Duration::from_millis(1)).await,
        Err(FrameworkError::Indeterminate(_))
    ));
    release.notify_one();
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    assert_eq!(
        observation.status(),
        RequestStatus::CompletedAfterCallerTimeout
    );
    println!("request-response: completed=42, timeout=Indeterminate");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn response_and_running_timeout_are_distinct() {
    run().await.unwrap();
}
