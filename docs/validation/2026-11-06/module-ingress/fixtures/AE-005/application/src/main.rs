use modorch::{prelude::*, *};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
struct Factory {
    ticks: Arc<AtomicUsize>,
    changed: Arc<Notify>,
    once: Arc<Notify>,
}
struct Worker {
    ticks: Arc<AtomicUsize>,
    changed: Arc<Notify>,
    once: Arc<Notify>,
    timer: Option<TimerHandle>,
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("worker"))
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Worker {
            ticks: self.ticks.clone(),
            changed: self.changed.clone(),
            once: self.once.clone(),
            timer: None,
        }))
    }
}
#[async_trait]
impl Lifecycle for Worker {
    async fn start(&mut self, ctx: &RuntimeContext) -> FrameworkResult<()> {
        let once = self.once.clone();
        ctx.tasks.delay(
            Duration::from_millis(1),
            Arc::new(move |_| {
                let once = once.clone();
                Box::pin(async move {
                    once.notify_one();
                    Ok(())
                })
            }),
        )?;
        let ticks = self.ticks.clone();
        let changed = self.changed.clone();
        self.timer = Some(ctx.tasks.interval(
            IntervalSpec {
                period: Duration::from_millis(5),
                policy: TickPolicy::SkipIfRunning,
            },
            Arc::new(move |_| {
                let ticks = ticks.clone();
                let changed = changed.clone();
                Box::pin(async move {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    changed.notify_one();
                    Ok(())
                })
            }),
        )?);
        Ok(())
    }
    async fn quiesce(&mut self, _: &ShutdownContext) -> FrameworkResult<()> {
        if let Some(timer) = self.timer.take() {
            timer.cancel();
            timer.wait().await;
        }
        Ok(())
    }
}
impl Module for Worker {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(ComponentRef::module("worker"))
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let ticks = Arc::new(AtomicUsize::new(0));
    let changed = Arc::new(Notify::new());
    let once = Arc::new(Notify::new());
    let mut builder = ApplicationBuilder::new("scheduled-tasks");
    builder.register_module(Factory {
        ticks: ticks.clone(),
        changed: changed.clone(),
        once: once.clone(),
    })?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("worker");
    let handle = builder.selection(selection).build()?.start().await?;
    tokio::time::timeout(Duration::from_secs(1), async {
        once.notified().await;
        while ticks.load(Ordering::SeqCst) < 2 {
            changed.notified().await;
        }
    })
    .await?;
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    assert_eq!(handle.task_metrics().tasks, 0);
    assert_eq!(handle.task_metrics().timers, 0);
    assert!(ticks.load(Ordering::SeqCst) >= 2);
    println!("scheduled-tasks: once completed, ticks>=2, drained");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run().await
}
#[tokio::test]
async fn timers_stop_and_dispatched_tasks_drain() {
    run().await.unwrap();
}
