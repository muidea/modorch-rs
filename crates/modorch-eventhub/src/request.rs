use crate::{waitgraph::WaitEdge, *};
use std::{
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

pub(crate) struct RequestRecord {
    pub status: RequestStatus,
    pub timed_out: bool,
    wait_ended: bool,
    pub result: Option<FrameworkResult<HandlerReply>>,
    observation: Option<Arc<AtomicU8>>,
}
impl RequestRecord {
    fn set_status(&mut self, status: RequestStatus) {
        self.status = status;
        if let Some(observation) = &self.observation {
            observation.store(status as u8, Ordering::Release);
        }
    }
}
pub(crate) struct RequestShared {
    pub record: Mutex<RequestRecord>,
    changed: Notify,
    metrics: crate::metrics::MetricSet,
}
impl RequestShared {
    pub fn new(metrics: crate::metrics::MetricSet) -> Arc<Self> {
        Arc::new(Self {
            record: Mutex::new(RequestRecord {
                status: RequestStatus::Queued,
                timed_out: false,
                wait_ended: false,
                result: None,
                observation: None,
            }),
            changed: Notify::new(),
            metrics,
        })
    }
    pub fn begin(&self) -> bool {
        let mut r = self.record.lock().unwrap();
        if r.status != RequestStatus::Queued {
            return false;
        }
        r.set_status(RequestStatus::Running);
        true
    }
    pub fn complete(&self, result: FrameworkResult<HandlerReply>) {
        let mut r = self.record.lock().unwrap();
        let status = if matches!(&result, Err(FrameworkError::CanceledBeforeStart(_))) {
            if r.status == RequestStatus::Queued {
                self.metrics.canceled();
            }
            RequestStatus::CanceledBeforeStart
        } else if r.timed_out {
            RequestStatus::CompletedAfterCallerTimeout
        } else {
            RequestStatus::Completed
        };
        r.set_status(status);
        // A caller that stopped waiting does not need the eventual reply retained.
        if !r.wait_ended {
            r.result = Some(result);
        }
        self.changed.notify_waiters();
    }
    pub(crate) fn cancel_wait(&self, id: OperationId, timeout: bool) -> FrameworkError {
        self.cancel_locked(&mut self.record.lock().unwrap(), id, timeout)
    }
    fn cancel_locked(
        &self,
        r: &mut RequestRecord,
        id: OperationId,
        timeout: bool,
    ) -> FrameworkError {
        if !r.wait_ended {
            if timeout {
                self.metrics.timed_out();
            } else {
                self.metrics.caller_canceled();
            }
            r.wait_ended = true;
        }
        r.result.take();
        r.timed_out |= timeout;
        match r.status {
            RequestStatus::Queued | RequestStatus::CanceledBeforeStart => {
                if r.status == RequestStatus::Queued {
                    self.metrics.canceled();
                }
                r.set_status(RequestStatus::CanceledBeforeStart);
                self.changed.notify_waiters();
                FrameworkError::CanceledBeforeStart(id)
            }
            _ => FrameworkError::Indeterminate(id),
        }
    }
}
/// Single-consumer typed response handle; dropping it cancels queued work only.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct RequestHandle<R> {
    pub(crate) target: ComponentRef,
    pub(crate) id: OperationId,
    pub(crate) shared: Arc<RequestShared>,
    pub(crate) edge: Option<WaitEdge>,
    pub(crate) marker: PhantomData<R>,
}
impl<R: Send + Sync + 'static> RequestHandle<R> {
    /// Return the tracked operation identity.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn operation_id(&self) -> OperationId {
        self.id
    }
    /// Read request progress without acquiring or retaining its response.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn status(&self) -> RequestStatus {
        self.shared.record.lock().unwrap().status
    }
    /// Create a lightweight status observer which never retains the reply payload.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn observation(&self) -> RequestObservation {
        let mut record = self.shared.record.lock().unwrap();
        let status = record.status;
        let state = record
            .observation
            .get_or_insert_with(|| Arc::new(AtomicU8::new(status as u8)))
            .clone();
        RequestObservation { state }
    }
    /// Consume the typed response handle; timeout cancels Queued work or returns Indeterminate once Running. No rollback is implied.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub async fn wait(mut self, timeout: Duration) -> FrameworkResult<Arc<R>> {
        let wait = async {
            loop {
                let notified = self.shared.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut r = self.shared.record.lock().unwrap();
                    if let Some(result) = r.result.take() {
                        return result;
                    }
                    if r.status == RequestStatus::CanceledBeforeStart {
                        return Err(FrameworkError::CanceledBeforeStart(self.id));
                    }
                }
                notified.await;
            }
        };
        let reply = match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result,
            Err(_) => {
                let mut record = self.shared.record.lock().unwrap();
                match record.result.take() {
                    Some(result) => result,
                    None => Err(self.shared.cancel_locked(&mut record, self.id, true)),
                }
            }
        };
        self.edge.take();
        reply?.downcast::<R>().map_err(|_| {
            FrameworkError::TypeMismatch("request reply".into()).in_execution(
                self.target.clone(),
                Some(self.id),
                Phase::Handle,
            )
        })
    }
    /// Abandon the response wait; queued work becomes CanceledBeforeStart, running work is Indeterminate.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn cancel(self) -> FrameworkError {
        self.shared.cancel_wait(self.id, false)
    }
}
impl<R> Drop for RequestHandle<R> {
    fn drop(&mut self) {
        let mut record = self.shared.record.lock().unwrap();
        if matches!(
            record.status,
            RequestStatus::Queued | RequestStatus::Running
        ) {
            self.shared.cancel_locked(&mut record, self.id, false);
        } else {
            record.wait_ended = true;
            record.result.take();
        }
    }
}
#[derive(Clone)]
/// Lightweight status observer that does not retain response payloads.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct RequestObservation {
    state: Arc<AtomicU8>,
}
impl RequestObservation {
    /// Read request progress without acquiring or retaining its response.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn status(&self) -> RequestStatus {
        match self.state.load(Ordering::Acquire) {
            0 => RequestStatus::Queued,
            1 => RequestStatus::Running,
            2 => RequestStatus::Completed,
            3 => RequestStatus::CanceledBeforeStart,
            _ => RequestStatus::CompletedAfterCallerTimeout,
        }
    }
}

pub(crate) struct RequestGuard {
    pub shared: Arc<RequestShared>,
    pub id: OperationId,
}
impl std::ops::Deref for RequestGuard {
    type Target = RequestShared;
    fn deref(&self) -> &RequestShared {
        &self.shared
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        let status = self.shared.record.lock().unwrap().status;
        match status {
            RequestStatus::Running => self
                .shared
                .complete(Err(FrameworkError::Indeterminate(self.id))),
            RequestStatus::Queued => self
                .shared
                .complete(Err(FrameworkError::CanceledBeforeStart(self.id))),
            _ => {}
        }
    }
}
