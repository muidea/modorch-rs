use serde_json::Value;
use std::process::{Command, Output};

fn run(extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mixed-load"))
        .args([
            "--messages",
            "4",
            "--producers-per-kind",
            "1",
            "--idle-ms",
            "150",
        ])
        .args(extra)
        .output()
        .unwrap()
}
fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("missing JSON: {}", String::from_utf8_lossy(&output.stderr)))
}
#[test]
fn all_owners_complete_all_work_classes_and_release_resources() {
    let output = run(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["completed"], 32);
    assert_eq!(data["replies"], 32);
    assert_eq!(data["owners"].as_array().unwrap().len(), 8);
    assert_eq!(data["final_resources"]["cpu"]["completed"], 8);
    assert_eq!(data["final_resources"]["blocking_io"]["completed"], 8);
    assert_eq!(data["after"]["pending_operations"], 0);
    assert_eq!(data["after"]["live_payload_bytes"], 0);
    assert_eq!(data["after"]["live_handlers"], 0);
}
#[test]
fn dynamic_idle_and_recovery_cycles_reclaim_handlers_and_payloads() {
    let output = run(&["--cycles", "2", "--policy", "dynamic", "--recover", "true"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["recoveries"], 2);
    assert_eq!(data["completed"], 64);
    for idle in data["idle_samples"].as_array().unwrap() {
        for field in [
            "live_payload_bytes",
            "live_handlers",
            "active_lanes",
            "preparing_lanes",
            "worker_controls",
            "tasks",
            "pending_operations",
        ] {
            assert_eq!(idle[field], 0, "{field}: {idle}");
        }
    }
}
#[test]
fn exceeded_latency_slo_returns_failure_with_a_reviewable_report() {
    let output = run(&["--max-p99-us", "[0,1,0,0]"]);
    assert!(!output.status.success());
    let data = report(&output);
    assert_eq!(data["passed"], false);
    assert!(
        data["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("P99"))
    );
    assert_eq!(data["after"]["pending_operations"], 0);
}
#[test]
fn invalid_or_unbounded_workload_configuration_is_rejected_before_start() {
    for args in [
        vec!["--payload-budget", "1"],
        vec!["--unknown", "1"],
        vec!["--max-rejection-percent", "101"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
