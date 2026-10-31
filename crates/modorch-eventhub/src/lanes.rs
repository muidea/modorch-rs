use crate::*;
use std::{panic::AssertUnwindSafe, sync::MutexGuard};

pub(crate) struct LanePreparation {
    hub: EventHub,
    pub target: ComponentRef,
    pub key: String,
    pub node: usize,
    pub capacity: usize,
    pub idle: std::time::Duration,
    pub changed: Arc<tokio::sync::Notify>,
    pub runtime: tokio::runtime::Handle,
    factory: Arc<dyn HandlerFactory>,
    lease: OperationLease,
    armed: bool,
}
impl Drop for LanePreparation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut routing = self.hub.0.routing.lock().unwrap();
        if let Some(endpoint) = routing.endpoints.get_mut(&self.target) {
            if Arc::ptr_eq(&endpoint.capacity_changed, &self.changed) {
                if let Some(dynamic) = &mut endpoint.dynamic {
                    if dynamic.creating.get(&self.key) == Some(&self.node) {
                        dynamic.creating.remove(&self.key);
                    }
                }
            }
        }
        self.changed.notify_waiters();
    }
}
impl EventHub {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_target<'a>(
        &'a self,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        key: &Option<String>,
        fixed: Option<&ComponentRef>,
        deadline: Option<tokio::time::Instant>,
    ) -> FrameworkResult<(MutexGuard<'a, Routing>, ComponentRef)> {
        let result = self.prepare_target_inner(kind, topic, destination, key, fixed, deadline);
        if let Err(error) = &result {
            self.record_route::<()>(
                error.component().or(fixed),
                kind,
                topic,
                &Err(error.clone()),
            );
        }
        result
    }
    fn selected_target(
        &self,
        routing: &Routing,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        fixed: Option<&ComponentRef>,
    ) -> FrameworkResult<ComponentRef> {
        if routing.terminated {
            return Err(FrameworkError::Closed);
        }
        match fixed {
            Some(target)
                if routing
                    .subscriptions
                    .matching(topic, destination)
                    .any(|spec| &spec.target == target) =>
            {
                Ok(target.clone())
            }
            Some(_) => Err(FrameworkError::Unroutable(topic.into())),
            None => self.unique_target(routing, kind, topic, destination),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_target_inner<'a>(
        &'a self,
        kind: MessageKind,
        topic: &str,
        destination: &Destination,
        key: &Option<String>,
        fixed: Option<&ComponentRef>,
        deadline: Option<tokio::time::Instant>,
    ) -> FrameworkResult<(MutexGuard<'a, Routing>, ComponentRef)> {
        let mut routing = self.0.routing.lock().unwrap();
        Self::check_admission_deadline(deadline)?;
        let target = self.selected_target(&routing, kind, topic, destination, fixed)?;
        let endpoint = routing
            .endpoints
            .get_mut(&target)
            .ok_or_else(|| FrameworkError::Unroutable(topic.into()))?;
        let Some(dynamic) = &mut endpoint.dynamic else {
            return Ok((routing, target));
        };
        let key = key.as_deref().unwrap_or("");
        if dynamic.lanes.contains_key(key) {
            return Ok((routing, target));
        }
        if dynamic.creating.contains_key(key)
            || dynamic.lanes.len() + dynamic.creating.len() >= dynamic.max_lanes
        {
            return Err(FrameworkError::Backpressure(target));
        }
        let lease = self.0.tracker.admit(target.clone())?;
        let node = dynamic.next;
        dynamic.next = node
            .checked_add(1)
            .ok_or_else(|| FrameworkError::InvalidConfig("lane identity exhausted".into()))?;
        let mut plan = LanePreparation {
            hub: self.clone(),
            target: target.clone(),
            key: key.into(),
            node,
            capacity: dynamic.capacity,
            idle: dynamic.idle_timeout,
            runtime: dynamic.runtime.clone(),
            changed: endpoint.capacity_changed.clone(),
            factory: dynamic.factory.clone(),
            lease,
            armed: true,
        };
        dynamic.creating.insert(plan.key.clone(), node);
        drop(routing);
        // User construction and destruction run outside the application routing lock.
        let handler = std::panic::catch_unwind(AssertUnwindSafe(|| plan.factory.create(node)))
            .map_err(|_| FrameworkError::ExecutionFailed {
            component: target.clone(),
            operation_id: Some(plan.lease.id()),
            phase: Phase::Setup,
            detail: "dynamic handler factory panicked".into(),
            source: None,
        })??;
        let mut routing = self.0.routing.lock().unwrap();
        Self::check_admission_deadline(deadline)?;
        if self.selected_target(&routing, kind, topic, destination, fixed)? != target {
            return Err(FrameworkError::Unroutable(topic.into()));
        }
        self.0
            .tracker
            .check_bound(&target, self.0.tracker.generation(&target))?;
        let endpoint = routing
            .endpoints
            .get_mut(&target)
            .ok_or_else(|| FrameworkError::Unroutable(topic.into()))?;
        if !Arc::ptr_eq(&endpoint.capacity_changed, &plan.changed) {
            return Err(FrameworkError::Closed);
        }
        let dynamic = endpoint.dynamic.as_mut().expect("same dynamic endpoint");
        if dynamic.creating.get(&plan.key) != Some(&node) {
            return Err(FrameworkError::Closed);
        }
        dynamic.creating.remove(&plan.key);
        plan.armed = false;
        self.start_lane(&mut routing, &plan, handler);
        plan.changed.notify_waiters();
        Ok((routing, target))
    }
}
