//! Host-owned HTTP ingress. This small example deliberately serves one bounded
//! connection at a time; applications can replace it with their HTTP library.
use modorch::{prelude::*, *};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
struct HttpFactory {
    address: Arc<Mutex<Option<SocketAddr>>>,
}
impl InitiatorFactory for HttpFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::initiator("http"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Initiator>> {
        Ok(Box::new(Http {
            listener: None,
            address: self.address.clone(),
        }))
    }
}
struct Http {
    listener: Option<TcpListener>,
    address: Arc<Mutex<Option<SocketAddr>>>,
}
fn io_error(error: std::io::Error) -> FrameworkError {
    FrameworkError::InvalidConfig(error.to_string())
}
#[async_trait]
impl Lifecycle for Http {
    async fn setup(&mut self, _: &SetupContext) -> FrameworkResult<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        *self.address.lock().unwrap() = Some(listener.local_addr().map_err(io_error)?);
        self.listener = Some(listener);
        Ok(())
    }
}
#[async_trait]
impl Initiator for Http {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::initiator("http"))
    }
    async fn activate_ingress(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        let listener = self.listener.take().unwrap();
        let external = ctx.event_hub.external();
        ctx.tasks.spawn(move |cancel| async move {
            loop {
                let (mut stream, _) = tokio::select! { biased; _ = cancel.cancelled() => break, result = listener.accept() => result.map_err(io_error)? };
                // Parse with an idle bound, then admit each business request as a fresh root.
                let request = async {
                    let mut bytes = [0u8; 1024]; stream.read(&mut bytes).await.map_err(io_error)?;
                    let result = match external.request::<_, String>("http/ready/v1", Destination::Component(ComponentRef::module("health")), (), Default::default()) {
                        Ok(handle) => handle.wait(Duration::from_secs(1)).await,
                        Err(error) => Err(error),
                    };
                    let (status, body) = match result { Ok(body) => ("200 OK", (*body).clone()), Err(_) => ("503 Service Unavailable", "stopping".into()) };
                    let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.map_err(io_error)?; Ok::<_, FrameworkError>(())
                };
                let _ = tokio::time::timeout(Duration::from_secs(2), request).await;
            }
            Ok(())
        })?;
        Ok(())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = Arc::new(Mutex::new(None));
    let mut builder = ApplicationBuilder::new("http");
    builder.register_initiator(HttpFactory {
        address: address.clone(),
    })?;
    builder.register_module(HealthFactory)?;
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("http").enable_module("health");
    let handle = builder.selection(selection).build()?.start().await?;
    let listen_address = address.lock().unwrap().unwrap();
    println!("http://{listen_address}");
    // --once makes the example self-checking in CI; signals belong to the host.
    let run_result: Result<(), Box<dyn std::error::Error>> =
        if std::env::args().any(|a| a == "--once") {
            async {
                let mut stream = tokio::net::TcpStream::connect(listen_address).await?;
                stream
                    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .await?;
                let mut response = String::new();
                stream.read_to_string(&mut response).await?;
                assert!(response.ends_with("ready"));
                Ok(())
            }
            .await
        } else {
            tokio::signal::ctrl_c().await.map_err(Into::into)
        };
    let shutdown = handle.shutdown_checked(ShutdownOptions::graceful()).await;
    run_result?;
    shutdown?;
    Ok(())
}

struct HealthFactory;
impl ModuleFactory for HealthFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module("health"));
        d.requires.push(ComponentRef::initiator("http"));
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(HealthModule))
    }
}
struct HealthModule;
#[async_trait]
impl Lifecycle for HealthModule {}
impl Module for HealthModule {
    fn descriptor(&self) -> ComponentDescriptor {
        HealthFactory.descriptor()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Serial {
            handler: Box::new(HealthHandler),
            subscriptions: vec![SubscriptionSpec::new(
                1,
                self.descriptor().component(),
                "http/ready/v1",
            )],
        }))
    }
}
struct HealthHandler;
#[async_trait]
impl SerialHandler for HealthHandler {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        Ok(Arc::new(String::from("ready")))
    }
}
