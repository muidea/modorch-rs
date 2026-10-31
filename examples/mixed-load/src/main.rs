//! Bounded mixed-workload, burst/idle and recovery acceptance executable.
mod config;
mod stats;
mod workload;
use config::Config;
use modorch::*;
use serde_json::{Value, json};
use stats::{Counters, Histogram, Payload, process_sample, resource_json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use workload::{Factory, Job, KINDS, Reply, cpu_work, owner};

#[derive(Default)]
struct Row {
    attempts: u64,
    replies: u64,
    successful: u64,
    admission_rejected: u64,
    resource_rejected: u64,
    latency: Histogram,
    all_latency: Histogram,
    error: Option<String>,
}
impl Row {
    fn merge(&mut self, other: Self) {
        self.attempts += other.attempts;
        self.replies += other.replies;
        self.successful += other.successful;
        self.admission_rejected += other.admission_rejected;
        self.resource_rejected += other.resource_rejected;
        self.latency.merge(&other.latency);
        self.all_latency.merge(&other.all_latency);
        if other.error.is_some() {
            self.error = other.error;
        }
    }
    fn json(&self) -> Value {
        json!({"attempts":self.attempts,"replies":self.replies,"successful":self.successful,
            "admission_rejected":self.admission_rejected,"resource_rejected":self.resource_rejected,
            "successful_latency":self.latency.json(),"all_attempt_latency":self.all_latency.json(),"error":self.error})
    }
}
fn capacity_error(error: &FrameworkError) -> bool {
    matches!(
        error.root_cause(),
        FrameworkError::Backpressure(_)
            | FrameworkError::ResourceLimit(_)
            | FrameworkError::AdmissionTimeout
    )
}
async fn producer(
    config: Arc<Config>,
    counters: Arc<Counters>,
    hub: EventHub,
    owner_index: usize,
    kind: usize,
    producer_index: usize,
    expected: u64,
) -> Row {
    let mut row = Row::default();
    let deadline = Instant::now() + Duration::from_secs(config.seconds);
    let timeout = Duration::from_millis(config.timeout_ms);
    let destination = Destination::Component(owner(owner_index));
    while if config.messages == 0 {
        Instant::now() < deadline
    } else {
        row.attempts < config.messages
    } {
        let started = Instant::now();
        let sequence = ((producer_index as u64) << 48) | row.attempts;
        row.attempts += 1;
        let job = Job {
            sequence,
            kind,
            payload: Payload::new(config.payload_bytes, counters.clone()),
        };
        let options = MessageOptions {
            payload_bytes: Some(config.payload_bytes),
            lane_key: Some(format!(
                "lane-{}",
                (kind * config.producers_per_kind + producer_index) % config.lanes
            )),
            ..Default::default()
        };
        let response = match hub
            .request_wait::<_, Reply>("mixed/v1", destination.clone(), job, options, timeout)
            .await
        {
            Ok(request) => request.wait(timeout).await,
            Err(error) if capacity_error(&error) => {
                row.admission_rejected += 1;
                row.all_latency.record(started.elapsed());
                tokio::task::yield_now().await;
                continue;
            }
            Err(error) => Err(error),
        };
        row.all_latency.record(started.elapsed());
        match response {
            Ok(reply) if reply.sequence == sequence => {
                row.replies += 1;
                match &reply.result {
                    Ok(value) if *value == expected => {
                        row.successful += 1;
                        row.latency.record(started.elapsed());
                    }
                    Err(error) if capacity_error(error) => row.resource_rejected += 1,
                    _ => {
                        row.error = Some("business result/correlation mismatch".into());
                        break;
                    }
                }
            }
            Ok(_) => {
                row.error = Some("reply sequence mismatch".into());
                break;
            }
            Err(error) => {
                row.error = Some(error.to_string());
                break;
            }
        }
    }
    row
}
fn snapshot(handle: &ApplicationHandle, counters: &Counters, elapsed: Duration) -> Value {
    let m = handle.metrics();
    json!({"seconds":elapsed.as_secs_f64(),"process":process_sample(),
        "live_payload_bytes":counters.live_bytes.load(Ordering::Relaxed),
        "live_handlers":counters.handlers.load(Ordering::Relaxed),
        "created_handlers":counters.created_handlers.load(Ordering::Relaxed),
        "pending_operations":m.operations.operations,"queued_messages":m.hub.queued,
        "message_waiters":m.hub.admission.waiters,"message_waiting_bytes":m.hub.admission.waiting_bytes,
        "message_in_flight_bytes":m.hub.admission.in_flight_bytes,
        "tasks":m.tasks.tasks,"timers":m.tasks.timers,"available_task_slots":m.tasks.available_task_slots,
        "owner_resources":m.tasks.owner_resources.iter().map(|(owner,m)| json!({"owner":owner.id,
            "available_tasks":m.available_tasks,"capacity":m.task_capacity,"waiters":m.waiters,
            "async_queued":m.async_queued,"async_running":m.async_running,"cpu_queued":m.cpu_queued,"cpu_running":m.cpu_running,
            "blocking_io_queued":m.blocking_io_queued,"blocking_io_running":m.blocking_io_running})).collect::<Vec<_>>(),
        "cpu":resource_json(m.tasks.cpu),"blocking_io":resource_json(m.tasks.blocking_io),
        "active_lanes":m.mailboxes.iter().map(|m|m.active_lanes).sum::<usize>(),
        "preparing_lanes":m.mailboxes.iter().map(|m|m.preparing_lanes).sum::<usize>(),
        "worker_controls":m.hub.worker_tasks,"journal_entries":handle.lifecycle_journal().len(),"journal_dropped":m.journal_dropped})
}
fn push_bounded(samples: &mut VecDeque<Value>, sample: Value) {
    if samples.len() == 256 {
        samples.pop_front();
    }
    samples.push_back(sample);
}
async fn quiesce(
    handle: &ApplicationHandle,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(timeout, async {
        while handle.health().pending.operations != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| "work did not quiesce")?;
    Ok(())
}
async fn recover(
    handle: &ApplicationHandle,
    config: &Config,
    counters: &Arc<Counters>,
    index: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let component = owner(index);
    let before = handle.health().components[&component].generation;
    let timeout = Duration::from_millis(config.timeout_ms);
    handle
        .event_hub()
        .command_wait(
            "mixed/v1",
            Destination::Component(component.clone()),
            Job {
                sequence: 0,
                kind: 4,
                payload: Payload::new(config.payload_bytes, counters.clone()),
            },
            MessageOptions {
                payload_bytes: Some(config.payload_bytes),
                ..Default::default()
            },
            timeout,
        )
        .await?;
    tokio::time::timeout(timeout, async {
        loop {
            let health = handle.health();
            if health.components[&component].generation > before && health.ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "component did not recover: {:?}; {:?}",
            handle.health(),
            handle.task_metrics()
        )
    })?;
    Ok(())
}
async fn run(config: Arc<Config>) -> Result<(), Box<dyn std::error::Error>> {
    let counters = Arc::new(Counters::default());
    let mut builder = ApplicationBuilder::new("mixed-load")
        .task_limits(config.task_capacity, 4)
        .execution_resources(config.resources())
        .journal_capacity(64)
        .message_limits(HubLimits {
            max_waiters: config.waiters,
            max_waiters_per_target: config.owner_waiters,
            max_in_flight_bytes: Some(config.payload_budget),
            max_waiting_bytes: Some(config.payload_budget),
            max_message_bytes: Some(config.payload_bytes),
            ..Default::default()
        });
    let mut selection = ComponentSelection::new();
    for index in 0..config.owners {
        builder.register_module(Factory {
            index,
            config: config.clone(),
            counters: counters.clone(),
        })?;
        selection.enable_module(owner(index).id);
    }
    let handle = builder.selection(selection).build()?.start().await?;
    let started = Instant::now();
    let before = snapshot(&handle, &counters, started.elapsed());
    let mut rows: BTreeMap<(usize, usize), Row> = BTreeMap::new();
    let mut samples = VecDeque::new();
    let mut idle_samples = VecDeque::new();
    let mut timer_lag = Histogram::default();
    let mut peak_rss = 0;
    let mut peak_threads = 0;
    let mut first_idle_rss = None;
    let mut last_idle_rss = None;
    let mut errors = Vec::new();
    let mut recoveries = 0;
    let mut active_seconds = 0.;
    let cpu_expected = cpu_work(config.cpu_iterations);
    for cycle in 0..config.cycles {
        let phase_started = Instant::now();
        let mut jobs = tokio::task::JoinSet::new();
        for index in 0..config.owners {
            for kind in 0..4 {
                for producer_index in 0..config.producers_per_kind {
                    let config = config.clone();
                    let counters = counters.clone();
                    let hub = handle.event_hub();
                    jobs.spawn(async move {
                        (
                            index,
                            kind,
                            producer(
                                config,
                                counters,
                                hub,
                                index,
                                kind,
                                producer_index,
                                if kind == 2 { cpu_expected } else { 0x5a },
                            )
                            .await,
                        )
                    });
                }
            }
        }
        let mut ticks = tokio::time::interval(Duration::from_millis(config.sample_ms));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticks.tick().await;
        while !jobs.is_empty() {
            tokio::select! {
                result = jobs.join_next() => match result {
                    Some(Ok((index,kind,row))) => rows.entry((index,kind)).or_default().merge(row),
                    Some(Err(error)) => { errors.push(error.to_string()); jobs.abort_all(); },
                    None => {},
                },
                scheduled = ticks.tick() => {
                    timer_lag.record(tokio::time::Instant::now().saturating_duration_since(scheduled));
                    let sample = snapshot(&handle, &counters, started.elapsed());
                    peak_rss = peak_rss.max(sample["process"]["rss_kib"].as_u64().unwrap_or(0));
                    peak_threads = peak_threads.max(sample["process"]["threads"].as_u64().unwrap_or(0));
                    push_bounded(&mut samples, sample);
                }
            }
        }
        active_seconds += phase_started.elapsed().as_secs_f64();
        quiesce(&handle, Duration::from_millis(config.timeout_ms)).await?;
        if config.recover {
            recover(&handle, &config, &counters, cycle as usize % config.owners).await?;
            recoveries += 1;
            quiesce(&handle, Duration::from_millis(config.timeout_ms)).await?;
        }
        tokio::time::sleep(Duration::from_millis(config.idle_ms)).await;
        let mut idle = snapshot(&handle, &counters, started.elapsed());
        idle["cycle"] = cycle.into();
        let rss = idle["process"]["rss_kib"].as_u64();
        if first_idle_rss.is_none() {
            first_idle_rss = rss;
        }
        last_idle_rss = rss;
        peak_rss = peak_rss.max(rss.unwrap_or(0));
        peak_threads = peak_threads.max(idle["process"]["threads"].as_u64().unwrap_or(0));
        if idle["pending_operations"] != 0
            || idle["live_payload_bytes"] != 0
            || idle["tasks"] != 0
            || idle["message_waiters"] != 0
            || idle["message_in_flight_bytes"] != 0
            || idle["message_waiting_bytes"] != 0
            || idle["preparing_lanes"] != 0
            || idle["cpu"]["available"] != config.cpu_capacity
            || idle["blocking_io"]["available"] != config.io_capacity
            || idle["available_task_slots"] != config.task_capacity
            || (config.policy == "dynamic"
                && (idle["live_handlers"] != 0
                    || idle["active_lanes"] != 0
                    || idle["worker_controls"] != 0))
        {
            errors.push(format!("cycle {cycle}: idle resources not reclaimed"));
        }
        push_bounded(&mut idle_samples, idle);
        eprintln!(
            "cycle {}/{}: quiesced, {} recoveries",
            cycle + 1,
            config.cycles,
            recoveries
        );
        if !errors.is_empty() {
            break;
        }
    }
    let mut totals: [Row; 4] = std::array::from_fn(|_| Row::default());
    let owner_rows: Vec<_> = rows
        .into_iter()
        .map(|((index, kind), row)| {
            if row.successful == 0 {
                errors.push(format!(
                    "owner {index} {} made no successful progress",
                    KINDS[kind]
                ));
            }
            if (row.admission_rejected + row.resource_rejected) as u128 * 100
                > row.attempts as u128 * config.max_rejection_percent as u128
            {
                errors.push(format!(
                    "owner {index} {} rejection rate exceeded configured SLO",
                    KINDS[kind]
                ));
            }
            if let Some(error) = &row.error {
                errors.push(format!("owner {index} {}: {error}", KINDS[kind]));
            }
            if config.max_p99_us[kind] > 0 && row.latency.quantile(99) > config.max_p99_us[kind] {
                errors.push(format!(
                    "owner {index} {} P99 exceeded configured SLO",
                    KINDS[kind]
                ));
            }
            let value = json!({"owner":index,"kind":KINDS[kind],"stats":row.json()});
            totals[kind].merge(row);
            value
        })
        .collect();
    let idle_rss_growth = last_idle_rss
        .zip(first_idle_rss)
        .map(|(last, first)| last as i64 - first as i64);
    if config.max_idle_rss_growth_kib > 0
        && idle_rss_growth.is_none_or(|growth| growth > config.max_idle_rss_growth_kib as i64)
    {
        errors.push("idle RSS growth unavailable or exceeds configured SLO".into());
    }
    if config.max_threads > 0 && (peak_threads == 0 || peak_threads > config.max_threads as u64) {
        errors.push("thread count unavailable or exceeds configured SLO".into());
    }
    if config.max_timer_lag_us > 0
        && (timer_lag.count == 0 || timer_lag.quantile(99) > config.max_timer_lag_us)
    {
        errors.push("timer lag unavailable or exceeds configured SLO".into());
    }
    let replies: u64 = totals.iter().map(|r| r.replies).sum();
    if counters.completed.load(Ordering::Relaxed) as u64 != replies {
        errors.push("handler completion/reply counts disagree".into());
    }
    let final_resources = snapshot(&handle, &counters, started.elapsed());
    let shutdown_started = Instant::now();
    let shutdown = handle
        .shutdown_checked(ShutdownOptions {
            timeout: Duration::from_millis(config.timeout_ms),
        })
        .await?;
    let after = snapshot(&handle, &counters, started.elapsed());
    if shutdown != ShutdownReceipt::GracefullyStopped
        || after["pending_operations"] != 0
        || after["live_handlers"] != 0
        || after["live_payload_bytes"] != 0
    {
        errors.push("shutdown left live resources".into());
    }
    let kinds: BTreeMap<_, _> = KINDS
        .iter()
        .zip(&totals)
        .map(|(kind, row)| (*kind, row.json()))
        .collect();
    let output = json!({"schema_version":1,"config":*config,"wall_seconds":started.elapsed().as_secs_f64(),"active_seconds":active_seconds,
        "successful_per_active_second":totals.iter().map(|r|r.successful).sum::<u64>() as f64 / active_seconds,
        "completed":counters.completed.load(Ordering::Relaxed),"replies":replies,"recoveries":recoveries,
        "kinds":kinds,"owners":owner_rows,"timer_lag":timer_lag.json(),"rss_sample_peak_kib":peak_rss,
        "thread_sample_peak":peak_threads,"idle_rss_growth_kib":idle_rss_growth,
        "payload_peak_bytes":counters.peak_bytes.load(Ordering::Relaxed),"before":before,
        "final_resources":final_resources,"after":after,"samples":samples,"idle_samples":idle_samples,
        "sample_retention":256,"shutdown_millis":shutdown_started.elapsed().as_secs_f64()*1000.,
        "shutdown":format!("{shutdown:?}"),"passed":errors.is_empty(),"errors":errors,
        "latency_semantics":"All attempts and successes use cumulative microsecond power-of-two upper buckets; successful latency includes allocation, admission, execution and reply. Timer lag and RSS/threads are sampled; live_payload_bytes counts harness-owned Vec bytes only."});
    println!("{}", serde_json::to_string_pretty(&output)?);
    if !errors.is_empty() {
        return Err("mixed-load acceptance failed".into());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!(
            "mixed-load [--profile FILE] [--field-name VALUE] (all Config fields supported; array values use JSON). See examples/mixed-load/README.md"
        );
        return Ok(());
    }
    let config = Arc::new(Config::parse(&args)?);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.tokio_workers)
        .enable_all()
        .build()?
        .block_on(run(config))
}
