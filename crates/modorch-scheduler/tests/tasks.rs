use modorch_core::*;
use modorch_scheduler::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
fn scheduler(
    tasks: usize,
    timers: usize,
) -> (
    TaskScheduler,
    SchedulerRuntime,
    OperationTracker,
    FailureReporter,
) {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let failures = FailureReporter::new();
    let runtime = SchedulerRuntime::new(tracker.clone(), failures.clone(), tasks, timers).unwrap();
    (
        runtime.for_owner(ComponentRef::module("worker")),
        runtime,
        tracker,
        failures,
    )
}
#[tokio::test]
async fn tasks_have_results_bounds_and_cooperative_shutdown() {
    let (scheduler, runtime, tracker, _) = scheduler(1, 1);
    let handle = scheduler
        .spawn(|cancel| async move {
            cancel.cancelled().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        scheduler.spawn(|_| async { Ok(()) }),
        Err(FrameworkError::Backpressure(_))
    ));
    tracker.close();
    runtime.begin_shutdown();
    assert_eq!(tracker.pending().operations, 1);
    handle.wait().await.unwrap();
    tracker.drain().await;
    assert_eq!(handle.state(), TaskState::Completed);
    assert!(matches!(
        scheduler.spawn(|_| async { Ok(()) }),
        Err(FrameworkError::Closed)
    ));
}
#[tokio::test]
async fn task_panic_is_failure_and_never_success() {
    let (scheduler, _, tracker, failures) = scheduler(4, 1);
    let handle = scheduler
        .spawn(|_| async {
            panic!("injected fault");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        handle.wait().await,
        Err(FrameworkError::ExecutionFailed { .. })
    ));
    assert_eq!(handle.state(), TaskState::Failed);
    assert!(failures.current().is_some());
    tracker.close();
    tracker.drain().await;
}
#[tokio::test(start_paused = true)]
async fn timers_do_not_consume_task_slots_and_skip_overlapping_ticks() {
    let (scheduler, runtime, tracker, _) = scheduler(1, 1);
    let count = Arc::new(AtomicUsize::new(0));
    let task_count = count.clone();
    let timer = scheduler
        .interval(
            IntervalSpec {
                period: Duration::from_secs(1),
                policy: TickPolicy::SkipIfRunning,
            },
            Arc::new(move |cancel| {
                let count = task_count.clone();
                Box::pin(async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    cancel.cancelled().await;
                    Ok(())
                })
            }),
        )
        .unwrap();
    assert!(matches!(
        scheduler.delay(
            Duration::from_secs(1),
            Arc::new(|_| Box::pin(async { Ok(()) }))
        ),
        Err(FrameworkError::Backpressure(_))
    ));
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    tracker.close();
    runtime.begin_shutdown();
    timer.wait().await;
    tracker.drain().await;
    assert_eq!(tracker.pending().operations, 0);
}

#[tokio::test]
async fn forced_task_abort_has_a_cancelled_receipt_and_releases_its_owner() {
    let (scheduler, runtime, tracker, _) = scheduler(1, 1);
    let handle = scheduler
        .spawn(|_| async {
            std::future::pending::<()>().await;
            Ok(())
        })
        .unwrap();
    tokio::task::yield_now().await;
    runtime.force_cancel();
    assert!(matches!(
        handle.wait().await,
        Err(FrameworkError::Cancelled)
    ));
    assert_eq!(handle.state(), TaskState::Cancelled);
    tracker.close();
    tracker.drain().await;
    assert!(runtime.pending_owners().is_empty());
}
#[tokio::test(start_paused = true)]
async fn bounded_timer_queue_preserves_only_its_configured_backlog() {
    let (scheduler, runtime, tracker, _) = scheduler(8, 1);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let task_gate = gate.clone();
    let task_count = count.clone();
    let timer = scheduler
        .interval(
            IntervalSpec {
                period: Duration::from_secs(1),
                policy: TickPolicy::QueueBounded { capacity: 2 },
            },
            Arc::new(move |_| {
                let gate = task_gate.clone();
                let count = task_count.clone();
                Box::pin(async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    gate.acquire().await.unwrap().forget();
                    Ok(())
                })
            }),
        )
        .unwrap();
    tokio::task::yield_now().await;
    for _ in 0..5 {
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    gate.add_permits(3);
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(count.load(Ordering::SeqCst), 3);
    timer.cancel();
    timer.wait().await;
    tracker.close();
    runtime.begin_shutdown();
    tracker.drain().await;
}
#[tokio::test(start_paused = true)]
async fn timer_overlap_has_an_explicit_limit() {
    let (scheduler, runtime, tracker, _) = scheduler(8, 1);
    let count = Arc::new(AtomicUsize::new(0));
    let task_count = count.clone();
    let timer = scheduler
        .interval(
            IntervalSpec {
                period: Duration::from_secs(1),
                policy: TickPolicy::AllowOverlap { max_in_flight: 2 },
            },
            Arc::new(move |cancel| {
                let count = task_count.clone();
                Box::pin(async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    cancel.cancelled().await;
                    Ok(())
                })
            }),
        )
        .unwrap();
    tokio::task::yield_now().await;
    for _ in 0..4 {
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    tracker.close();
    runtime.begin_shutdown();
    timer.wait().await;
    tracker.drain().await;
}

