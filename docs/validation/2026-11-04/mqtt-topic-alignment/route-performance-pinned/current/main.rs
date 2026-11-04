
extern crate self as modorch_core;
use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;
use std::time::Instant;
type ComponentRef = String;
type SubscriptionId = u64;
#[derive(Debug)] pub enum FrameworkError { InvalidConfig(String) }
pub type FrameworkResult<T> = Result<T, FrameworkError>;
#[derive(Clone, PartialEq, Eq)] enum Destination { Broadcast, Component(ComponentRef), Named(String) }
#[derive(Clone)] enum DestinationPattern { Any, Exact(Destination) }
#[derive(Clone)] struct SubscriptionSpec { id: SubscriptionId, target: ComponentRef, topic_pattern: String, destination_pattern: DestinationPattern }
mod topic;
mod routing;
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status").unwrap().lines()
        .find(|line| line.starts_with("VmRSS:")).unwrap()
        .split_whitespace().nth(1).unwrap().parse().unwrap()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let entries: u64 = args[1].parse().unwrap();
    let iterations: u64 = args[2].parse().unwrap();
    let wildcard = args[3] == "wildcard";
    let destination = if args[4] == "directed" { Destination::Component("c42".into()) } else { Destination::Broadcast };
    let mut index = routing::RouteIndex::default();
    let before = rss();
    for id in 0..entries {
        let pattern = format!("sensor/{id}/{}", if wildcard { "+" } else { "v1" });
        let spec = SubscriptionSpec { id, target: format!("c{id}"), topic_pattern: pattern.clone(), destination_pattern: DestinationPattern::Any };
        index.insert(id, spec, topic::TopicFilter::parse(&pattern).unwrap());
    }
    let retained = rss().saturating_sub(before);
    let topic = black_box("sensor/42/v1");
    for _ in 0..100 { assert_eq!(index.matching(topic, &destination).count(), 1); }
    let start = Instant::now();
    for _ in 0..iterations {
        assert_eq!(black_box(index.matching(black_box(topic), black_box(&destination)).count()), 1);
    }
    let nanos = start.elapsed().as_nanos() as f64 / iterations as f64;
    for id in 0..entries { assert!(index.remove(&id).is_some()); }
    assert_eq!(index.len(), 0);
    assert_eq!(index.matching(topic, &destination).count(), 0);
    println!("{{\"ns_per_lookup\":{nanos},\"rss_index_growth_kib\":{retained}}}");
}
