#!/usr/bin/env python3
"""Compare actual RouteIndex sources in an isolated rustc harness; not an E2E SLO."""
import argparse
import hashlib
import json
import pathlib
import statistics
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[4]
BASELINE = "ccc77ce9501cd3e942677bba51b6ff3efb1c63f6"
HARNESS = r'''
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
        INSERT
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
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    commands = []
    def run(command, cwd):
        result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, check=False, timeout=120)
        commands.append({"command": command, "cwd": str(cwd), "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        if result.returncode:
            raise RuntimeError(result.stderr)
        return result.stdout
    rows = []
    with tempfile.TemporaryDirectory(prefix="modorch-topic-routes-") as temporary:
        build = pathlib.Path(temporary)
        for label in ("baseline", "current"):
            source = args.output / label
            source.mkdir()
            route = "crates/modorch-eventhub/src/routing.rs"
            (source / "routing.rs").write_bytes(subprocess.check_output(["git", "show", f"{BASELINE}:{route}"], cwd=ROOT) if label == "baseline" else (ROOT / route).read_bytes())
            (source / "topic.rs").write_bytes((ROOT / "crates/modorch-eventhub/src/topic.rs").read_bytes())
            insert = "index.insert(id, spec);" if label == "baseline" else "index.insert(id, spec, topic::TopicFilter::parse(&pattern).unwrap());"
            (source / "main.rs").write_text(HARNESS.replace("INSERT", insert))
            run(["rustc", "--edition=2024", "-O", "-Adead_code", str((source / "main.rs").resolve()), "-o", str(build / label)], ROOT)
        for entries in (64, 4096):
            for mode in ("exact", "wildcard"):
                for destination in ("broadcast", "directed"):
                    iterations = 1000 if mode == "wildcard" and destination == "broadcast" else 100_000
                    for repeat in range(5):
                        # Alternate order to reduce fixed warmup/order bias.
                        for label in (("baseline", "current") if repeat % 2 == 0 else ("current", "baseline")):
                            row = json.loads(run([str(build / label), str(entries), str(iterations), mode, destination], ROOT))
                            rows.append(dict(label=label, entries=entries, mode=mode, destination=destination, iterations=iterations, repeat=repeat, **row))
    summary = []
    for entries in (64, 4096):
        for mode in ("exact", "wildcard"):
            for destination in ("broadcast", "directed"):
                selection = [r for r in rows if (r["entries"], r["mode"], r["destination"]) == (entries, mode, destination)]
                item = dict(entries=entries, mode=mode, destination=destination)
                for label in ("baseline", "current"):
                    runs = [r for r in selection if r["label"] == label]
                    item[label + "_median_ns"] = statistics.median(r["ns_per_lookup"] for r in runs)
                    item[label + "_median_rss_kib"] = statistics.median(r["rss_index_growth_kib"] for r in runs)
                item["time_ratio"] = item["current_median_ns"] / item["baseline_median_ns"]
                summary.append(item)
    files = {str(path.relative_to(args.output)): hashlib.sha256(path.read_bytes()).hexdigest() for path in args.output.rglob("*.rs")}
    report = dict(status="Passed", baseline_commit=BASELINE, versions=run(["rustc", "-Vv"], ROOT), sources_sha256=files, limits=["Actual routing.rs included with minimal DTO/error stubs; not full EventHub, lock contention or payload cost", "Linux RSS deltas include allocator/page effects, not precise retained heap", "Five samples per scenario, no SLO or cross-platform inference"], summary=summary, runs=rows, commands=commands)
    (args.output / "results.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
