use modorch::{prelude::*, *};
use std::{sync::Arc, time::Duration};

// A real application would put these owner-defined DTOs in a contract crate.
mod contract {
    pub const CREATE: &str = "order/create/v1";
    pub const SAVE: &str = "storage/save/v1";
    pub struct CreateOrder {
        pub id: String,
    }
    pub struct OrderCreated {
        pub count: usize,
    }
}
use contract::*;
struct BusinessFactory {
    storage: bool,
}
impl ModuleFactory for BusinessFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module(if self.storage {
            "storage"
        } else {
            "order"
        }));
        if !self.storage {
            d.requires.push(ComponentRef::module("storage"));
        }
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(BusinessModule {
            d: self.descriptor(),
            storage: self.storage,
        }))
    }
}
struct BusinessModule {
    d: ComponentDescriptor,
    storage: bool,
}
#[async_trait]
impl Lifecycle for BusinessModule {}
impl Module for BusinessModule {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: if self.storage {
                Box::new(Storage { orders: vec![] })
            } else {
                Box::new(Order)
            },
            subscriptions: vec![SubscriptionSpec::new(
                if self.storage { 1 } else { 2 },
                self.d.component(),
                if self.storage { SAVE } else { CREATE },
            )],
        }))
    }
}
struct Storage {
    orders: Vec<String>,
}
#[async_trait]
impl SerialHandler for Storage {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.orders
            .push(message.payload::<CreateOrder>()?.id.clone());
        Ok(Arc::new(OrderCreated {
            count: self.orders.len(),
        }))
    }
}
struct Order;
#[async_trait]
impl SerialHandler for Order {
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        let input = message.payload::<CreateOrder>()?;
        let result = ctx
            .event_hub
            .request::<_, OrderCreated>(
                SAVE,
                Destination::Component(ComponentRef::module("storage")),
                CreateOrder {
                    id: input.id.clone(),
                },
                Default::default(),
            )?
            .wait(Duration::from_secs(1))
            .await?;
        Ok(result)
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = ApplicationBuilder::new("orders");
    builder
        .register_module(BusinessFactory { storage: true })?
        .register_module(BusinessFactory { storage: false })?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("order").enable_module("storage");
    let handle = builder.selection(selection).build()?.start().await?;
    let result = handle
        .event_hub()
        .request::<_, OrderCreated>(
            CREATE,
            Destination::Component(ComponentRef::module("order")),
            CreateOrder {
                id: "order-1".into(),
            },
            Default::default(),
        )?
        .wait(Duration::from_secs(1))
        .await;
    let shutdown = handle.shutdown_checked(ShutdownOptions::graceful()).await;
    println!("stored orders: {}", result?.count);
    shutdown?;
    Ok(())
}
