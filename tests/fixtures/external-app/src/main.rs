use modorch::{prelude::*, *};
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = modorch::config::CompositionConfig::parse(
        "[application]\nname='external-consumer'\n[modules.external]\nenabled=true",
    )?;
    let mut builder = ApplicationBuilder::new(&config.application.name)
        .selection(config.selection()?)
        .task_limits(1, 1)
        .execution_resources(ExecutionResources {
            cpu_threads: 1,
            cpu_capacity: 1,
            blocking_io_threads: 1,
            blocking_io_capacity: 1,
            max_tasks_per_owner: 1,
            ..ExecutionResources::default()
        })
        .queue_capacities(
            config.messaging.default_queue_capacity,
            config.messaging.per_shard_capacity,
        );
    builder.register_module(ExternalFactory)?;
    let application = builder.build()?;
    let handle = application.start().await?;
    let metrics = modorch::observe::ApplicationMetrics::capture(&handle);
    assert_eq!(metrics.0.application_id, handle.id());
    assert_eq!(metrics.0.tasks.tasks, 0);
    assert_eq!(metrics.0.tasks.cpu.completed, 1);
    assert_eq!(metrics.0.tasks.blocking_io.completed, 1);
    assert_eq!(metrics.0.tasks.available_task_slots, 1);
    // Idle Owner quota entries disappear once no scheduler or task retains them.
    assert!(metrics.0.tasks.owner_resources.is_empty());
    handle
        .shutdown_checked(ShutdownOptions {
            timeout: config.grace_period(),
        })
        .await?;
    Ok(())
}

struct ExternalFactory;
impl ModuleFactory for ExternalFactory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("external"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(ExternalModule))
    }
}
struct ExternalModule;
#[async_trait]
impl Lifecycle for ExternalModule {
    async fn ready(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        let timeout = std::time::Duration::from_secs(1);
        let cpu = ctx.tasks.spawn_cpu_wait(|_| Ok(7usize), timeout).await?;
        assert_eq!(cpu.wait_finished().await?, 7);
        let io = ctx
            .tasks
            .spawn_blocking_io_wait(|_| Ok(9usize), timeout)
            .await?;
        assert_eq!(io.wait_finished().await?, 9);
        ctx.tasks.spawn(|_| async { Ok(()) })?.wait_finished().await
    }
}
impl Module for ExternalModule {
    fn descriptor(&self) -> ComponentDescriptor {
        ExternalFactory.descriptor()
    }
}
