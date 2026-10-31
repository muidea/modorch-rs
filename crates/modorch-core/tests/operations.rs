use modorch_core::*;
use std::time::Duration;

#[tokio::test]
async fn shutdown_allows_children_but_zero_is_a_fixed_point() {
    let tracker = OperationTracker::new();
    assert!(matches!(
        tracker.admit(ComponentRef::module("root")),
        Err(FrameworkError::NotReady)
    ));
    tracker.open().unwrap();
    let root = tracker.admit(ComponentRef::module("root")).unwrap();
    tracker.close();
    assert!(matches!(
        tracker.admit(ComponentRef::module("root")),
        Err(FrameworkError::Closed)
    ));
    tracker
        .scope(root, async {
            let child = tracker.admit(ComponentRef::module("child")).unwrap();
            assert_eq!(tracker.pending().operations, 2);
            drop(child);
            assert!(
                tokio::time::timeout(Duration::from_millis(1), tracker.drain())
                    .await
                    .is_err()
            );
        })
        .await;
    tracker.drain().await;
    assert!(matches!(
        tracker.admit(ComponentRef::module("late")),
        Err(FrameworkError::Closed)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_races_with_admission_without_lost_operations() {
    for _ in 0..50 {
        let tracker = OperationTracker::new();
        tracker.open().unwrap();
        let mut tasks = vec![];
        for _ in 0..8 {
            let tracker = tracker.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..20 {
                    if let Ok(lease) = tracker.admit(ComponentRef::module("worker")) {
                        tokio::task::yield_now().await;
                        drop(lease);
                    }
                }
            }));
        }
        tracker.close();
        for task in tasks {
            task.await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(1), tracker.drain())
            .await
            .unwrap();
        assert_eq!(tracker.pending().operations, 0);
    }
}
