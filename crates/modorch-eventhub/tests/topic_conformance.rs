use async_trait::async_trait;
use modorch_core::*;
use modorch_eventhub::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "support/topic_vectors.rs"]
mod topic_vectors;

struct Recorder(Arc<Mutex<Vec<String>>>);
#[async_trait]
impl SerialHandler for Recorder {
    async fn handle(
        &mut self,
        _: &HandlerContext,
        message: MessageEnvelope,
    ) -> FrameworkResult<HandlerReply> {
        self.0.lock().unwrap().push(message.topic);
        Ok(Arc::new(()))
    }
}
fn hub(max_topic_bytes: usize) -> (EventHub, OperationTracker) {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let hub = EventHub::with_limits(
        tracker.clone(),
        FailureReporter::new(),
        HubLimits {
            max_topic_bytes,
            ..Default::default()
        },
    )
    .unwrap();
    (hub, tracker)
}
fn install(
    hub: &EventHub,
    target: &ComponentRef,
    subscriptions: Vec<SubscriptionSpec>,
    seen: &Arc<Mutex<Vec<String>>>,
) -> FrameworkResult<()> {
    let mut descriptor = ComponentDescriptor::new(target.clone());
    descriptor.queue_capacity = 128;
    hub.install(
        &descriptor,
        HandlerRegistration::Serial {
            handler: Box::new(Recorder(seen.clone())),
            subscriptions,
        },
    )
}
async fn close(hub: &EventHub, tracker: &OperationTracker) {
    tracker.close();
    tokio::time::timeout(Duration::from_secs(5), tracker.drain())
        .await
        .unwrap();
    hub.terminate(false).await;
}
fn invalid<T>(result: FrameworkResult<T>) {
    assert!(matches!(result, Err(FrameworkError::InvalidConfig(_))));
}
fn limited<T>(result: FrameworkResult<T>) {
    assert!(matches!(result, Err(FrameworkError::ResourceLimit(_))));
}

