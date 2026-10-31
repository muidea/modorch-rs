use crate::{
    config::Config,
    stats::{Counters, Payload},
};
use modorch::*;
use std::{
    hint::black_box,
    num::NonZeroUsize,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub const KINDS: [&str; 4] = ["short", "async_io", "cpu", "blocking_io"];
pub fn owner(index: usize) -> ComponentRef {
    ComponentRef::module(format!("owner_{index}"))
}
pub fn cpu_work(iterations: u64) -> u64 {
    let mut value = black_box(0x123456789abcdefu64);
    for i in 0..iterations {
        value = black_box(
            value
                .rotate_left(7)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(i),
        );
    }
    black_box(value)
}
pub struct Job {
    pub sequence: u64,
    pub kind: usize,
    pub payload: Payload,
}
pub struct Reply {
    pub sequence: u64,
    pub result: FrameworkResult<u64>,
}
pub struct Factory {
    pub index: usize,
    pub config: Arc<Config>,
    pub counters: Arc<Counters>,
}
impl Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(owner(self.index));
        d.queue_capacity = self.config.capacity;
        d.execution = if self.config.policy == "dynamic" {
            ExecutionPolicy::Dynamic {
                max_lanes: NonZeroUsize::new(self.config.lanes).unwrap(),
                idle_timeout: Duration::from_millis(self.config.lane_idle_ms),
            }
        } else {
            ExecutionPolicy::Concurrent {
                max_in_flight: NonZeroUsize::new(self.config.concurrency).unwrap(),
            }
        };
        d.failure = FailurePolicy::Restart {
            max_attempts: self.config.cycles,
            backoff: Duration::from_millis(1),
        };
        d
    }
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        Ok(Box::new(Worker {
            descriptor: self.descriptor(),
            config: self.config.clone(),
            counters: self.counters.clone(),
            tasks: None,
            index: self.index,
        }))
    }
}
struct Worker {
    descriptor: ComponentDescriptor,
    config: Arc<Config>,
    counters: Arc<Counters>,
    tasks: Option<TaskScheduler>,
    index: usize,
}
#[async_trait]
impl Lifecycle for Worker {
    async fn setup(&mut self, ctx: &SetupContext) -> FrameworkResult<()> {
        self.tasks = Some(ctx.tasks.clone());
        Ok(())
    }
}
impl Module for Worker {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor.clone()
    }
    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        let subscriptions = (0..self.config.subscriptions)
            .map(|i| {
                SubscriptionSpec::new(
                    (self.index * self.config.subscriptions + i + 1) as u64,
                    self.descriptor.component(),
                    if i == 0 {
                        "mixed/v1".into()
                    } else {
                        format!("unused/{i}")
                    },
                )
            })
            .collect();
        let tasks = self.tasks.as_ref().unwrap().clone();
        let counters = self.counters.clone();
        let config = self.config.clone();
        let make = move |_| {
            counters.handlers.fetch_add(1, Ordering::Relaxed);
            counters.created_handlers.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Handler {
                config: config.clone(),
                counters: counters.clone(),
                tasks: tasks.clone(),
            }) as Box<dyn SerialHandler>)
        };
        Ok(Some(match self.descriptor.execution {
            ExecutionPolicy::Dynamic {
                max_lanes,
                idle_timeout,
            } => HandlerRegistration::Dynamic {
                max_lanes,
                idle_timeout,
                factory: Box::new(make),
                subscriptions,
            },
            ExecutionPolicy::Concurrent { max_in_flight } => {
                self.counters.handlers.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .created_handlers
                    .fetch_add(1, Ordering::Relaxed);
                HandlerRegistration::Concurrent {
                    max_in_flight,
                    handler: Arc::new(Handler {
                        config: self.config.clone(),
                        counters: self.counters.clone(),
                        tasks: self.tasks.as_ref().unwrap().clone(),
                    }),
                    subscriptions,
                }
            }
            _ => unreachable!(),
        }))
    }
}
struct Handler {
    config: Arc<Config>,
    counters: Arc<Counters>,
    tasks: TaskScheduler,
}
impl Drop for Handler {
    fn drop(&mut self) {
        self.counters.handlers.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Handler {
    async fn reply(&self, msg: MessageEnvelope) -> FrameworkResult<HandlerReply> {
        let job = msg.payload::<Job>()?;
        if job.kind == 4 {
            return Err(FrameworkError::InvalidConfig(
                "injected recovery probe".into(),
            ));
        }
        let input = job.payload.bytes.first().copied().unwrap_or_default();
        if black_box(input) != 0x5a {
            return Err(FrameworkError::TypeMismatch("payload corrupted".into()));
        }
        let timeout = Duration::from_millis(self.config.timeout_ms);
        let result = match job.kind {
            0 => Ok(input as u64),
            1 => {
                tokio::time::sleep(Duration::from_millis(self.config.async_ms)).await;
                Ok(input as u64)
            }
            2 if self.config.execution == "inline" => Ok(cpu_work(self.config.cpu_iterations)),
            2 => {
                let iterations = self.config.cpu_iterations;
                match self
                    .tasks
                    .spawn_cpu_wait(move |_| Ok(cpu_work(iterations)), timeout)
                    .await
                {
                    Ok(task) => task.wait_finished().await,
                    Err(error) => Err(error),
                }
            }
            3 if self.config.execution == "inline" => {
                std::thread::sleep(Duration::from_millis(self.config.blocking_ms));
                Ok(input as u64)
            }
            3 => {
                let delay = Duration::from_millis(self.config.blocking_ms);
                match self
                    .tasks
                    .spawn_blocking_io_wait(
                        move |_| {
                            std::thread::sleep(delay);
                            Ok(input as u64)
                        },
                        timeout,
                    )
                    .await
                {
                    Ok(task) => task.wait_finished().await,
                    Err(error) => Err(error),
                }
            }
            _ => return Err(FrameworkError::TypeMismatch("unknown workload kind".into())),
        };
        self.counters.completed.fetch_add(1, Ordering::Relaxed);
        // Expected capacity failures are business replies, so overload does not
        // accidentally trigger component supervision and distort the load test.
        Ok(Arc::new(Reply {
            sequence: job.sequence,
            result,
        }))
    }
}
#[async_trait]
impl SerialHandler for Handler {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        msg: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.reply(msg).await
    }
}
#[async_trait]
impl ConcurrentHandler for Handler {
    async fn handle(
        &self,
        _: &HandlerContext,
        msg: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.reply(msg).await
    }
}
