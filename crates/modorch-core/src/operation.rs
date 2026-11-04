use crate::*;
#[cfg(test)]
use loom::sync::Mutex;
#[cfg(not(test))]
use std::sync::Mutex;
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    sync::{Arc, Weak},
};
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq)]
enum Admission {
    Startup,
    Open,
    Closed,
    Terminated,
}
struct State {
    admission: Admission,
    next: u64,
    blocked: BTreeSet<ComponentRef>,
    generations: HashMap<ComponentRef, u64>,
    privileged: BTreeSet<OperationId>,
    active: HashMap<OperationId, ComponentRef>,
    owners: HashMap<ComponentRef, usize>,
}
static NEXT_APPLICATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
struct Inner {
    application_id: ApplicationId,
    state: Mutex<State>,
    changed: Notify,
    admission_events: Notify,
}
#[derive(Clone)]
/// Application-local admission gate, causal scope and operation accounting.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct OperationTracker(Arc<Inner>);
struct LeaseInner {
    tracker: Weak<Inner>,
    id: OperationId,
}
/// Clones share a single counted operation; the last clone releases it.
#[derive(Clone)]
pub struct OperationLease(Arc<LeaseInner>);
tokio::task_local! { static CURRENT: Option<OperationLease>; }
impl Default for OperationTracker {
    fn default() -> Self {
        Self::new()
    }
}
impl OperationTracker {
    /// Create a new isolated tracker with root admission initially not ready.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            application_id: NEXT_APPLICATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            state: Mutex::new(State {
                admission: Admission::Startup,
                next: 1,
                blocked: BTreeSet::new(),
                generations: HashMap::new(),
                privileged: BTreeSet::new(),
                active: HashMap::new(),
                owners: HashMap::new(),
            }),
            changed: Notify::new(),
            admission_events: Notify::new(),
        }))
    }
    /// Return this tracker's isolated application identity.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn application_id(&self) -> ApplicationId {
        self.0.application_id
    }
    /// Open root admission; return Closed if the tracker has irreversibly terminated.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn open(&self) -> FrameworkResult<()> {
        let mut state = self.0.state.lock().unwrap();
        if state.admission != Admission::Startup {
            return Err(FrameworkError::Closed);
        }
        state.admission = Admission::Open;
        self.0.admission_events.notify_waiters();
        Ok(())
    }
    /// Close root admission while allowing admitted causal children to finish.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn close(&self) {
        let mut state = self.0.state.lock().unwrap();
        if state.admission != Admission::Terminated {
            state.admission = Admission::Closed;
        }
        self.0.admission_events.notify_waiters();
        self.0.changed.notify_waiters();
    }
    /// Reserve the lifecycle coordinator's final causal chain and close roots in
    /// one linearized step. Only the runtime coordinator uses this capability.
    #[doc(hidden)]
    pub fn close_with_lease(&self, owner: ComponentRef) -> Option<OperationLease> {
        let mut state = self.0.state.lock().unwrap();
        if matches!(state.admission, Admission::Closed | Admission::Terminated) {
            return None;
        }
        state.admission = Admission::Closed;
        self.0.admission_events.notify_waiters();
        let id = state.next;
        state.privileged.insert(id);
        state.next += 1;
        if let Some(count) = state.owners.get_mut(&owner) {
            *count += 1;
        } else {
            state.owners.insert(owner.clone(), 1);
        }
        state.active.insert(id, owner);
        Some(OperationLease(Arc::new(LeaseInner {
            tracker: Arc::downgrade(&self.0),
            id,
        })))
    }
    /// Irreversibly reject further tracker admission.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn terminate(&self) {
        self.0.state.lock().unwrap().admission = Admission::Terminated;
        self.0.admission_events.notify_waiters();
        self.0.changed.notify_waiters();
    }
    /// Report whether root admission is currently closed.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn is_closed(&self) -> bool {
        matches!(
            self.0.state.lock().unwrap().admission,
            Admission::Closed | Admission::Terminated
        )
    }
    /// Read the current tracked causal lease, if any.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn current(&self) -> Option<OperationLease> {
        CURRENT
            .try_with(Clone::clone)
            .ok()
            .flatten()
            .filter(|lease| Weak::ptr_eq(&lease.0.tracker, &Arc::downgrade(&self.0)))
    }
    /// Admit an operation in the trusted current causal scope; NotReady or Closed rejects it.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn admit(&self, owner: ComponentRef) -> FrameworkResult<OperationLease> {
        self.admit_inner(owner, false, None)
    }
    /// Framework startup tasks/services may run before external ingress opens.
    pub fn admit_background(&self, owner: ComponentRef) -> FrameworkResult<OperationLease> {
        self.admit_inner(owner, true, None)
    }
    fn admit_inner(
        &self,
        owner: ComponentRef,
        startup: bool,
        generation: Option<u64>,
    ) -> FrameworkResult<OperationLease> {
        let parent = self.current();
        let mut state = self.0.state.lock().unwrap();
        Self::check_locked(&state, &owner, startup, generation, parent.as_ref())?;
        let id = state.next;
        state.next += 1;
        if let Some(count) = state.owners.get_mut(&owner) {
            *count += 1;
        } else {
            state.owners.insert(owner.clone(), 1);
        }
        state.active.insert(id, owner);
        Ok(OperationLease(Arc::new(LeaseInner {
            tracker: Arc::downgrade(&self.0),
            id,
        })))
    }
    fn check_locked(
        state: &State,
        owner: &ComponentRef,
        startup: bool,
        generation: Option<u64>,
        parent: Option<&OperationLease>,
    ) -> FrameworkResult<()> {
        if generation
            .is_some_and(|expected| *state.generations.get(owner).unwrap_or(&0) != expected)
        {
            return Err(FrameworkError::Closed);
        }
        let child = parent
            .as_ref()
            .is_some_and(|p| state.active.contains_key(&p.id()));
        match state.admission {
            Admission::Terminated => return Err(FrameworkError::Closed),
            Admission::Closed if !child => return Err(FrameworkError::Closed),
            Admission::Startup if !startup => return Err(FrameworkError::NotReady),
            _ => {}
        }
        if state.blocked.contains(owner)
            && !parent.as_ref().is_some_and(|p| {
                state.privileged.contains(&p.id())
                    || state
                        .active
                        .get(&p.id())
                        .is_some_and(|c| state.blocked.contains(c))
            })
        {
            return Err(FrameworkError::Closed);
        }
        Ok(())
    }
    /// Non-reserving check used by bounded resource waiters. Actual admission
    /// repeats this check atomically with lease creation.
    pub fn check_bound(&self, owner: &ComponentRef, generation: u64) -> FrameworkResult<()> {
        let parent = self.current();
        Self::check_locked(
            &self.0.state.lock().unwrap(),
            owner,
            true,
            Some(generation),
            parent.as_ref(),
        )
    }
    /// Register before checking the gate to avoid losing a close notification.
    #[doc(hidden)]
    pub fn admission_notification(&self) -> tokio::sync::futures::Notified<'_> {
        self.0.admission_events.notified()
    }
    /// Poll a future in the supplied tracked operation scope; dropping the future releases its scope.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub async fn scope<F: Future>(&self, lease: OperationLease, future: F) -> F::Output {
        {
            use tracing::Instrument;
            let span = tracing::info_span!(
                "operation",
                application_id = self.application_id(),
                operation_id = lease.id()
            );
            CURRENT.scope(Some(lease), future.instrument(span)).await
        }
    }
    /// Bind owned synchronous work to its causal operation on the current thread.
    pub fn scope_sync<T>(&self, lease: OperationLease, action: impl FnOnce() -> T) -> T {
        CURRENT.sync_scope(Some(lease), action)
    }
    /// Establish a fresh external root even inside an owner task.
    pub fn as_root<T>(&self, action: impl FnOnce() -> T) -> T {
        CURRENT.sync_scope(None, action)
    }
    /// Admit only if the owner generation is current and its admission gate permits this causal operation.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn admit_bound(
        &self,
        owner: ComponentRef,
        generation: u64,
    ) -> FrameworkResult<OperationLease> {
        self.admit_inner(owner, true, Some(generation))
    }
    /// Return the current component generation.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn generation(&self, owner: &ComponentRef) -> u64 {
        *self
            .0
            .state
            .lock()
            .unwrap()
            .generations
            .get(owner)
            .unwrap_or(&0)
    }
    /// Check that a capability still names the current component generation.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn generation_valid(&self, owner: &ComponentRef, generation: u64) -> bool {
        self.generation(owner) == generation
    }
    /// Close a dependency closure to outside producers and reserve its cleanup chain.
    pub fn close_owners(&self, owners: &BTreeSet<ComponentRef>) -> OperationLease {
        let mut state = self.0.state.lock().unwrap();
        state.blocked.extend(owners.iter().cloned());
        let id = state.next;
        state.next += 1;
        let owner = owners.first().expect("nonempty closure").clone();
        if let Some(count) = state.owners.get_mut(&owner) {
            *count += 1;
        } else {
            state.owners.insert(owner.clone(), 1);
        }
        state.active.insert(id, owner);
        self.0.admission_events.notify_waiters();
        self.0.changed.notify_waiters();
        OperationLease(Arc::new(LeaseInner {
            tracker: Arc::downgrade(&self.0),
            id,
        }))
    }
    /// Advance only after all old-generation work has drained. Gates stay closed
    /// until the replacement instances pass readiness.
    pub fn advance_owners(&self, owners: &BTreeSet<ComponentRef>) -> FrameworkResult<()> {
        let mut state = self.0.state.lock().unwrap();
        if owners.iter().any(|c| state.owners.contains_key(c)) {
            return Err(FrameworkError::InvalidConfig(
                "old generation has pending operations".into(),
            ));
        }
        for owner in owners {
            *state.generations.entry(owner.clone()).or_default() += 1;
        }
        self.0.admission_events.notify_waiters();
        Ok(())
    }
    /// Reopen component admission after recovery; terminated applications return Closed.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn open_owners(&self, owners: &BTreeSet<ComponentRef>) -> FrameworkResult<()> {
        let mut state = self.0.state.lock().unwrap();
        if state.admission != Admission::Open {
            return Err(FrameworkError::Closed);
        }
        for owner in owners {
            state.blocked.remove(owner);
        }
        self.0.admission_events.notify_waiters();
        self.0.changed.notify_waiters();
        Ok(())
    }
    /// Snapshot tracked operations associated with the specified owners.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn pending_owners(&self, owners: &BTreeSet<ComponentRef>) -> PendingWork {
        let state = self.0.state.lock().unwrap();
        let owners: BTreeMap<_, _> = owners
            .iter()
            .filter_map(|owner| state.owners.get(owner).map(|count| (owner.clone(), *count)))
            .collect();
        PendingWork {
            operations: owners.values().sum(),
            owners,
        }
    }
    /// Wait without an implicit timeout until the specified owners have no tracked operations.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub async fn drain_owners(&self, owners: &BTreeSet<ComponentRef>) {
        loop {
            let notified = self.0.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let drained = {
                let state = self.0.state.lock().unwrap();
                !owners.iter().any(|owner| state.owners.contains_key(owner))
            };
            if drained {
                return;
            }
            notified.await;
        }
    }
    /// Wait for an admission-state notification; callers must recheck their condition.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub async fn admission_changed(&self) {
        self.0.changed.notified().await;
    }
    /// Poll a future with inherited causal identity cleared, establishing an external root boundary.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub async fn scope_root<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(None, future).await
    }
    /// Snapshot pending operations and owner distribution.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn pending(&self) -> PendingWork {
        let state = self.0.state.lock().unwrap();
        PendingWork {
            operations: state.active.len(),
            owners: state
                .owners
                .iter()
                .map(|(owner, count)| (owner.clone(), *count))
                .collect(),
        }
    }
    /// After close, zero is a fixed point: no live parent remains to admit children.
    pub async fn drain(&self) {
        loop {
            let notified = self.0.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.state.lock().unwrap().active.is_empty() {
                return;
            }
            notified.await;
        }
    }
}
impl OperationLease {
    /// Return the application or operation identity represented by this handle.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn id(&self) -> OperationId {
        self.0.id
    }
}
impl Drop for LeaseInner {
    fn drop(&mut self) {
        if let Some(tracker) = self.tracker.upgrade() {
            {
                let mut state = tracker.state.lock().unwrap();
                if let Some(owner) = state.active.remove(&self.id) {
                    let count = state.owners.get_mut(&owner).expect("counted owner");
                    *count -= 1;
                    if *count == 0 {
                        state.owners.remove(&owner);
                    }
                }
                state.privileged.remove(&self.id);
            }
            tracker.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod model {
    use super::*;
    #[test]
    fn close_and_admit_are_linearized_and_never_lose_leases() {
        loom::model(|| {
            let tracker = OperationTracker::new();
            tracker.open().unwrap();
            let producer = tracker.clone();
            let closer = tracker.clone();
            let submit =
                loom::thread::spawn(move || producer.admit(ComponentRef::module("worker")));
            let close = loom::thread::spawn(move || closer.close());
            let lease = submit.join().unwrap();
            close.join().unwrap();
            match lease {
                Ok(lease) => {
                    assert_eq!(tracker.pending().operations, 1);
                    drop(lease);
                }
                Err(FrameworkError::Closed) => assert_eq!(tracker.pending().operations, 0),
                Err(error) => panic!("{error}"),
            }
            assert_eq!(tracker.pending().operations, 0);
            assert!(tracker.admit(ComponentRef::module("late")).is_err());
        });
    }
    #[test]
    fn owner_generation_cannot_advance_while_an_old_task_is_admitted() {
        loom::model(|| {
            let tracker = OperationTracker::new();
            tracker.open().unwrap();
            let owner = ComponentRef::module("worker");
            let owners = BTreeSet::from([owner.clone()]);
            let worker = tracker.clone();
            let worker_owner = owner.clone();
            let submit = loom::thread::spawn(move || worker.admit_bound(worker_owner, 0));
            let cleanup = tracker.close_owners(&owners);
            drop(cleanup);
            let lease = submit.join().unwrap();
            if let Ok(lease) = lease {
                assert!(tracker.advance_owners(&owners).is_err());
                drop(lease);
            }
            tracker.advance_owners(&owners).unwrap();
            tracker.open_owners(&owners).unwrap();
            assert!(tracker.admit_bound(owner.clone(), 0).is_err());
            drop(tracker.admit_bound(owner, 1).unwrap());
            assert_eq!(tracker.pending().operations, 0);
        });
    }
}
