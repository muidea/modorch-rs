use async_trait::async_trait;
use modorch_core::*;
use modorch_eventhub::*;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
fn hub() -> (EventHub, OperationTracker, FailureReporter) {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let failures = FailureReporter::new();
    (
        EventHub::new(tracker.clone(), failures.clone()),
        tracker,
        failures,
    )
}
fn dest(id: &str) -> Destination {
    Destination::Component(ComponentRef::module(id))
}
struct Recorder {
    seen: Arc<Mutex<Vec<usize>>>,
    gate: Option<Arc<Semaphore>>,
}
#[async_trait]
impl SerialHandler for Recorder {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        let value = *message.payload::<usize>()?;
        self.seen.lock().unwrap().push(value);
        Ok(Arc::new(value + 1))
    }
}
fn serial(
    hub: &EventHub,
    id: &str,
    sub: u64,
    capacity: usize,
    handler: impl SerialHandler + 'static,
) {
    let mut descriptor = ComponentDescriptor::new(ComponentRef::module(id));
    descriptor.queue_capacity = capacity;
    hub.install(
        &descriptor,
        HandlerRegistration::Serial {
            handler: Box::new(handler),
            subscriptions: vec![SubscriptionSpec::new(sub, descriptor.component(), "test/#")],
        },
    )
    .unwrap();
}
async fn close(hub: &EventHub, tracker: &OperationTracker) {
    tracker.close();
    tokio::time::timeout(Duration::from_secs(3), tracker.drain())
        .await
        .unwrap();
    hub.terminate(false).await;
}
#[tokio::test]
async fn serial_fifo_dynamic_subscriptions_and_typed_requests() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        32,
        Recorder {
            seen: seen.clone(),
            gate: None,
        },
    );
    for value in 0..20usize {
        hub.command(
            "test/item",
            dest("a"),
            value,
            MessageOptions {
                lane_key: Some(format!("{value}")),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let response = hub
        .request::<_, usize>("test/item", dest("a"), 20usize, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(*response, 21);
    assert_eq!(*seen.lock().unwrap(), (0..=20).collect::<Vec<_>>());
    let removed = hub.unsubscribe(1).unwrap();
    assert!(matches!(
        hub.command("test/item", dest("a"), 1usize, Default::default()),
        Err(FrameworkError::Unroutable(_))
    ));
    let added = hub
        .subscribe(SubscriptionSpec::new(
            2,
            ComponentRef::module("a"),
            "test/:id",
        ))
        .unwrap();
    assert!(added.generation > removed.generation);
    hub.command("test/item", dest("a"), 1usize, Default::default())
        .unwrap();
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn bounded_fanout_reports_partial_delivery_and_ambiguous_requests() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    serial(
        &hub,
        "a",
        1,
        1,
        Recorder {
            seen: Arc::default(),
            gate: Some(gate.clone()),
        },
    );
    serial(
        &hub,
        "b",
        2,
        8,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    hub.command("test/x", dest("a"), 0usize, Default::default())
        .unwrap();
    tokio::task::yield_now().await;
    hub.command("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    assert!(matches!(
        hub.command("test/x", dest("a"), 2usize, Default::default()),
        Err(FrameworkError::Backpressure(_))
    ));
    let report = hub
        .publish(
            "test/x",
            Destination::Broadcast,
            3usize,
            Default::default(),
            PublishPolicy::Reject,
        )
        .unwrap();
    assert!(report.is_partial());
    assert_eq!(report.accepted(), 1);
    assert_eq!(report.rejected(), 1);
    assert!(matches!(
        hub.request::<_, usize>("test/x", Destination::Broadcast, 1usize, Default::default()),
        Err(FrameworkError::AmbiguousDestination(_))
    ));
    gate.add_permits(2);
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn queued_timeout_cancels_but_running_timeout_is_indeterminate() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        4,
        Recorder {
            seen: seen.clone(),
            gate: Some(gate.clone()),
        },
    );
    let running = hub
        .request::<_, usize>("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    let observation = running.observation();
    tokio::task::yield_now().await;
    assert!(matches!(
        running.wait(Duration::from_millis(1)).await,
        Err(FrameworkError::Indeterminate(_))
    ));
    let queued = hub
        .request::<_, usize>("test/x", dest("a"), 2usize, Default::default())
        .unwrap();
    assert!(matches!(
        queued.wait(Duration::from_millis(1)).await,
        Err(FrameworkError::CanceledBeforeStart(_))
    ));
    assert_eq!(tracker.pending().operations, 2);
    gate.add_permits(1);
    close(&hub, &tracker).await;
    assert_eq!(*seen.lock().unwrap(), vec![1]);
    assert_eq!(
        observation.status(),
        RequestStatus::CompletedAfterCallerTimeout
    );
    let metrics = hub.metrics().execution;
    assert_eq!(metrics.caller_timeouts, 2);
    assert_eq!(metrics.caller_cancellations, 0);
    assert_eq!(metrics.canceled_before_start, 1);
}
struct Reentrant {
    next: String,
}
#[async_trait]
impl SerialHandler for Reentrant {
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        let child = ctx.event_hub.request::<_, usize>(
            "test/cycle",
            dest(&self.next),
            0usize,
            Default::default(),
        )?;
        child
            .wait(Duration::from_secs(1))
            .await
            .map(|v| v as HandlerReply)
    }
}
#[tokio::test]
async fn rejects_self_and_cross_component_wait_cycles() {
    for names in [
        vec![("a", "a")],
        vec![("a", "b"), ("b", "a")],
        vec![("a", "b"), ("b", "c"), ("c", "a")],
    ] {
        let (hub, tracker, _) = hub();
        for (index, (id, next)) in names.into_iter().enumerate() {
            serial(&hub, id, index as u64, 8, Reentrant { next: next.into() });
        }
        let result = hub
            .request::<_, usize>("test/cycle", dest("a"), 0usize, Default::default())
            .unwrap()
            .wait(Duration::from_secs(1))
            .await;
        assert!(
            matches!(
                result.as_ref().map_err(FrameworkError::root_cause),
                Err(FrameworkError::CircularWait(_))
            ),
            "{result:?}"
        );
        close(&hub, &tracker).await;
    }
}
struct Parallel {
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}
#[async_trait]
impl ConcurrentHandler for Parallel {
    async fn handle(
        &self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(message.payload)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workers_obey_strict_limit() {
    let (hub, tracker, _) = hub();
    let peak = Arc::new(AtomicUsize::new(0));
    let limit = NonZeroUsize::new(3).unwrap();
    let mut d = ComponentDescriptor::new(ComponentRef::module("a"));
    d.execution = ExecutionPolicy::Concurrent {
        max_in_flight: limit,
    };
    hub.install(
        &d,
        HandlerRegistration::Concurrent {
            max_in_flight: limit,
            handler: Arc::new(Parallel {
                active: Arc::default(),
                peak: peak.clone(),
            }),
            subscriptions: vec![SubscriptionSpec::new(1, d.component(), "test/#")],
        },
    )
    .unwrap();
    for i in 0..30 {
        hub.command("test/x", dest("a"), i, Default::default())
            .unwrap();
    }
    close(&hub, &tracker).await;
    assert_eq!(peak.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn partitioned_same_key_is_fifo_and_independent_shards_progress() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    let gate = Arc::new(Semaphore::new(0));
    let shards = NonZeroUsize::new(2).unwrap();
    let mut d = ComponentDescriptor::new(ComponentRef::module("a"));
    d.execution = ExecutionPolicy::Partitioned { shards };
    let factory_seen = seen.clone();
    let factory_gate = gate.clone();
    hub.install(
        &d,
        HandlerRegistration::Partitioned {
            shards,
            factory: Box::new(move |shard| {
                Ok(Box::new(Recorder {
                    seen: factory_seen.clone(),
                    gate: if shard == 0 {
                        Some(factory_gate.clone())
                    } else {
                        None
                    },
                }) as Box<dyn SerialHandler>)
            }),
            subscriptions: vec![SubscriptionSpec::new(1, d.component(), "test/#")],
        },
    )
    .unwrap();
    // FNV parity: "a" goes to shard 0, "b" to shard 1.
    for i in 0..5usize {
        hub.command(
            "test/x",
            dest("a"),
            i,
            MessageOptions {
                lane_key: Some("a".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let reply = hub
        .request::<_, usize>(
            "test/x",
            dest("a"),
            99usize,
            MessageOptions {
                lane_key: Some("b".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(*reply, 100);
    gate.add_permits(5);
    close(&hub, &tracker).await;
    assert_eq!(*seen.lock().unwrap(), vec![99, 0, 1, 2, 3, 4]);
}
#[test]
fn patterns_are_directional_and_segment_based() {
    assert!(topic_matches("orders/+/:id/#", "orders/create/42/a"));
    assert!(topic_matches("orders/#", "orders"));
    assert!(!topic_matches("orders/x", "orders/+"));
    assert!(!topic_matches("orders/+", "orders/x/y"));
}

struct IndependentWait {
    next: String,
    barrier: Arc<tokio::sync::Barrier>,
}
#[async_trait]
impl SerialHandler for IndependentWait {
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        if *message.payload::<bool>()? {
            self.barrier.wait().await;
            return ctx
                .event_hub
                .request::<_, usize>("test/x", dest(&self.next), false, Default::default())?
                .wait(Duration::from_secs(1))
                .await
                .map(|reply| reply as HandlerReply);
        }
        Ok(Arc::new(1usize))
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detects_wait_cycles_between_independent_roots() {
    let (hub, tracker, _) = hub();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    serial(
        &hub,
        "a",
        1,
        8,
        IndependentWait {
            next: "b".into(),
            barrier: barrier.clone(),
        },
    );
    serial(
        &hub,
        "b",
        2,
        8,
        IndependentWait {
            next: "a".into(),
            barrier,
        },
    );
    let a = hub
        .request::<_, usize>("test/x", dest("a"), true, Default::default())
        .unwrap();
    let b = hub
        .request::<_, usize>("test/x", dest("b"), true, Default::default())
        .unwrap();
    let (a, b) = tokio::join!(
        a.wait(Duration::from_secs(1)),
        b.wait(Duration::from_secs(1))
    );
    assert!(
        matches!(
            a.as_ref().map_err(FrameworkError::root_cause),
            Err(FrameworkError::CircularWait(_))
        ) || matches!(
            b.as_ref().map_err(FrameworkError::root_cause),
            Err(FrameworkError::CircularWait(_))
        )
    );
    assert!(a.is_ok() || b.is_ok());
    close(&hub, &tracker).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_changes_never_publish_stale_routes() {
    let (hub, tracker, _) = hub();
    serial(
        &hub,
        "a",
        1,
        128,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    hub.unsubscribe(1).unwrap();
    let producer_hub = hub.clone();
    let producer = tokio::spawn(async move {
        for i in 0..1000usize {
            let result = producer_hub.command("test/x", dest("a"), i, Default::default());
            assert!(
                result.is_ok()
                    || matches!(
                        result,
                        Err(FrameworkError::Unroutable(_) | FrameworkError::Backpressure(_))
                    )
            );
            tokio::task::yield_now().await;
        }
    });
    for i in 2..102 {
        hub.subscribe(SubscriptionSpec::new(
            i,
            ComponentRef::module("a"),
            "test/#",
        ))
        .unwrap();
        tokio::task::yield_now().await;
        hub.unsubscribe(i).unwrap();
        assert!(matches!(
            hub.command("test/x", dest("a"), 0usize, Default::default()),
            Err(FrameworkError::Unroutable(_))
        ));
    }
    producer.await.unwrap();
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn aborting_a_worker_completes_waiters_with_an_honest_result() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    serial(
        &hub,
        "a",
        1,
        8,
        Recorder {
            seen: Arc::default(),
            gate: Some(gate),
        },
    );
    let running = hub
        .request::<_, usize>("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    tokio::task::yield_now().await;
    let queued = hub
        .request::<_, usize>("test/x", dest("a"), 2usize, Default::default())
        .unwrap();
    tracker.terminate();
    hub.terminate(true).await;
    assert!(matches!(
        running.wait(Duration::from_secs(1)).await,
        Err(FrameworkError::Indeterminate(_))
    ));
    assert!(matches!(
        queued.wait(Duration::from_secs(1)).await,
        Err(FrameworkError::CanceledBeforeStart(_))
    ));
    assert_eq!(tracker.pending().operations, 0);
}
#[tokio::test]
async fn external_ingress_cannot_inherit_a_producer_tasks_shutdown_privileges() {
    let (hub, tracker, _) = hub();
    serial(
        &hub,
        "a",
        1,
        8,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    let producer = tracker
        .admit_background(ComponentRef::initiator("ingress"))
        .unwrap();
    tracker.close();
    tracker
        .scope(producer, async {
            assert!(matches!(
                hub.external()
                    .command("test/x", dest("a"), 0usize, Default::default()),
                Err(FrameworkError::Closed)
            ));
            hub.command("test/x", dest("a"), 1usize, Default::default())
                .unwrap();
        })
        .await;
    close(&hub, &tracker).await;
}

struct Panicking;
#[async_trait]
impl SerialHandler for Panicking {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        panic!("injected handler panic")
    }
}
#[tokio::test]
async fn handler_panic_is_supervised_and_has_no_success_receipt() {
    let (hub, tracker, failures) = hub();
    serial(&hub, "a", 1, 4, Panicking);
    let result = hub
        .request::<_, usize>("test/x", dest("a"), 0usize, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await;
    assert!(matches!(
        result,
        Err(FrameworkError::ExecutionFailed { .. })
    ));
    assert!(failures.current().is_some());
    close(&hub, &tracker).await;
    let metrics = hub.metrics();
    assert_eq!(metrics.execution.active, 0);
    assert_eq!(metrics.execution.failed, 1);
    assert_eq!(metrics.execution.completed, 1);
    assert!(metrics.execution.p95_micros > 0);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bounded_mailbox_recovers_under_competing_producers_without_losing_fifo() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        8,
        Recorder {
            seen: seen.clone(),
            gate: None,
        },
    );
    let mut producers = vec![];
    for producer in 0..16usize {
        let hub = hub.clone();
        producers.push(tokio::spawn(async move {
            for sequence in 0..100usize {
                loop {
                    match hub.command(
                        "test/x",
                        dest("a"),
                        producer * 1000 + sequence,
                        Default::default(),
                    ) {
                        Ok(_) => break,
                        Err(FrameworkError::Backpressure(_)) => tokio::task::yield_now().await,
                        Err(error) => panic!("unexpected rejection: {error}"),
                    }
                }
            }
        }));
    }
    for producer in producers {
        producer.await.unwrap();
    }
    close(&hub, &tracker).await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1600);
    for producer in 0..16usize {
        assert_eq!(
            seen.iter()
                .copied()
                .filter(|value| value / 1000 == producer)
                .map(|value| value % 1000)
                .collect::<Vec<_>>(),
            (0..100).collect::<Vec<_>>()
        );
    }
    assert_eq!(hub.metrics().accepted, 1600);
}

#[tokio::test]
async fn named_destinations_and_topics_route_independently_of_lane_keys() {
    let (hub, tracker, _) = hub();
    let a = Arc::new(Mutex::new(vec![]));
    let b = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        8,
        Recorder {
            seen: a.clone(),
            gate: None,
        },
    );
    serial(
        &hub,
        "b",
        2,
        8,
        Recorder {
            seen: b.clone(),
            gate: None,
        },
    );
    hub.unsubscribe(1).unwrap();
    hub.unsubscribe(2).unwrap();
    for (id, target, name) in [(3, "a", "orders"), (4, "b", "billing")] {
        let mut spec = SubscriptionSpec::new(id, ComponentRef::module(target), "test/item");
        spec.destination_pattern = DestinationPattern::Exact(Destination::Named(name.into()));
        hub.subscribe(spec).unwrap();
    }
    for (value, lane) in [(1usize, "customer-a"), (2, "customer-b")] {
        let result = hub
            .request::<_, usize>(
                "test/item",
                Destination::Named("orders".into()),
                value,
                MessageOptions {
                    lane_key: Some(lane.into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .wait(Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(*result, value + 1);
    }
    let report = hub
        .publish(
            "test/item",
            Destination::Named("billing".into()),
            3usize,
            Default::default(),
            PublishPolicy::Reject,
        )
        .unwrap();
    assert_eq!(report.accepted(), 1);
    assert!(matches!(
        hub.command(
            "test/other",
            Destination::Named("orders".into()),
            0usize,
            Default::default()
        ),
        Err(FrameworkError::Unroutable(_))
    ));
    assert!(matches!(
        hub.subscribe(SubscriptionSpec::new(
            9,
            ComponentRef::module("disabled"),
            "test/#"
        )),
        Err(FrameworkError::ComponentUnavailable(_))
    ));
    close(&hub, &tracker).await;
    assert_eq!(*a.lock().unwrap(), vec![1, 2]);
    assert_eq!(*b.lock().unwrap(), vec![3]);
}

#[tokio::test(start_paused = true)]
async fn dynamic_lanes_are_bounded_retire_when_idle_and_recreate_without_reordering() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    let creates = Arc::new(AtomicUsize::new(0));
    let mut d = ComponentDescriptor::new(ComponentRef::module("a"));
    let max = NonZeroUsize::new(2).unwrap();
    let idle = Duration::from_secs(1);
    d.execution = ExecutionPolicy::Dynamic {
        max_lanes: max,
        idle_timeout: idle,
    };
    let records = seen.clone();
    let factories = creates.clone();
    hub.install(
        &d,
        HandlerRegistration::Dynamic {
            max_lanes: max,
            idle_timeout: idle,
            factory: Box::new(move |_| {
                factories.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Recorder {
                    seen: records.clone(),
                    gate: None,
                }) as Box<dyn SerialHandler>)
            }),
            subscriptions: vec![SubscriptionSpec::new(1, d.component(), "test/#")],
        },
    )
    .unwrap();
    for (i, key) in [(0usize, "a"), (1, "a"), (2, "b")] {
        hub.command(
            "test/x",
            dest("a"),
            i,
            MessageOptions {
                lane_key: Some(key.into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(matches!(
        hub.command(
            "test/x",
            dest("a"),
            3usize,
            MessageOptions {
                lane_key: Some("c".into()),
                ..Default::default()
            }
        ),
        Err(FrameworkError::Backpressure(_))
    ));
    assert_eq!(hub.mailbox_metrics()[0].shard_queue_depths.len(), 2);
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(hub.mailbox_metrics()[0].shard_queue_depths.is_empty());
    let response = hub
        .request::<_, usize>(
            "test/x",
            dest("a"),
            3usize,
            MessageOptions {
                lane_key: Some("a".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(*response, 4);
    assert_eq!(creates.load(Ordering::SeqCst), 3);
    close(&hub, &tracker).await;
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().position(|v| *v == 0).unwrap() < seen.iter().position(|v| *v == 1).unwrap()
    );
    assert!(hub.mailbox_metrics().is_empty());
}
#[tokio::test]
async fn dynamic_lane_sync_reentry_is_detected() {
    let (hub, tracker, _) = hub();
    let mut d = ComponentDescriptor::new(ComponentRef::module("a"));
    let max = NonZeroUsize::new(2).unwrap();
    let idle = Duration::from_millis(10);
    d.execution = ExecutionPolicy::Dynamic {
        max_lanes: max,
        idle_timeout: idle,
    };
    hub.install(
        &d,
        HandlerRegistration::Dynamic {
            max_lanes: max,
            idle_timeout: idle,
            factory: Box::new(|_| {
                Ok(Box::new(Reentrant { next: "a".into() }) as Box<dyn SerialHandler>)
            }),
            subscriptions: vec![SubscriptionSpec::new(1, d.component(), "test/#")],
        },
    )
    .unwrap();
    let result = hub
        .request::<_, usize>("test/x", dest("a"), 0usize, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await;
    assert!(matches!(
        result.as_ref().map_err(FrameworkError::root_cause),
        Err(FrameworkError::CircularWait(_))
    ));
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn waiting_admission_succeeds_after_capacity_and_timeout_never_enqueues_later() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        1,
        Recorder {
            seen: seen.clone(),
            gate: Some(gate.clone()),
        },
    );
    hub.command("test/x", dest("a"), 0usize, Default::default())
        .unwrap();
    tokio::task::yield_now().await;
    hub.command("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    let result = hub
        .command_wait(
            "test/x",
            dest("a"),
            99usize,
            Default::default(),
            Duration::from_millis(1),
        )
        .await;
    assert!(matches!(result, Err(FrameworkError::AdmissionTimeout)));
    let pending_hub = hub.clone();
    let pending = tokio::spawn(async move {
        pending_hub
            .request_wait::<_, usize>(
                "test/x",
                dest("a"),
                2usize,
                Default::default(),
                Duration::from_secs(1),
            )
            .await
    });
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    gate.add_permits(3);
    let request = pending.await.unwrap().unwrap();
    assert_eq!(*request.wait(Duration::from_secs(1)).await.unwrap(), 3);
    close(&hub, &tracker).await;
    assert_eq!(*seen.lock().unwrap(), vec![0, 1, 2]);
}
#[tokio::test]
async fn waiting_admission_is_cancel_safe_and_wakes_on_shutdown() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        1,
        Recorder {
            seen: seen.clone(),
            gate: Some(gate.clone()),
        },
    );
    hub.command("test/x", dest("a"), 0usize, Default::default())
        .unwrap();
    tokio::task::yield_now().await;
    hub.command("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    let h = hub.clone();
    let canceled = tokio::spawn(async move {
        h.command_wait(
            "test/x",
            dest("a"),
            99usize,
            Default::default(),
            Duration::from_secs(60),
        )
        .await
    });
    tokio::task::yield_now().await;
    canceled.abort();
    let _ = canceled.await;
    let h = hub.clone();
    let stopped = tokio::spawn(async move {
        h.command_wait(
            "test/x",
            dest("a"),
            100usize,
            Default::default(),
            Duration::from_secs(60),
        )
        .await
    });
    tokio::task::yield_now().await;
    tracker.close();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), stopped)
            .await
            .unwrap()
            .unwrap(),
        Err(FrameworkError::Closed)
    ));
    gate.add_permits(2);
    close(&hub, &tracker).await;
    assert_eq!(*seen.lock().unwrap(), vec![0, 1]);
}
struct WaitingHandler;
#[async_trait]
impl SerialHandler for WaitingHandler {
    async fn handle(
        &mut self,
        ctx: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        ctx.event_hub
            .command_wait(
                "test/x",
                dest("a"),
                0usize,
                Default::default(),
                Duration::from_secs(1),
            )
            .await?;
        Ok(Arc::new(0usize))
    }
}
#[tokio::test]
async fn handler_cannot_deadlock_by_waiting_for_mailbox_capacity() {
    let (hub, tracker, _) = hub();
    serial(&hub, "a", 1, 1, WaitingHandler);
    let result = hub
        .request::<_, usize>("test/x", dest("a"), 0usize, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await;
    assert!(matches!(
        result.as_ref().map_err(FrameworkError::root_cause),
        Err(FrameworkError::InvalidConfig(_))
    ));
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn waiting_fanout_delivers_ready_subscribers_without_retrying_accepted_legs() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    let a = Arc::new(Mutex::new(vec![]));
    let b = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "a",
        1,
        1,
        Recorder {
            seen: a.clone(),
            gate: Some(gate.clone()),
        },
    );
    serial(
        &hub,
        "b",
        2,
        8,
        Recorder {
            seen: b.clone(),
            gate: None,
        },
    );
    hub.command("test/x", dest("a"), 0usize, Default::default())
        .unwrap();
    tokio::task::yield_now().await;
    hub.command("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    let h = hub.clone();
    let publish = tokio::spawn(async move {
        h.publish_wait(
            "test/x",
            Destination::Broadcast,
            2usize,
            Default::default(),
            Duration::from_millis(20),
        )
        .await
        .unwrap()
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(*b.lock().unwrap(), vec![2]);
    let report = publish.await.unwrap();
    assert!(report.is_partial());
    assert!(matches!(
        report.deliveries[0].result,
        Err(FrameworkError::AdmissionTimeout)
    ));
    gate.add_permits(2);
    close(&hub, &tracker).await;
    assert_eq!(*a.lock().unwrap(), vec![0, 1]);
    assert_eq!(*b.lock().unwrap(), vec![2]);
}

#[tokio::test]
async fn message_metrics_distinguish_routes_kinds_targets_and_applications() {
    let (hub, tracker, _) = hub();
    serial(
        &hub,
        "a",
        1,
        4,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    serial(
        &hub,
        "b",
        2,
        4,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    assert!(
        hub.command("missing", dest("a"), (), Default::default())
            .is_err()
    );
    assert!(
        hub.request::<_, usize>("test/x", Destination::Broadcast, (), Default::default())
            .is_err()
    );
    let request = hub
        .request::<_, usize>("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    request.wait(Duration::from_secs(1)).await.unwrap();
    let report = hub
        .publish(
            "test/x",
            Destination::Broadcast,
            2usize,
            Default::default(),
            PublishPolicy::Reject,
        )
        .unwrap();
    assert_eq!(report.accepted(), 2);
    close(&hub, &tracker).await;
    assert!(
        hub.command("test/x", dest("a"), (), Default::default())
            .is_err()
    );
    let rows = hub.message_metrics();
    let count = |outcome| rows.iter().map(|row| row.routing[&outcome]).sum::<u64>();
    assert_eq!(count(RouteOutcome::Unroutable), 1);
    assert_eq!(count(RouteOutcome::AmbiguousDestination), 1);
    assert_eq!(count(RouteOutcome::Closed), 1);
    assert_eq!(count(RouteOutcome::Accepted), 3);
    assert_eq!(hub.metrics().accepted, 3);
    assert_eq!(hub.metrics().rejected, 3);
    assert!(
        rows.iter()
            .all(|row| row.application_id == tracker.application_id())
    );
    let request = rows
        .iter()
        .find(|row| {
            row.labels.kind == Some(MessageKind::Request)
                && row.labels.component == Some(ComponentRef::module("a"))
        })
        .unwrap();
    assert_eq!(request.execution.completed, 1);
    assert_eq!(request.execution.active, 0);
    assert!(request.execution.p99_micros >= request.execution.p95_micros);
    assert!(request.execution.p95_micros > 0);
    assert_eq!(
        rows.iter()
            .filter(|row| row.labels.kind == Some(MessageKind::Event))
            .count(),
        2
    );
    let other = EventHub::new(OperationTracker::new(), FailureReporter::new());
    assert_ne!(other.metrics().application_id, tracker.application_id());
    assert!(other.message_metrics().is_empty());
}

#[tokio::test]
async fn metric_label_overflow_preserves_totals_and_existing_labels() {
    let (hub, tracker, _) = hub();
    for i in 0..MESSAGE_METRIC_LABEL_LIMIT + 10 {
        assert!(
            hub.command(
                &format!("unknown/{i}"),
                dest("missing"),
                (),
                Default::default()
            )
            .is_err()
        );
    }
    // New labels share overflow, while old labels keep accumulating accurately.
    assert!(
        hub.command("unknown/0", dest("missing"), (), Default::default())
            .is_err()
    );
    serial(
        &hub,
        "a",
        1,
        4,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    hub.request::<_, usize>("test/overflow", dest("a"), 0usize, Default::default())
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap();
    close(&hub, &tracker).await;
    let rows = hub.message_metrics();
    assert_eq!(rows.len(), MESSAGE_METRIC_LABEL_LIMIT + 1);
    let overflow = rows.iter().find(|row| row.overflow).unwrap();
    assert_eq!(overflow.routing[&RouteOutcome::Accepted], 1);
    assert_eq!(overflow.execution.completed, 1);
    assert_eq!(
        rows.iter().find(|row| row.overflow).unwrap().routing[&RouteOutcome::Unroutable],
        10
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.labels.topic.as_deref() == Some("unknown/0"))
            .unwrap()
            .routing[&RouteOutcome::Unroutable],
        2
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.routing[&RouteOutcome::Unroutable])
            .sum::<u64>(),
        (MESSAGE_METRIC_LABEL_LIMIT + 11) as u64
    );
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn metrics_separate_admission_deadlines_reply_timeouts_and_cancellation() {
    let (hub, tracker, _) = hub();
    let gate = Arc::new(Semaphore::new(0));
    serial(
        &hub,
        "a",
        1,
        1,
        Recorder {
            seen: Arc::default(),
            gate: Some(gate.clone()),
        },
    );
    let running = hub
        .request::<_, usize>("test/x", dest("a"), 0usize, Default::default())
        .unwrap();
    while running.status() != RequestStatus::Running {
        tokio::task::yield_now().await;
    }
    let canceled = hub
        .request::<_, usize>("test/x", dest("a"), 1usize, Default::default())
        .unwrap();
    canceled.cancel();
    assert!(matches!(
        hub.request_wait::<_, usize>(
            "test/x",
            dest("a"),
            2usize,
            Default::default(),
            Duration::from_millis(1)
        )
        .await,
        Err(FrameworkError::AdmissionTimeout)
    ));
    let metrics = hub.metrics().execution;
    // Failed admission attempts must not look like queued-request cancellations.
    assert_eq!(metrics.canceled_before_start, 1);
    assert_eq!(metrics.caller_cancellations, 1);
    assert_eq!(metrics.caller_timeouts, 0);
    assert!(matches!(
        running.wait(Duration::from_millis(1)).await,
        Err(FrameworkError::Indeterminate(_))
    ));
    gate.add_permits(1);
    tracker.close();
    tracker.drain().await;
    let rows = hub.message_metrics();
    let execution = rows
        .iter()
        .find(|row| row.labels.component == Some(ComponentRef::module("a")))
        .unwrap()
        .execution;
    assert_eq!(execution.canceled_before_start, 1);
    assert_eq!(execution.caller_cancellations, 1);
    assert_eq!(execution.caller_timeouts, 1);
    assert_eq!(execution.completed, 1);
    assert_eq!(
        rows.iter()
            .map(|row| row.routing[&RouteOutcome::AdmissionTimeout])
            .sum::<u64>(),
        1
    );
    assert!(
        rows.iter()
            .map(|row| row.routing[&RouteOutcome::Backpressure])
            .sum::<u64>()
            > 0
    );
    hub.terminate(false).await;
}

#[tokio::test]
async fn handler_errors_keep_execution_identity_and_the_original_source() {
    let (hub, tracker, _) = hub();
    serial(
        &hub,
        "a",
        1,
        4,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    // Recorder expects usize; a payload type error is a real handler failure.
    let request = hub
        .request::<_, usize>("test/wrong", dest("a"), "wrong", Default::default())
        .unwrap();
    let id = request.operation_id();
    let error = request.wait(Duration::from_secs(1)).await.unwrap_err();
    assert_eq!(error.operation_id(), Some(id));
    assert_eq!(error.component(), Some(&ComponentRef::module("a")));
    assert_eq!(error.phase(), Some(Phase::Handle));
    assert!(!error.retryable());
    assert!(matches!(
        error.root_cause(),
        FrameworkError::TypeMismatch(_)
    ));
    assert!(std::error::Error::source(&error).is_some());
    close(&hub, &tracker).await;
    let row = hub
        .message_metrics()
        .into_iter()
        .find(|row| row.labels.component.is_some())
        .unwrap();
    assert_eq!(row.execution.failed, 1);
}

struct LargeReply {
    observed: Arc<Mutex<Option<std::sync::Weak<Vec<u8>>>>>,
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
}
#[async_trait]
impl SerialHandler for LargeReply {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        _: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.started.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        let value = Arc::new(vec![7u8; 16 * 1024 * 1024]);
        *self.observed.lock().unwrap() = Some(Arc::downgrade(&value));
        Ok(value)
    }
}
#[tokio::test]
async fn status_observers_never_retain_consumed_or_abandoned_replies() {
    for abandon in [false, true] {
        let (hub, tracker, _) = hub();
        let observed = Arc::new(Mutex::new(None));
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        serial(
            &hub,
            "large",
            1,
            2,
            LargeReply {
                observed: observed.clone(),
                started: started.clone(),
                release: release.clone(),
            },
        );
        let handle = hub
            .request::<_, Vec<u8>>("test/large", dest("large"), (), MessageOptions::default())
            .unwrap();
        let observation = handle.observation();
        started.acquire().await.unwrap().forget();
        if abandon {
            drop(handle);
        } else {
            release.add_permits(1);
            drop(handle.wait(Duration::from_secs(2)).await.unwrap());
        }
        if abandon {
            release.add_permits(1);
        }
        tracker.drain().await;
        assert!(
            observed
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .upgrade()
                .is_none()
        );
        assert_eq!(observation.status(), RequestStatus::Completed);
        close(&hub, &tracker).await;
    }
}

#[tokio::test]
async fn byte_budgets_and_waiter_limits_release_on_cancellation() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let limits = HubLimits {
        max_waiters: 1,
        max_waiters_per_target: 1,
        max_waiting_bytes: Some(8),
        max_in_flight_bytes: Some(8),
        max_in_flight_bytes_per_target: Some(8),
        ..HubLimits::default()
    };
    let hub = EventHub::with_limits(tracker.clone(), FailureReporter::new(), limits).unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "limited",
        1,
        4,
        Recorder {
            seen: seen.clone(),
            gate: Some(gate.clone()),
        },
    );
    let options = MessageOptions {
        payload_bytes: Some(8),
        ..MessageOptions::default()
    };
    assert!(matches!(
        hub.command("test/x", dest("limited"), 0usize, MessageOptions::default()),
        Err(FrameworkError::InvalidConfig(_))
    ));
    hub.command("test/x", dest("limited"), 1usize, options.clone())
        .unwrap();
    let waiter_hub = hub.clone();
    let waiter_options = options.clone();
    let waiter = tokio::spawn(async move {
        waiter_hub
            .command_wait(
                "test/x",
                dest("limited"),
                2usize,
                waiter_options,
                Duration::from_secs(2),
            )
            .await
    });
    while hub.metrics().admission.waiters != 1 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        hub.command_wait(
            "test/x",
            dest("limited"),
            3usize,
            options.clone(),
            Duration::from_secs(1)
        )
        .await,
        Err(FrameworkError::ResourceLimit(_))
    ));
    assert_eq!(hub.metrics().admission.waiting_bytes, 8);
    waiter.abort();
    let _ = waiter.await;
    assert_eq!(hub.metrics().admission.waiters, 0);
    gate.add_permits(1);
    tracker.drain().await;
    assert_eq!(hub.metrics().admission.in_flight_bytes, 0);
    hub.command("test/x", dest("limited"), 4usize, options)
        .unwrap();
    gate.add_permits(1);
    close(&hub, &tracker).await;
    assert_eq!(*seen.lock().unwrap(), vec![1, 4]);
}

#[tokio::test]
async fn metadata_and_subscription_bounds_reject_before_dispatch() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let hub = EventHub::with_limits(
        tracker.clone(),
        FailureReporter::new(),
        HubLimits {
            max_topic_bytes: 8,
            max_lane_key_bytes: 4,
            max_headers: 1,
            max_header_bytes: 4,
            max_subscriptions: 1,
            ..HubLimits::default()
        },
    )
    .unwrap();
    serial(
        &hub,
        "a",
        1,
        2,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    assert!(matches!(
        hub.subscribe(SubscriptionSpec::new(2, ComponentRef::module("a"), "other")),
        Err(FrameworkError::ResourceLimit(_))
    ));
    for (topic, options) in [
        ("test/long", MessageOptions::default()),
        (
            "test/x",
            MessageOptions {
                lane_key: Some("12345".into()),
                ..MessageOptions::default()
            },
        ),
        (
            "test/x",
            MessageOptions {
                headers: [("key".into(), "value".into())].into(),
                ..MessageOptions::default()
            },
        ),
    ] {
        assert!(matches!(
            hub.command(topic, dest("a"), 0usize, options),
            Err(FrameworkError::InvalidConfig(_))
        ));
    }
    assert_eq!(tracker.pending().operations, 0);
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn completed_unconsumed_and_timed_out_replies_release_with_observers_alive() {
    for timed_out in [false, true] {
        let (hub, tracker, _) = hub();
        let observed = Arc::new(Mutex::new(None));
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        serial(
            &hub,
            "large",
            1,
            2,
            LargeReply {
                observed: observed.clone(),
                started: started.clone(),
                release: release.clone(),
            },
        );
        let handle = hub
            .request::<_, Vec<u8>>("test/large", dest("large"), (), MessageOptions::default())
            .unwrap();
        let observation = handle.observation();
        started.acquire().await.unwrap().forget();
        if timed_out {
            assert!(matches!(
                handle.wait(Duration::from_millis(1)).await,
                Err(FrameworkError::Indeterminate(_))
            ));
            release.add_permits(1);
            tracker.drain().await;
            assert_eq!(
                observation.status(),
                RequestStatus::CompletedAfterCallerTimeout
            );
        } else {
            release.add_permits(1);
            tracker.drain().await;
            assert!(
                observed
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .is_some()
            );
            drop(handle);
            assert_eq!(observation.status(), RequestStatus::Completed);
        }
        assert!(
            observed
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .upgrade()
                .is_none()
        );
        close(&hub, &tracker).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contended_admission_notifications_remain_safe_across_capacity_retries() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    serial(
        &hub,
        "contended",
        1,
        1,
        Recorder {
            seen: seen.clone(),
            gate: None,
        },
    );
    let mut producers = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let hub = hub.clone();
        producers.spawn(async move {
            for value in 0..512usize {
                let result = hub
                    .request_wait::<_, usize>(
                        "test/x",
                        dest("contended"),
                        value,
                        MessageOptions::default(),
                        Duration::from_secs(10),
                    )
                    .await
                    .unwrap()
                    .wait(Duration::from_secs(10))
                    .await
                    .unwrap();
                assert_eq!(*result, value + 1);
            }
        });
    }
    while let Some(result) = producers.join_next().await {
        result.unwrap();
    }
    close(&hub, &tracker).await;
    assert_eq!(seen.lock().unwrap().len(), 64 * 512);
    assert_eq!(hub.metrics().admission.waiters, 0);
}

struct ParallelEntry(Arc<AtomicUsize>);
#[async_trait]
impl ConcurrentHandler for ParallelEntry {
    async fn handle(
        &self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while self.0.load(Ordering::SeqCst) < 4 {
            if std::time::Instant::now() >= deadline {
                return Err(FrameworkError::InvalidConfig(
                    "receiver handoff stranded a parallel worker".into(),
                ));
            }
            std::thread::yield_now();
        }
        Ok(message.payload)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_worker_handoff_allows_parallel_synchronous_handlers() {
    for _ in 0..8 {
        let (hub, tracker, _) = hub();
        let started = Arc::new(AtomicUsize::new(0));
        let limit = NonZeroUsize::new(4).unwrap();
        let mut descriptor = ComponentDescriptor::new(ComponentRef::module("parallel"));
        descriptor.execution = ExecutionPolicy::Concurrent {
            max_in_flight: limit,
        };
        hub.install(
            &descriptor,
            HandlerRegistration::Concurrent {
                max_in_flight: limit,
                handler: Arc::new(ParallelEntry(started)),
                subscriptions: vec![SubscriptionSpec::new(1, descriptor.component(), "test")],
            },
        )
        .unwrap();
        let requests: Vec<_> = (0..4usize)
            .map(|value| {
                hub.request::<_, usize>("test", dest("parallel"), value, MessageOptions::default())
                    .unwrap()
            })
            .collect();
        for request in requests {
            request.wait(Duration::from_secs(5)).await.unwrap();
        }
        close(&hub, &tracker).await;
    }
}

fn install_dynamic(hub: &EventHub, factory: impl HandlerFactory + 'static) {
    let mut d = ComponentDescriptor::new(ComponentRef::module("creating"));
    let limit = NonZeroUsize::new(1).unwrap();
    d.execution = ExecutionPolicy::Dynamic {
        max_lanes: limit,
        idle_timeout: Duration::from_secs(1),
    };
    hub.install(
        &d,
        HandlerRegistration::Dynamic {
            max_lanes: limit,
            idle_timeout: Duration::from_secs(1),
            factory: Box::new(factory),
            subscriptions: vec![SubscriptionSpec::new(900, d.component(), "dynamic")],
        },
    )
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_lane_construction_does_not_hold_routing_lock_and_reserves_its_slot() {
    let (hub, tracker, _) = hub();
    let started = Arc::new(Semaphore::new(0));
    let signal = started.clone();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    install_dynamic(&hub, move |_| {
        signal.add_permits(1);
        released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        Ok(Box::new(Recorder {
            seen: Arc::default(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    serial(
        &hub,
        "other",
        901,
        4,
        Recorder {
            seen: Arc::default(),
            gate: None,
        },
    );
    let producer = hub.clone();
    let create = tokio::task::spawn_blocking(move || {
        producer.command(
            "dynamic",
            dest("creating"),
            1usize,
            MessageOptions::default(),
        )
    });
    started.acquire().await.unwrap().forget();
    let other = hub.clone();
    let response = tokio::time::timeout(Duration::from_secs(1), async move {
        other
            .request::<_, usize>("test/x", dest("other"), 1usize, MessageOptions::default())?
            .wait(Duration::from_secs(1))
            .await
    })
    .await;
    release.send(()).unwrap();
    assert_eq!(*response.unwrap().unwrap(), 2);
    create.await.unwrap().unwrap();
    close(&hub, &tracker).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pending_lane_is_bounded_and_unsubscribe_discards_its_constructed_handler() {
    let (hub, tracker, _) = hub();
    let started = Arc::new(Semaphore::new(0));
    let signal = started.clone();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    install_dynamic(&hub, move |_| {
        signal.add_permits(1);
        released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        Ok(Box::new(Recorder {
            seen: Arc::default(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    let producer = hub.clone();
    let create = tokio::task::spawn_blocking(move || {
        producer.command(
            "dynamic",
            dest("creating"),
            1usize,
            MessageOptions::default(),
        )
    });
    started.acquire().await.unwrap().forget();
    assert_eq!(hub.mailbox_metrics()[0].preparing_lanes, 1);
    assert!(matches!(
        hub.command(
            "dynamic",
            dest("creating"),
            2usize,
            MessageOptions {
                lane_key: Some("another".into()),
                ..MessageOptions::default()
            }
        ),
        Err(FrameworkError::Backpressure(_))
    ));
    hub.unsubscribe(900).unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        create.await.unwrap(),
        Err(FrameworkError::Unroutable(_))
    ));
    assert_eq!(hub.mailbox_metrics()[0].active_lanes, 0);
    assert_eq!(hub.mailbox_metrics()[0].preparing_lanes, 0);
    assert_eq!(tracker.pending().operations, 0);
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn lane_construction_that_exceeds_admission_deadline_never_enqueues() {
    let (hub, tracker, _) = hub();
    let seen = Arc::new(Mutex::new(vec![]));
    let output = seen.clone();
    install_dynamic(&hub, move |_| {
        std::thread::sleep(Duration::from_millis(10));
        Ok(Box::new(Recorder {
            seen: output.clone(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    assert!(matches!(
        hub.command_wait(
            "dynamic",
            dest("creating"),
            1usize,
            MessageOptions::default(),
            Duration::from_millis(1)
        )
        .await,
        Err(FrameworkError::AdmissionTimeout)
    ));
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(hub.mailbox_metrics()[0].preparing_lanes, 0);
    assert_eq!(hub.mailbox_metrics()[0].active_lanes, 0);
    close(&hub, &tracker).await;
}
#[tokio::test]
async fn panicking_lane_factory_clears_reservation_and_can_retry() {
    let (hub, tracker, _) = hub();
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    install_dynamic(&hub, move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("factory fault");
        }
        Ok(Box::new(Recorder {
            seen: Arc::default(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    assert!(matches!(
        hub.command(
            "dynamic",
            dest("creating"),
            1usize,
            MessageOptions::default()
        ),
        Err(FrameworkError::ExecutionFailed {
            phase: Phase::Setup,
            ..
        })
    ));
    assert_eq!(hub.mailbox_metrics()[0].preparing_lanes, 0);
    hub.command(
        "dynamic",
        dest("creating"),
        2usize,
        MessageOptions::default(),
    )
    .unwrap();
    close(&hub, &tracker).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_recovery_waits_for_construction_and_never_installs_a_stale_lane() {
    let (hub, tracker, _) = hub();
    let started = Arc::new(Semaphore::new(0));
    let signal = started.clone();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    install_dynamic(&hub, move |_| {
        signal.add_permits(1);
        released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        Ok(Box::new(Recorder {
            seen: Arc::default(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    let producer = hub.clone();
    let create = tokio::task::spawn_blocking(move || {
        producer.command(
            "dynamic",
            dest("creating"),
            1usize,
            MessageOptions::default(),
        )
    });
    started.acquire().await.unwrap().forget();
    let owners = std::collections::BTreeSet::from([ComponentRef::module("creating")]);
    drop(tracker.close_owners(&owners));
    assert_eq!(tracker.pending_owners(&owners).operations, 1);
    release.send(()).unwrap();
    assert!(matches!(create.await.unwrap(), Err(FrameworkError::Closed)));
    tracker.drain_owners(&owners).await;
    hub.remove_owners(&owners).await;
    tracker.advance_owners(&owners).unwrap();
    tracker.open_owners(&owners).unwrap();
    install_dynamic(&hub, |_| {
        Ok(Box::new(Recorder {
            seen: Arc::default(),
            gate: None,
        }) as Box<dyn SerialHandler>)
    });
    assert_eq!(
        *hub.request::<_, usize>(
            "dynamic",
            dest("creating"),
            5usize,
            MessageOptions::default()
        )
        .unwrap()
        .wait(Duration::from_secs(1))
        .await
        .unwrap(),
        6
    );
    close(&hub, &tracker).await;
}
