use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub execution: String,
    pub policy: String,
    pub tokio_workers: usize,
    pub owners: usize,
    pub producers_per_kind: usize,
    pub capacity: usize,
    pub concurrency: usize,
    pub lanes: usize,
    pub subscriptions: usize,
    pub cpu_threads: usize,
    pub cpu_capacity: usize,
    pub io_threads: usize,
    pub io_capacity: usize,
    pub task_capacity: usize,
    pub owner_tasks: usize,
    pub waiters: usize,
    pub owner_waiters: usize,
    pub seconds: u64,
    pub messages: u64,
    pub cycles: u32,
    pub idle_ms: u64,
    pub lane_idle_ms: u64,
    pub recover: bool,
    pub payload_bytes: usize,
    pub payload_budget: usize,
    pub cpu_iterations: u64,
    pub async_ms: u64,
    pub blocking_ms: u64,
    pub timeout_ms: u64,
    pub sample_ms: u64,
    /// Per-kind cumulative upper-bucket P99 limits; zero disables the check.
    pub max_p99_us: [u64; 4],
    pub max_rejection_percent: u64,
    pub max_timer_lag_us: u64,
    /// Compare last idle RSS with first idle RSS after pools have warmed up.
    pub max_idle_rss_growth_kib: u64,
    pub max_threads: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            execution: "isolated".into(),
            policy: "concurrent".into(),
            tokio_workers: 4,
            owners: 2,
            producers_per_kind: 2,
            capacity: 8,
            concurrency: 16,
            lanes: 32,
            subscriptions: 1,
            cpu_threads: 2,
            cpu_capacity: 4,
            io_threads: 4,
            io_capacity: 8,
            task_capacity: 32,
            owner_tasks: 8,
            waiters: 64,
            owner_waiters: 32,
            seconds: 5,
            messages: 0,
            cycles: 1,
            idle_ms: 250,
            lane_idle_ms: 50,
            recover: false,
            payload_bytes: 1024,
            payload_budget: 64 * 1024 * 1024,
            cpu_iterations: 100_000,
            async_ms: 2,
            blocking_ms: 5,
            timeout_ms: 5000,
            sample_ms: 100,
            max_p99_us: [0; 4],
            max_rejection_percent: 100,
            max_timer_lag_us: 0,
            max_idle_rss_growth_kib: 0,
            max_threads: 0,
        }
    }
}
impl Config {
    pub fn parse(args: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        if args.len() % 2 != 0 {
            return Err("arguments require --name value pairs".into());
        }
        let mut config = Self::default();
        let profiles: Vec<_> = args.chunks(2).filter(|a| a[0] == "--profile").collect();
        if profiles.len() > 1 {
            return Err("only one --profile is allowed".into());
        }
        if let Some(profile) = profiles.first() {
            config = toml::from_str(&std::fs::read_to_string(&profile[1])?)?;
        }
        let mut values = serde_json::to_value(config)?;
        let mut seen = std::collections::BTreeSet::new();
        for arg in args.chunks(2).filter(|a| a[0] != "--profile") {
            let key = arg[0]
                .strip_prefix("--")
                .ok_or("expected --option")?
                .replace('-', "_");
            if !seen.insert(key.clone()) {
                return Err(format!("duplicate option: {key}").into());
            }
            let current = values
                .get_mut(&key)
                .ok_or_else(|| format!("unknown option: {}", arg[0]))?;
            *current = if current.is_string() {
                arg[1].clone().into()
            } else {
                serde_json::from_str(&arg[1])?
            };
        }
        let config: Self = serde_json::from_value(values)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        let producers = self
            .owners
            .checked_mul(self.producers_per_kind)
            .and_then(|n| n.checked_mul(4))
            .ok_or("producer overflow")?;
        let bytes = producers
            .checked_mul(self.payload_bytes)
            .ok_or("payload overflow")?;
        if !matches!(self.execution.as_str(), "inline" | "isolated")
            || !matches!(self.policy.as_str(), "concurrent" | "dynamic")
            || self.owners == 0
            || self.owners > 64
            || self.producers_per_kind == 0
            || producers > 4096
            || self.tokio_workers == 0
            || self.tokio_workers > 64
            || self.capacity == 0
            || self.capacity > 65536
            || self.concurrency == 0
            || self.concurrency > 4096
            || self.lanes == 0
            || self.lanes > 4096
            || self.subscriptions == 0
            || self
                .subscriptions
                .checked_mul(self.owners)
                .is_none_or(|n| n > 65536)
            || self.seconds == 0
            || self.seconds > 86400
            || self.cycles == 0
            || self.cycles > 10000
            || self.messages >= (1 << 48)
            || self.timeout_ms == 0
            || self.timeout_ms > 300_000
            || self.sample_ms == 0
            || self.sample_ms > 60_000
            || self.lane_idle_ms == 0
            || self.lane_idle_ms > 60_000
            || self.idle_ms > 300_000
            || self.payload_bytes == 0
            || self.payload_bytes > 16 * 1024 * 1024
            || bytes > self.payload_budget
            || self.cpu_iterations == 0
            || self.cpu_iterations > 100_000_000
            || self.async_ms > 60_000
            || self.blocking_ms > 60_000
            || self.max_rejection_percent > 100
            || (self.max_idle_rss_growth_kib > 0 && self.cycles < 2)
            || (self.policy == "dynamic" && self.idle_ms < self.lane_idle_ms * 2)
        {
            return Err("invalid workload bounds (including producer payload budget and dynamic idle window)".into());
        }
        self.resources().validate()?;
        if self.task_capacity == 0 || self.task_capacity > 65536 {
            return Err("invalid task capacity".into());
        }
        Ok(())
    }
    pub fn resources(&self) -> modorch::ExecutionResources {
        modorch::ExecutionResources {
            cpu_threads: self.cpu_threads,
            cpu_capacity: self.cpu_capacity,
            blocking_io_threads: self.io_threads,
            blocking_io_capacity: self.io_capacity,
            max_tasks_per_owner: self.owner_tasks,
            max_waiters: self.waiters,
            max_waiters_per_owner: self.owner_waiters,
            ..Default::default()
        }
    }
}
