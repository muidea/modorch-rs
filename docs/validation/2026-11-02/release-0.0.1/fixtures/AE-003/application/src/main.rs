use modorch::{prelude::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
const TOPIC: &str = "configuration/changed/v1";
struct Changed {
    revision: usize,
}
struct ConsumerFactory {
    id: &'static str,
    subscription: u64,
    processed: Arc<AtomicUsize>,
}
struct Consumer {
    id: &'static str,
    subscription: u64,
    processed: Arc<AtomicUsize>,
}
impl ModuleFactory for ConsumerFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module(self.id))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Consumer {
            id: self.id,
            subscription: self.subscription,
            processed: self.processed.clone(),
        }))
    }
}
#[async_trait]
impl Lifecycle for Consumer {}
impl Module for Consumer {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module(self.id))
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(Counter(self.processed.clone())),
            subscriptions: vec![SubscriptionSpec::new(
                self.subscription,
                ComponentRef::module(self.id),
                TOPIC,
            )],
        }))
    }
}
struct Counter(Arc<AtomicUsize>);
#[async_trait]
impl SerialHandler for Counter {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        let revision = message.payload::<Changed>()?.revision;
        assert_eq!(revision, 7);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(()))
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let processed = Arc::new(AtomicUsize::new(0));
    let mut builder = ApplicationBuilder::new("event-messaging").queue_capacities(2, 2);
    builder
        .register_module(ConsumerFactory {
            id: "first",
            subscription: 1,
            processed: processed.clone(),
        })?
        .register_module(ConsumerFactory {
            id: "second",
            subscription: 2,
            processed: processed.clone(),
        })?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("first").enable_module("second");
    let handle = builder.selection(selection).build()?.start().await?;
    let report = handle.event_hub().external().publish(
        TOPIC,
        Destination::Broadcast,
        Changed { revision: 7 },
        Default::default(),
        PublishPolicy::Reject,
    )?;
    assert_eq!(report.accepted(), 2);
    assert_eq!(report.rejected(), 0);
    // Only the checked drain establishes that accepted work finished.
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    assert_eq!(processed.load(Ordering::SeqCst), 2);
    println!("event-messaging: accepted=2, processed=2");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn fanout_finishes_before_successful_shutdown() {
    run().await.unwrap();
}