#[tokio::test]
async fn task_errors_retain_operation_owner_phase_and_retryability() {
    let (scheduler, _, tracker, failures) = scheduler(1, 1);
    let task = scheduler
        .spawn(|_| async {
            Err(FrameworkError::Backpressure(ComponentRef::module(
                "dependency",
            )))
        })
        .unwrap();
    let error = task.wait().await.unwrap_err();
    assert_eq!(error.operation_id(), Some(task.operation_id()));
    assert_eq!(error.component(), Some(&ComponentRef::module("worker")));
    assert_eq!(error.phase(), Some(Phase::Task));
    assert!(error.retryable());
    assert!(matches!(
        error.root_cause(),
        FrameworkError::Backpressure(_)
    ));
    assert_eq!(
        failures.current().unwrap().operation_id(),
        Some(task.operation_id())
    );
    tracker.close();
    tracker.drain().await;
}

fn resources() -> ExecutionResources {
    ExecutionResources {
        cpu_threads: 1,
        cpu_capacity: 1,
        blocking_io_threads: 1,
        blocking_io_capacity: 1,
        max_tasks_per_owner: 3,
        ..ExecutionResources::default()
    }
}
#[tokio::test(flavor = "current_thread")]
async fn execution_classes_are_independent_and_force_waits_for_real_completion() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        2,
        resources(),
    )
    .unwrap();
    let scheduler = runtime.for_owner(ComponentRef::module("cpu"));
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let handle = scheduler
        .spawn_cpu(move |_| {
            let _ = started_tx.send(());
            released.recv().unwrap();
            Ok(7usize)
        })
        .unwrap();
    started.await.unwrap();
    assert!(matches!(
        scheduler.spawn_cpu(|_| Ok(())),
        Err(FrameworkError::Backpressure(_))
    ));
    // Both async timers and the separate synchronous I/O resource keep progressing
    // even though the only CPU worker is occupied.
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::time::sleep(Duration::from_millis(1)),
    )
    .await
    .unwrap();
    let io_owner = ComponentRef::module("io");
    let io = runtime
        .for_owner(io_owner.clone())
        .spawn_blocking_io(|_| Ok(9usize))
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), io.wait())
            .await
            .unwrap()
            .unwrap(),
        9
    );
    tracker
        .drain_owners(&std::collections::BTreeSet::from([io_owner]))
        .await;
    tracker.close();
    runtime.force_cancel();
    assert_eq!(handle.state(), TaskState::CancelRequested);
    assert_eq!(tracker.pending().operations, 1);
    assert_eq!(runtime.metrics().cpu_tasks, 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), tracker.drain())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    assert_eq!(handle.wait().await.unwrap(), 7);
    tracker.drain().await;
    assert_eq!(runtime.metrics().cpu_tasks, 0);
}

#[tokio::test]
async fn owner_quota_is_shared_between_handles_and_all_execution_classes() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        1,
        ExecutionResources {
            max_tasks_per_owner: 1,
            ..resources()
        },
    )
    .unwrap();
    let owner = ComponentRef::module("one");
    let task = runtime
        .for_owner(owner.clone())
        .spawn(|cancel| async move {
            cancel.cancelled().await;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        runtime.for_owner(owner).spawn_cpu(|_| Ok(())),
        Err(FrameworkError::Backpressure(_))
    ));
    assert_eq!(
        runtime
            .for_owner(ComponentRef::module("two"))
            .spawn_cpu(|_| Ok(2usize))
            .unwrap()
            .wait()
            .await
            .unwrap(),
        2
    );
    task.cancel();
    task.wait().await.unwrap();
    tracker.close();
    tracker.drain().await;
}

