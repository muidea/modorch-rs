#![cfg(feature = "runtime")]
use modorch::{prelude::*, *};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::{
    Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*, registry::LookupSpan};

#[derive(Clone, Debug)]
struct SpanRecord {
    id: Id,
    parent: Option<Id>,
    name: &'static str,
    fields: BTreeMap<String, String>,
}
#[derive(Default)]
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
}
#[derive(Clone, Default)]
struct Capture {
    spans: Arc<Mutex<Vec<SpanRecord>>>,
    events: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Capture {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        let parent = attrs.parent().cloned().or_else(|| {
            if attrs.is_contextual() {
                ctx.current_span().id().cloned()
            } else {
                None
            }
        });
        self.spans.lock().unwrap().push(SpanRecord {
            id: id.clone(),
            parent,
            name: attrs.metadata().name(),
            fields: fields.0,
        });
    }
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.events.lock().unwrap().push(fields.0);
    }
}
struct Factory;
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module("trace"));
        d.execution = ExecutionPolicy::Concurrent {
            max_in_flight: std::num::NonZeroUsize::new(4).unwrap(),
        };
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(TraceModule { tasks: None }))
    }
}
struct TraceModule {
    tasks: Option<TaskScheduler>,
}
#[async_trait]
impl Lifecycle for TraceModule {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        self.tasks = Some(ctx.tasks.clone());
        Ok(())
    }
}
impl Module for TraceModule {
    fn descriptor(&self) -> ComponentDescriptor {
        Factory.descriptor()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(Some(HandlerRegistration::Concurrent {
            max_in_flight: std::num::NonZeroUsize::new(4).unwrap(),
            handler: Arc::new(TraceHandler(self.tasks.clone().unwrap())),
            subscriptions: vec![SubscriptionSpec::new(
                1,
                self.descriptor().component(),
                "work/#",
            )],
        }))
    }
}
struct TraceHandler(TaskScheduler);
#[async_trait]
impl ConcurrentHandler for TraceHandler {
    async fn handle(
        &self,
        ctx: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        if message.topic == "work/parent" {
            let hub = ctx.event_hub.clone();
            // A supervised task carries the Handler's causal trace across spawn.
            self.0
                .spawn(move |_| async move {
                    hub.request::<_, usize>(
                        "work/child",
                        Destination::Broadcast,
                        (),
                        Default::default(),
                    )?
                    .wait(Duration::from_secs(1))
                    .await?;
                    // An external root must break the producer's trace ancestry too.
                    hub.external()
                        .request::<_, usize>(
                            "work/external",
                            Destination::Broadcast,
                            (),
                            Default::default(),
                        )?
                        .wait(Duration::from_secs(1))
                        .await?;
                    Ok(())
                })?
                .wait()
                .await?;
        }
        Ok(Arc::new(42usize))
    }
}
#[tokio::test]
async fn traces_preserve_task_ancestry_separate_external_roots_and_log_shutdown_outcomes() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    // Host-owned, thread-local Subscriber; no global Subscriber is installed.
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let mut builder = ApplicationBuilder::new("traces");
    builder.register_module(Factory).unwrap();
    let mut selection = ComponentSelection::new();
    selection.enable_module("trace");
    let handle = builder
        .selection(selection)
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let request = handle
        .event_hub()
        .request::<_, usize>(
            "work/parent",
            Destination::Broadcast,
            (),
            Default::default(),
        )
        .unwrap();
    let root_id = request.operation_id();
    assert_eq!(*request.wait(Duration::from_secs(1)).await.unwrap(), 42);
    handle.shutdown_checked(Default::default()).await.unwrap();
    let forced = ApplicationBuilder::new("forced")
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    forced.force_shutdown().await;
    let spans = capture.spans.lock().unwrap();
    let message = |topic| {
        spans
            .iter()
            .find(|span| {
                span.name == "message" && span.fields.get("topic").is_some_and(|v| v == topic)
            })
            .unwrap()
    };
    let parent = message("work/parent");
    let child = message("work/child");
    let external = message("work/external");
    assert!(parent.parent.is_none());
    assert!(external.parent.is_none());
    assert_eq!(parent.fields["operation_id"], root_id.to_string());
    assert_eq!(parent.fields["application_id"], handle.id().to_string());
    let task = spans.iter().find(|span| span.name == "task").unwrap();
    let mut ancestor = child.parent.clone();
    let mut path = Vec::new();
    while let Some(id) = ancestor {
        let span = spans.iter().find(|span| span.id == id).unwrap();
        path.push(span.id.clone());
        ancestor = span.parent.clone();
    }
    assert!(path.contains(&task.id));
    assert!(path.contains(&parent.id));
    let events = capture.events.lock().unwrap();
    assert!(events.iter().any(
        |event| event.get("outcome").is_some_and(|v| v == "graceful")
            && event.get("application_id") == Some(&handle.id().to_string())
    ));
    assert!(
        events
            .iter()
            .any(|event| event.get("outcome").is_some_and(|v| v == "forced")
                && event.get("application_id") == Some(&forced.id().to_string()))
    );
}
