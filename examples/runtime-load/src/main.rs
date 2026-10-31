//! Repeatable bounded-load and soak harness. JSON on stdout, progress on stderr.
use modorch::*;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
struct Factory {
    policy: ExecutionPolicy,
    capacity: usize,
    completed: Arc<AtomicU64>,
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module("worker"));
        d.execution = self.policy.clone();
        d.queue_capacity = self.capacity;
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Worker {
            d: self.descriptor(),
            completed: self.completed.clone(),
        }))
    }
}
struct Worker {
    d: ComponentDescriptor,
    completed: Arc<AtomicU64>,
}
#[async_trait]
impl Lifecycle for Worker {}
impl Module for Worker {
    fn descriptor(&self) -> ComponentDescriptor {
        self.d.clone()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        let subscriptions = vec![SubscriptionSpec::new(1, self.d.component(), "load/echo/v1")];
        let completed = self.completed.clone();
        let factory = move |_| {
            Ok(Box::new(Echo {
                completed: completed.clone(),
            }) as Box<dyn SerialHandler>)
        };
        Ok(Some(match self.d.execution {
            ExecutionPolicy::Serial => HandlerRegistration::Serial {
                handler: factory(0)?,
                subscriptions,
            },
            ExecutionPolicy::Partitioned { shards } => HandlerRegistration::Partitioned {
                shards,
                factory: Box::new(factory),
                subscriptions,
            },
            ExecutionPolicy::Concurrent { max_in_flight } => HandlerRegistration::Concurrent {
                max_in_flight,
                handler: Arc::new(Echo {
                    completed: self.completed.clone(),
                }),
                subscriptions,
            },
            ExecutionPolicy::Dynamic {
                max_lanes,
                idle_timeout,
            } => HandlerRegistration::Dynamic {
                max_lanes,
                idle_timeout,
                factory: Box::new(factory),
                subscriptions,
            },
        }))
    }
}
struct Echo {
    completed: Arc<AtomicU64>,
}
impl Echo {
    fn reply(&self, msg: MessageEnvelope) -> FrameworkResult<HandlerReply> {
        let value = msg.payload::<u64>()?;
        self.completed.fetch_add(1, Ordering::Relaxed);
        Ok(value)
    }
}
#[async_trait]
impl SerialHandler for Echo {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        msg: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.reply(msg)
    }
}
#[async_trait]
impl ConcurrentHandler for Echo {
    async fn handle(
        &self,
        _: &HandlerContext,
        msg: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.reply(msg)
    }
}
fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() % 2 != 0
        || args.chunks(2).any(|a| {
            !matches!(
                a[0].as_str(),
                "--mode" | "--seconds" | "--producers" | "--capacity" | "--messages"
            )
        })
    {
        return Err("usage: runtime-load [--mode serial|partitioned|concurrent|dynamic] [--seconds N] [--producers N] [--capacity N] [--messages N]".into());
    }
    let get = |name: &str, default: &str| -> String {
        args.chunks(2)
            .find(|a| a[0] == name)
            .map(|a| a[1].clone())
            .unwrap_or_else(|| default.into())
    };
    let mode = get("--mode", "serial");
    let seconds: u64 = get("--seconds", "30").parse()?;
    let producers: usize = get("--producers", "32").parse()?;
    let capacity: usize = get("--capacity", "64").parse()?;
    let messages: u64 = get("--messages", "0").parse()?;
    if seconds == 0 || producers == 0 || producers > 4096 || capacity == 0 {
        return Err("invalid workload bounds".into());
    }
    let nz = |n| NonZeroUsize::new(n).unwrap();
    let policy = match mode.as_str() {
        "serial" => ExecutionPolicy::Serial,
        "partitioned" => ExecutionPolicy::Partitioned { shards: nz(16) },
        "concurrent" => ExecutionPolicy::Concurrent {
            max_in_flight: nz(16),
        },
        "dynamic" => ExecutionPolicy::Dynamic {
            max_lanes: nz(128),
            idle_timeout: Duration::from_millis(100),
        },
        _ => return Err("unknown mode".into()),
    };
    let completed = Arc::new(AtomicU64::new(0));
    let replies = Arc::new(AtomicU64::new(0));
    let mut builder = ApplicationBuilder::new("runtime-load");
    builder.register_module(Factory {
        policy,
        capacity,
        completed: completed.clone(),
    })?;
    let mut selection = ComponentSelection::new();
    selection.enable_module("worker");
    let handle = builder.selection(selection).build()?.start().await?;
    let rss_before = rss_kib();
    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_secs(seconds))
        .ok_or("duration overflow")?;
    let mut jobs = tokio::task::JoinSet::new();
    for producer in 0..producers {
        let hub = handle.event_hub();
        let replies = replies.clone();
        jobs.spawn(async move {
            let mut sequence = 0u64;
            loop {
                if (messages > 0 && sequence >= messages)
                    || (messages == 0 && Instant::now() >= deadline)
                {
                    break;
                }
                let value = ((producer as u64) << 48) | sequence;
                let options = MessageOptions {
                    lane_key: Some(format!("lane-{}", producer % 128)),
                    ..Default::default()
                };
                let request = hub
                    .request_wait::<_, u64>(
                        "load/echo/v1",
                        Destination::Component(ComponentRef::module("worker")),
                        value,
                        options,
                        Duration::from_secs(5),
                    )
                    .await?;
                let reply = request.wait(Duration::from_secs(5)).await?;
                if *reply != value {
                    return Err(FrameworkError::TypeMismatch(
                        "reply correlation failed".into(),
                    ));
                }
                replies.fetch_add(1, Ordering::Relaxed);
                sequence += 1;
            }
            Ok::<(), FrameworkError>(())
        });
    }
    let mut samples = vec![];
    let mut sample = tokio::time::interval(Duration::from_secs(10));
    sample.tick().await;
    let mut error = None;
    while !jobs.is_empty() {
        tokio::select! {
            result=jobs.join_next()=> {if let Some(result)=result {match result {Ok(Ok(()))=>{},Ok(Err(e))=>{error=Some(e.to_string());jobs.abort_all();},Err(e)=>{error=Some(e.to_string());jobs.abort_all();}}}},
            _=sample.tick()=> {let m=handle.event_hub().metrics();samples.push(serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"rss_kib":rss_kib(),"completed":completed.load(Ordering::Relaxed),"queued":m.queued,"active":m.execution.active}));eprintln!("{mode}: {:.0}s, {} replies",started.elapsed().as_secs_f64(),replies.load(Ordering::Relaxed));}
        }
    }
    let elapsed = started.elapsed();
    let metrics = handle.event_hub().metrics();
    let shutdown_start = Instant::now();
    let shutdown = handle.shutdown_checked(Default::default()).await?;
    let count = completed.load(Ordering::Relaxed);
    let responses = replies.load(Ordering::Relaxed);
    let output = serde_json::json!({"mode":mode,"producers":producers,"queue_capacity":capacity,"seconds":elapsed.as_secs_f64(),"completed":count,"replies":responses,"requests_per_second":responses as f64/elapsed.as_secs_f64(),"admission_rejections":metrics.rejected,"handler_p95_micros":metrics.execution.p95_micros,"handler_p99_micros":metrics.execution.p99_micros,"peak_handlers":metrics.execution.peak,"rss_before_kib":rss_before,"rss_after_kib":rss_kib(),"samples":samples,"shutdown_millis":shutdown_start.elapsed().as_secs_f64()*1000.,"pending_after_shutdown":handle.health().pending.operations,"shutdown":format!("{shutdown:?}"),"error":error});
    println!("{}", serde_json::to_string_pretty(&output)?);
    if error.is_some()
        || count != responses
        || handle.health().pending.operations != 0
        || shutdown != ShutdownReceipt::GracefullyStopped
    {
        return Err("load validation failed".into());
    }
    Ok(())
}