#[tokio::test]
async fn blocking_panic_retains_identity_and_releases_capacity() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let failures = FailureReporter::new();
    let runtime =
        SchedulerRuntime::with_resources(tracker.clone(), failures.clone(), 4, 1, resources())
            .unwrap();
    let owner = ComponentRef::module("panic");
    let scheduler = runtime.for_owner(owner.clone());
    let handle = scheduler
        .spawn_cpu(|_| -> FrameworkResult<()> { panic!("injected CPU fault") })
        .unwrap();
    let id = handle.operation_id();
    assert!(
        matches!(handle.wait().await, Err(FrameworkError::ExecutionFailed { component, operation_id: Some(actual), phase: Phase::Task, .. }) if component == owner && actual == id)
    );
    tracker.drain().await;
    assert!(failures.current().is_some());
    assert_eq!(
        scheduler
            .spawn_cpu(|_| Ok(3usize))
            .unwrap()
            .wait()
            .await
            .unwrap(),
        3
    );
    tracker.close();
    tracker.drain().await;
}

struct HeldExecutor(std::sync::Mutex<Vec<BlockingJob>>);
impl BlockingExecutor for HeldExecutor {
    fn execute(&self, job: BlockingJob) -> FrameworkResult<()> {
        self.0.lock().unwrap().push(job);
        Ok(())
    }
}
#[tokio::test]
async fn queued_blocking_work_can_be_cancelled_and_dropped_waiters_do_not_uncount_it() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let executor = Arc::new(HeldExecutor(std::sync::Mutex::new(vec![])));
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        4,
        1,
        ExecutionResources {
            cpu_executor: Some(executor.clone()),
            ..resources()
        },
    )
    .unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let count = called.clone();
    let handle = runtime
        .for_owner(ComponentRef::module("queued"))
        .spawn_cpu(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let observation = handle.task().clone();
    let wait = tokio::spawn(handle.wait());
    wait.abort();
    let _ = wait.await;
    tracker.close();
    runtime.force_cancel();
    assert_eq!(tracker.pending().operations, 1);
    let job = executor.0.lock().unwrap().pop().unwrap();
    job();
    assert!(matches!(
        observation.wait().await,
        Err(FrameworkError::Cancelled)
    ));
    assert_eq!(called.load(Ordering::SeqCst), 0);
    tracker.drain().await;
}

#[tokio::test]
async fn timer_controls_are_removed_before_exit_acknowledgement() {
    let (scheduler, runtime, tracker, _) = scheduler(4, 64);
    let mut timers = vec![];
    for _ in 0..64 {
        timers.push(
            scheduler
                .delay(
                    Duration::from_secs(60),
                    Arc::new(|_| Box::pin(async { Ok(()) })),
                )
                .unwrap(),
        );
    }
    for timer in &timers {
        timer.cancel();
    }
    for timer in &timers {
        timer.wait().await;
    }
    assert_eq!(runtime.metrics().timers, 0);
    tracker.close();
    tracker.drain().await;
}

#[tokio::test]
async fn finished_ack_returns_all_permits_before_a_capacity_one_resubmission() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        1,
        1,
        ExecutionResources {
            max_tasks_per_owner: 1,
            ..resources()
        },
    )
    .unwrap();
    let scheduler = runtime.for_owner(ComponentRef::module("serial_jobs"));
    for value in 0..256usize {
        assert_eq!(
            scheduler
                .spawn_cpu(move |_| Ok(value))
                .unwrap()
                .wait_finished()
                .await
                .unwrap(),
            value
        );
        assert_eq!(tracker.pending().operations, 0);
        assert_eq!(runtime.metrics().cpu.available, 1);
    }
    tracker.close();
    tracker.drain().await;
}