#[tokio::test]
async fn mqtt_vectors_match_real_subscription_and_delivery() {
    let (hub, tracker) = hub(65_536);
    let seen = Arc::default();
    let target = ComponentRef::module("conformance");
    install(&hub, &target, vec![], &seen).unwrap();
    for &(id, filter, topic, filter_valid, name_valid, matched) in topic_vectors::VECTORS {
        assert_eq!(topic_matches(filter, topic), matched, "helper: {id}");
        let subscription = hub.subscribe(SubscriptionSpec::new(1, target.clone(), filter));
        assert_eq!(subscription.is_ok(), filter_valid, "subscribe: {id}");
        if !filter_valid {
            invalid(subscription);
        }
        let report = hub.publish(
            topic,
            Destination::Broadcast,
            (),
            Default::default(),
            PublishPolicy::Reject,
        );
        if !name_valid {
            invalid(report);
        } else if matched {
            assert_eq!(report.unwrap().accepted(), 1, "delivery: {id}");
        } else {
            assert!(
                matches!(report, Err(FrameworkError::Unroutable(_))),
                "delivery: {id}"
            );
        }
        tracker.drain().await;
        let delivered = std::mem::take(&mut *seen.lock().unwrap());
        assert_eq!(
            delivered,
            if matched {
                vec![topic.to_owned()]
            } else {
                vec![]
            },
            "handler: {id}"
        );
        if filter_valid {
            hub.unsubscribe(1).unwrap();
        }
    }
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn mqtt_utf8_byte_ceiling_is_separate_from_local_budget() {
    let (hub, tracker) = hub(65_536);
    let target = ComponentRef::module("boundary");
    let seen = Arc::default();
    install(&hub, &target, vec![], &seen).unwrap();
    // Both ASCII and multi-byte names land exactly on the encoded-byte ceiling.
    for name in ["a".repeat(65_535), "界".repeat(21_845)] {
        assert_eq!(name.len(), 65_535);
        assert!(topic_matches(&name, &name));
        hub.subscribe(SubscriptionSpec::new(1, target.clone(), &name))
            .unwrap();
        assert_eq!(
            hub.publish(
                &name,
                Destination::Broadcast,
                (),
                Default::default(),
                PublishPolicy::Reject
            )
            .unwrap()
            .accepted(),
            1
        );
        tracker.drain().await;
        hub.unsubscribe(1).unwrap();
        let overflow = name + "a";
        assert!(!topic_matches("#", &overflow));
        assert!(!topic_matches(&overflow, "a"));
        invalid(hub.subscribe(SubscriptionSpec::new(1, target.clone(), &overflow)));
        invalid(hub.command(&overflow, Destination::Broadcast, (), Default::default()));
    }
    close(&hub, &tracker).await;
    let (hub, tracker) = self::hub(4);
    install(&hub, &target, vec![], &seen).unwrap();
    let before = hub
        .subscribe(SubscriptionSpec::new(1, target.clone(), "界a"))
        .unwrap();
    hub.command("界a", Destination::Broadcast, (), Default::default())
        .unwrap();
    assert!(topic_matches("界ab", "界ab"));
    limited(hub.subscribe(SubscriptionSpec::new(2, target.clone(), "界ab")));
    limited(hub.publish(
        "界ab",
        Destination::Broadcast,
        (),
        Default::default(),
        PublishPolicy::Reject,
    ));
    limited(
        hub.command_wait(
            "界ab",
            Destination::Broadcast,
            (),
            Default::default(),
            Duration::from_secs(1),
        )
        .await,
    );
    invalid(hub.command("bad/+", Destination::Broadcast, (), Default::default()));
    let after = hub.unsubscribe(1).unwrap();
    assert_eq!(after.generation, before.generation + 1);
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn mqtt_invalid_names_are_rejected_by_every_ingress() {
    let (hub, tracker) = hub(65_536);
    let external = hub.external();
    let wait = Duration::from_secs(1);
    for name in ["", "a/+", "a/#", "a/\0", &"x".repeat(65_536)] {
        invalid(hub.publish(
            name,
            Destination::Broadcast,
            (),
            Default::default(),
            PublishPolicy::Reject,
        ));
        invalid(hub.command(name, Destination::Broadcast, (), Default::default()));
        invalid(hub.request::<_, ()>(name, Destination::Broadcast, (), Default::default()));
        invalid(
            hub.publish_wait(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
        invalid(
            hub.command_wait(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
        invalid(
            hub.request_wait::<_, ()>(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
        invalid(external.publish(
            name,
            Destination::Broadcast,
            (),
            Default::default(),
            PublishPolicy::Reject,
        ));
        invalid(external.command(name, Destination::Broadcast, (), Default::default()));
        invalid(external.request::<_, ()>(name, Destination::Broadcast, (), Default::default()));
        invalid(
            external
                .publish_wait(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
        invalid(
            external
                .command_wait(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
        invalid(
            external
                .request_wait::<_, ()>(name, Destination::Broadcast, (), Default::default(), wait)
                .await,
        );
    }
    assert_eq!(hub.metrics().accepted, 0);
    assert_eq!(hub.metrics().admission.waiters, 0);
    assert_eq!(tracker.pending().operations, 0);
    close(&hub, &tracker).await;
}

#[tokio::test]
async fn mqtt_invalid_subscriptions_leave_registry_and_generation_unchanged() {
    let (hub, tracker) = hub(1024);
    let seen = Arc::default();
    let target = ComponentRef::module("atomic");
    // Failed initial installation must not leave its first subscription or endpoint.
    invalid(install(
        &hub,
        &target,
        vec![
            SubscriptionSpec::new(1, target.clone(), "a/#"),
            SubscriptionSpec::new(2, target.clone(), "a/#/b"),
        ],
        &seen,
    ));
    install(&hub, &target, vec![], &seen).unwrap();
    let first = hub
        .subscribe(SubscriptionSpec::new(1, target.clone(), "a/#"))
        .unwrap();
    for filter in ["", "a/#/b", "a+", "a/+x", "a/b#", "a/\0"] {
        invalid(hub.subscribe(SubscriptionSpec::new(1, target.clone(), filter)));
        invalid(hub.subscribe(SubscriptionSpec::new(2, target.clone(), filter)));
    }
    assert_eq!(
        hub.publish(
            "a",
            Destination::Broadcast,
            (),
            Default::default(),
            PublishPolicy::Reject
        )
        .unwrap()
        .accepted(),
        1
    );
    let second = hub
        .subscribe(SubscriptionSpec::new(2, target, "a/+"))
        .unwrap();
    assert_eq!(second.generation, first.generation + 1);
    hub.unsubscribe(1).unwrap();
    hub.unsubscribe(2).unwrap();
    assert!(matches!(
        hub.command("a/", Destination::Broadcast, (), Default::default()),
        Err(FrameworkError::Unroutable(_))
    ));
    close(&hub, &tracker).await;
    assert_eq!(seen.lock().unwrap().as_slice(), ["a"]);
}

#[tokio::test]
async fn mqtt_destination_lane_and_component_dedup_remain_orthogonal() {
    let (hub, tracker) = hub(1024);
    let target = ComponentRef::module("selected");
    let other = ComponentRef::module("other");
    let seen = Arc::default();
    let other_seen = Arc::default();
    install(
        &hub,
        &target,
        vec![
            SubscriptionSpec::new(1, target.clone(), "a/#"),
            SubscriptionSpec::new(2, target.clone(), "a/+"),
            SubscriptionSpec::new(3, target.clone(), "a/"),
        ],
        &seen,
    )
    .unwrap();
    let mut spec = SubscriptionSpec::new(4, other.clone(), "a/#");
    spec.destination_pattern = DestinationPattern::Exact(Destination::Named("private".into()));
    install(&hub, &other, vec![spec], &other_seen).unwrap();
    for key in ["first", "second"] {
        let options = MessageOptions {
            lane_key: Some(key.into()),
            ..Default::default()
        };
        assert_eq!(
            hub.publish(
                "a/",
                Destination::Broadcast,
                (),
                options.clone(),
                PublishPolicy::Reject
            )
            .unwrap()
            .accepted(),
            1
        );
        hub.command(
            "a/",
            Destination::Component(target.clone()),
            (),
            options.clone(),
        )
        .unwrap();
        hub.request::<_, ()>("a/", Destination::Component(target.clone()), (), options)
            .unwrap()
            .wait(Duration::from_secs(1))
            .await
            .unwrap();
    }
    assert_eq!(
        hub.publish(
            "a/",
            Destination::Named("private".into()),
            (),
            Default::default(),
            PublishPolicy::Reject
        )
        .unwrap()
        .accepted(),
        2
    );
    assert!(matches!(
        hub.command(
            "a/",
            Destination::Named("private".into()),
            (),
            Default::default()
        ),
        Err(FrameworkError::AmbiguousDestination(_))
    ));
    close(&hub, &tracker).await;
    assert_eq!(seen.lock().unwrap().len(), 7);
    assert_eq!(other_seen.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mqtt_subscription_churn_never_leaves_stale_routes() {
    let (hub, tracker) = hub(1024);
    let target = ComponentRef::module("churn");
    let seen = Arc::default();
    install(&hub, &target, vec![], &seen).unwrap();
    let producer_hub = hub.clone();
    let producer = tokio::spawn(async move {
        for _ in 0..500 {
            for name in ["a/", "a/:id", "$SYS/x", "a//x"] {
                let result =
                    producer_hub.command(name, Destination::Broadcast, (), Default::default());
                assert!(
                    result.is_ok()
                        || matches!(
                            result,
                            Err(FrameworkError::Unroutable(_) | FrameworkError::Backpressure(_))
                        )
                );
            }
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..50 {
        for (filter, name) in [
            ("a/+", "a/"),
            ("a/:id", "a/:id"),
            ("$SYS/#", "$SYS/x"),
            ("a//#", "a//x"),
        ] {
            hub.subscribe(SubscriptionSpec::new(1, target.clone(), filter))
                .unwrap();
            tokio::task::yield_now().await;
            hub.unsubscribe(1).unwrap();
            assert!(matches!(
                hub.command(name, Destination::Broadcast, (), Default::default()),
                Err(FrameworkError::Unroutable(_))
            ));
        }
    }
    producer.await.unwrap();
    close(&hub, &tracker).await;
}