#[tokio::test]
async fn async_abort_before_first_poll_drops_captures_before_finished_ack() {
    let (scheduler, runtime, tracker, _) = scheduler(1, 1);
    let drops = Arc::new(AtomicUsize::new(0));
    let captured = DroppedCapture(drops.clone());
    let task = scheduler
        .spawn(move |_| async move {
            drop(captured);
            panic!("aborted task ran");
        })
        .unwrap();
    runtime.force_cancel();
    assert!(matches!(
        task.wait_finished().await,
        Err(FrameworkError::Cancelled)
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.pending().operations, 0);
    assert!(runtime.pending_owners().is_empty());
    scheduler
        .spawn(|_| async { Ok(()) })
        .unwrap()
        .wait_finished()
        .await
        .unwrap();
    tracker.close();
    tracker.drain().await;
}

#[tokio::test(start_paused = true)]
async fn blocking_io_wait_shares_owner_and_global_waiter_bounds_with_cpu() {
    for global_bound in [false, true] {
        let tracker = OperationTracker::new();
        tracker.open().unwrap();
        let runtime = SchedulerRuntime::with_resources(
            tracker.clone(),
            FailureReporter::new(),
            if global_bound { 1 } else { 8 },
            1,
            ExecutionResources {
                max_tasks_per_owner: 1,
                max_waiters: if global_bound { 1 } else { 8 },
                max_waiters_per_owner: 1,
                ..resources()
            },
        )
        .unwrap();
        let owner = ComponentRef::module("shared");
        let scheduler = runtime.for_owner(owner.clone());
        let active = scheduler
            .spawn(|cancel| async move {
                cancel.cancelled().await;
                Ok(())
            })
            .unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let captured = DroppedCapture(drops.clone());
        let wait = scheduler.spawn_blocking_io_wait(
            move |_| {
                drop(captured);
                Ok(9usize)
            },
            Duration::from_secs(60),
        );
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        assert_eq!(tracker.pending().operations, 1);
        let metrics = runtime.metrics();
        assert_eq!(metrics.blocking_io.waiters, 1);
        assert_eq!(metrics.blocking_io.admitted, 0);
        assert_eq!(metrics.owner_resources[&owner].waiters, 1);
        assert_eq!(metrics.owner_resources[&owner].available_tasks, 0);
        assert_eq!(
            metrics.owner_resources[&owner].async_queued
                + metrics.owner_resources[&owner].async_running,
            1
        );
        let contender = runtime.for_owner(if global_bound {
            ComponentRef::module("other")
        } else {
            owner
        });
        assert!(matches!(
            contender
                .spawn_cpu_wait(|_| Ok(()), Duration::from_secs(60))
                .await,
            Err(FrameworkError::ResourceLimit(_))
        ));
        assert_eq!(runtime.metrics().cpu.rejected_waiters, 1);
        if global_bound {
            assert_eq!(metrics.blocking_io.rejected_global, 1);
        } else {
            assert_eq!(metrics.blocking_io.rejected_owner, 1);
        }
        active.cancel();
        active.wait_finished().await.unwrap();
        assert_eq!(wait.await.unwrap().wait_finished().await.unwrap(), 9);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let metrics = runtime.metrics();
        assert_eq!(metrics.blocking_io.waiters, 0);
        assert_eq!(metrics.blocking_io.completed, 1);
        assert_eq!(metrics.available_task_slots, metrics.task_capacity);
        tracker.close();
        tracker.drain().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contending_resource_waiters_make_progress_without_lost_capacity_wakeups() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        1,
        1,
        resources(),
    )
    .unwrap();
    let mut producers = tokio::task::JoinSet::new();
    for index in 0..16 {
        let scheduler = runtime.for_owner(ComponentRef::module(format!("producer_{index}")));
        producers.spawn(async move {
            for _ in 0..32 {
                let task = if index % 2 == 0 {
                    scheduler
                        .spawn_cpu_wait(|_| Ok(7), Duration::from_secs(10))
                        .await
                } else {
                    scheduler
                        .spawn_blocking_io_wait(|_| Ok(7), Duration::from_secs(10))
                        .await
                };
                assert_eq!(task.unwrap().wait_finished().await.unwrap(), 7);
            }
        });
    }
    while let Some(result) = producers.join_next().await {
        result.unwrap();
    }
    let metrics = runtime.metrics();
    assert_eq!(metrics.cpu.completed, 256);
    assert_eq!(metrics.blocking_io.completed, 256);
    assert_eq!(metrics.cpu.waiters + metrics.blocking_io.waiters, 0);
    assert_eq!(metrics.available_task_slots, 1);
    assert!(runtime.pending_owners().is_empty());
    assert_eq!(tracker.pending().operations, 0);
    tracker.close();
    tracker.drain().await;
}
#[tokio::test]
async fn resource_waiters_are_bounded_cancel_safe_and_timeout_without_late_execution() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        1,
        ExecutionResources {
            max_waiters: 1,
            max_waiters_per_owner: 1,
            ..resources()
        },
    )
    .unwrap();
    let scheduler = runtime.for_owner(ComponentRef::module("bounded"));
    let (release, released) = std::sync::mpsc::channel();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let active = scheduler
        .spawn_cpu(move |_| {
            started_tx.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        })
        .unwrap();
    started.await.unwrap();
    let invoked = Arc::new(AtomicUsize::new(0));
    let calls = invoked.clone();
    let producer = scheduler.clone();
    let wait = tokio::spawn(async move {
        producer
            .spawn_cpu_wait(
                move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                Duration::from_secs(10),
            )
            .await
    });
    while runtime.metrics().cpu.waiters != 1 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        scheduler
            .spawn_cpu_wait(|_| Ok(()), Duration::from_secs(1))
            .await,
        Err(FrameworkError::ResourceLimit(_))
    ));
    wait.abort();
    let _ = wait.await;
    assert_eq!(runtime.metrics().cpu.waiters, 0);
    let calls = invoked.clone();
    assert!(matches!(
        scheduler
            .spawn_cpu_wait(
                move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                Duration::from_millis(1)
            )
            .await,
        Err(FrameworkError::AdmissionTimeout)
    ));
    release.send(()).unwrap();
    active.wait_finished().await.unwrap();
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    let metrics = runtime.metrics();
    assert_eq!(metrics.cpu.waiters, 0);
    assert_eq!(metrics.cpu.admission_timeouts, 1);
    assert_eq!(metrics.cpu.rejected_waiters, 1);
    assert!(metrics.cpu.rejected_capacity > 0);
    tracker.close();
    tracker.drain().await;
}
#[tokio::test]
async fn resource_waiting_wakes_after_real_release_and_registers_only_once() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        1,
        resources(),
    )
    .unwrap();
    let scheduler = runtime.for_owner(ComponentRef::module("wait"));
    let (release, released) = std::sync::mpsc::channel();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let active = scheduler
        .spawn_cpu(move |_| {
            started_tx.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        })
        .unwrap();
    started.await.unwrap();
    let producer = scheduler.clone();
    let wait = tokio::spawn(async move {
        producer
            .spawn_cpu_wait(|_| Ok(7usize), Duration::from_secs(10))
            .await
    });
    while runtime.metrics().cpu.waiters != 1 {
        tokio::task::yield_now().await;
    }
    release.send(()).unwrap();
    active.wait_finished().await.unwrap();
    assert_eq!(
        wait.await.unwrap().unwrap().wait_finished().await.unwrap(),
        7
    );
    let metrics = runtime.metrics();
    assert_eq!(metrics.cpu.admitted, 2);
    assert_eq!(metrics.cpu.completed, 2);
    assert_eq!(metrics.cpu.waiters, 0);
    assert!(metrics.cpu.admission_wait_p99_micros > 0);
    tracker.close();
    tracker.drain().await;
}
#[tokio::test(start_paused = true)]
async fn resource_waiting_observes_owner_close_without_polling_or_late_enqueue() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let executor = Arc::new(HeldExecutor(std::sync::Mutex::new(vec![])));
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        1,
        ExecutionResources {
            cpu_executor: Some(executor.clone()),
            ..resources()
        },
    )
    .unwrap();
    let owner = ComponentRef::module("recovering");
    let scheduler = runtime.for_owner(owner.clone());
    let active = scheduler.spawn_cpu(|_| Ok(())).unwrap();
    let wait = scheduler.spawn_cpu_wait(
        |_| -> FrameworkResult<()> { panic!("closed work ran") },
        Duration::from_secs(600),
    );
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    let now = tokio::time::Instant::now();
    let owners = std::collections::BTreeSet::from([owner]);
    drop(tracker.close_owners(&owners));
    assert!(matches!(wait.await, Err(FrameworkError::Closed)));
    assert_eq!(tokio::time::Instant::now(), now);
    active.cancel();
    let job = executor.0.lock().unwrap().pop().unwrap();
    job();
    assert!(matches!(
        active.wait_finished().await,
        Err(FrameworkError::Cancelled)
    ));
    tracker.drain_owners(&owners).await;
    tracker.advance_owners(&owners).unwrap();
    tracker.open_owners(&owners).unwrap();
    assert!(matches!(
        scheduler
            .spawn_cpu_wait(|_| Ok(()), Duration::from_secs(1))
            .await,
        Err(FrameworkError::Closed)
    ));
    tracker.close();
    tracker.drain().await;
}
#[tokio::test]
async fn resource_metrics_distinguish_queued_running_and_canceled_work_by_owner() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let executor = Arc::new(HeldExecutor(std::sync::Mutex::new(vec![])));
    let runtime = SchedulerRuntime::with_resources(
        tracker.clone(),
        FailureReporter::new(),
        8,
        1,
        ExecutionResources {
            cpu_capacity: 2,
            cpu_executor: Some(executor.clone()),
            ..resources()
        },
    )
    .unwrap();
    let owner = ComponentRef::module("observed");
    let scheduler = runtime.for_owner(owner.clone());
    let (release, released) = std::sync::mpsc::channel();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let first = scheduler
        .spawn_cpu(move |_| {
            started_tx.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        })
        .unwrap();
    let second = scheduler
        .spawn_cpu(|_| -> FrameworkResult<()> { panic!("canceled queued work ran") })
        .unwrap();
    assert_eq!(runtime.metrics().cpu.queued, 2);
    let job = executor.0.lock().unwrap().remove(0);
    let running = std::thread::spawn(job);
    started.await.unwrap();
    let metrics = runtime.metrics();
    assert_eq!(metrics.cpu.running, 1);
    assert_eq!(metrics.cpu.queued, 1);
    assert_eq!(metrics.owner_resources[&owner].cpu_running, 1);
    assert_eq!(metrics.owner_resources[&owner].cpu_queued, 1);
    second.cancel();
    let job = executor.0.lock().unwrap().pop().unwrap();
    job();
    assert!(matches!(
        second.wait_finished().await,
        Err(FrameworkError::Cancelled)
    ));
    release.send(()).unwrap();
    first.wait_finished().await.unwrap();
    running.join().unwrap();
    let metrics = runtime.metrics();
    assert_eq!(metrics.cpu.completed, 1);
    assert_eq!(metrics.cpu.canceled_before_start, 1);
    assert!(metrics.cpu.queue_p99_micros > 0);
    assert!(metrics.cpu.execution_p99_micros > 0);
    assert_eq!(metrics.cpu.available, 2);
    tracker.close();
    tracker.drain().await;
}
struct RejectExecutor;
impl BlockingExecutor for RejectExecutor {
    fn execute(&self, _: BlockingJob) -> FrameworkResult<()> {
        Err(FrameworkError::ResourceLimit("host rejected".into()))
    }
}
struct DroppedCapture(Arc<AtomicUsize>);
impl Drop for DroppedCapture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn rejected_or_discarded_jobs_drop_captures_before_finished_ack_and_release_accounting() {
    for rejected in [false, true] {
        let tracker = OperationTracker::new();
        tracker.open().unwrap();
        let executor = Arc::new(HeldExecutor(std::sync::Mutex::new(vec![])));
        let adapter: Arc<dyn BlockingExecutor> = if rejected {
            Arc::new(RejectExecutor)
        } else {
            executor.clone()
        };
        let runtime = SchedulerRuntime::with_resources(
            tracker.clone(),
            FailureReporter::new(),
            1,
            1,
            ExecutionResources {
                cpu_executor: Some(adapter),
                ..resources()
            },
        )
        .unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let captured = DroppedCapture(drops.clone());
        let result = runtime
            .for_owner(ComponentRef::module("drop"))
            .spawn_cpu(move |_| {
                drop(captured);
                Ok(())
            });
        if rejected {
            assert!(matches!(result, Err(FrameworkError::ResourceLimit(_))));
            assert_eq!(runtime.metrics().cpu.executor_failures, 1);
        } else {
            let handle = result.unwrap();
            executor.0.lock().unwrap().clear();
            assert!(matches!(
                handle.wait_finished().await,
                Err(FrameworkError::Cancelled)
            ));
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(tracker.pending().operations, 0);
        assert_eq!(runtime.metrics().available_task_slots, 1);
        assert_eq!(runtime.metrics().cpu.available, 1);
        tracker.close();
        tracker.drain().await;
    }
}
