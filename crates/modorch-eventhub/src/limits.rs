use crate::*;

/// Framework admission bounds. Byte costs are declared by trusted message contracts;
/// they are not a deep-size measurement of `Any` or application-owned memory.
#[derive(Clone, Debug)]
pub struct HubLimits {
    pub max_waiters: usize,
    pub max_waiters_per_target: usize,
    pub max_waiting_bytes: Option<usize>,
    pub max_in_flight_bytes: Option<usize>,
    pub max_in_flight_bytes_per_target: Option<usize>,
    pub max_message_bytes: Option<usize>,
    pub max_topic_bytes: usize,
    pub max_lane_key_bytes: usize,
    pub max_headers: usize,
    pub max_header_bytes: usize,
    pub max_subscriptions: usize,
}
impl Default for HubLimits {
    fn default() -> Self {
        Self {
            max_waiters: 1024,
            max_waiters_per_target: 256,
            max_waiting_bytes: None,
            max_in_flight_bytes: None,
            max_in_flight_bytes_per_target: None,
            max_message_bytes: None,
            max_topic_bytes: 1024,
            max_lane_key_bytes: 1024,
            max_headers: 32,
            max_header_bytes: 8192,
            max_subscriptions: 65536,
        }
    }
}
impl HubLimits {
    pub fn validate(&self) -> FrameworkResult<()> {
        if [
            self.max_waiters,
            self.max_waiters_per_target,
            self.max_topic_bytes,
            self.max_lane_key_bytes,
            self.max_headers,
            self.max_header_bytes,
            self.max_subscriptions,
        ]
        .contains(&0)
            || [
                self.max_waiting_bytes,
                self.max_in_flight_bytes,
                self.max_in_flight_bytes_per_target,
                self.max_message_bytes,
            ]
            .contains(&Some(0))
        {
            return Err(FrameworkError::InvalidConfig(
                "zero message resource limit".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn message(
        &self,
        topic: &str,
        destination: &Destination,
        options: &MessageOptions,
    ) -> FrameworkResult<usize> {
        let header_bytes = options.headers.iter().try_fold(0usize, |n, (k, v)| {
            n.checked_add(k.len())?.checked_add(v.len())
        });
        if topic.len() > self.max_topic_bytes
            || matches!(destination, Destination::Named(name) if name.len() > self.max_topic_bytes)
            || options
                .lane_key
                .as_ref()
                .is_some_and(|k| k.len() > self.max_lane_key_bytes)
            || options.headers.len() > self.max_headers
            || header_bytes.is_none_or(|n| n > self.max_header_bytes)
        {
            return Err(FrameworkError::InvalidConfig(
                "message metadata limit exceeded".into(),
            ));
        }
        let bounded = self.max_waiting_bytes.is_some()
            || self.max_in_flight_bytes.is_some()
            || self.max_in_flight_bytes_per_target.is_some()
            || self.max_message_bytes.is_some();
        let bytes = match options.payload_bytes {
            None if bounded => {
                return Err(FrameworkError::InvalidConfig(
                    "payload_bytes required by message budget".into(),
                ));
            }
            value => value.unwrap_or(0),
        };
        if [
            self.max_message_bytes,
            self.max_in_flight_bytes,
            self.max_in_flight_bytes_per_target,
        ]
        .into_iter()
        .flatten()
        .any(|limit| bytes > limit)
        {
            return Err(FrameworkError::InvalidConfig(
                "message exceeds byte budget".into(),
            ));
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AdmissionMetrics {
    pub waiters: usize,
    pub waiting_bytes: usize,
    pub in_flight_bytes: usize,
}
#[derive(Default)]
pub(crate) struct BudgetState {
    pub metrics: AdmissionMetrics,
    waiters: BTreeMap<ComponentRef, usize>,
    bytes: BTreeMap<ComponentRef, usize>,
}
pub(crate) struct Budget {
    pub limits: HubLimits,
    pub state: Mutex<BudgetState>,
    pub changed: Arc<tokio::sync::Notify>,
}
pub(crate) struct BudgetGuard {
    budget: Arc<Budget>,
    owner: Option<ComponentRef>,
    bytes: usize,
    waiting: bool,
}
impl Budget {
    pub fn acquire(
        self: &Arc<Self>,
        owner: &ComponentRef,
        bytes: usize,
        waiting: bool,
    ) -> FrameworkResult<BudgetGuard> {
        if !waiting && bytes == 0 {
            return Ok(BudgetGuard {
                budget: self.clone(),
                owner: None,
                bytes,
                waiting,
            });
        }
        let mut state = self.state.lock().unwrap();
        let limits = &self.limits;
        if waiting {
            let next = state.metrics.waiting_bytes.checked_add(bytes);
            if state.metrics.waiters >= limits.max_waiters
                || *state.waiters.get(owner).unwrap_or(&0) >= limits.max_waiters_per_target
                || next.is_none()
                || limits.max_waiting_bytes.is_some_and(|n| next.unwrap() > n)
            {
                return Err(FrameworkError::ResourceLimit("admission waiters".into()));
            }
            state.metrics.waiters += 1;
            state.metrics.waiting_bytes = next.unwrap();
            *state.waiters.entry(owner.clone()).or_default() += 1;
        } else {
            let next = state.metrics.in_flight_bytes.checked_add(bytes);
            let owner_next = state.bytes.get(owner).unwrap_or(&0).checked_add(bytes);
            if next.is_none()
                || owner_next.is_none()
                || limits
                    .max_in_flight_bytes
                    .is_some_and(|n| next.unwrap() > n)
                || limits
                    .max_in_flight_bytes_per_target
                    .is_some_and(|n| owner_next.unwrap() > n)
            {
                return Err(FrameworkError::Backpressure(owner.clone()));
            }
            state.metrics.in_flight_bytes = next.unwrap();
            state.bytes.insert(owner.clone(), owner_next.unwrap());
        }
        Ok(BudgetGuard {
            budget: self.clone(),
            owner: Some(owner.clone()),
            bytes,
            waiting,
        })
    }
}
impl Drop for BudgetGuard {
    fn drop(&mut self) {
        if !self.waiting && self.bytes == 0 {
            return;
        }
        let mut state = self.budget.state.lock().unwrap();
        let map = if self.waiting {
            state.metrics.waiters -= 1;
            state.metrics.waiting_bytes -= self.bytes;
            &mut state.waiters
        } else {
            state.metrics.in_flight_bytes -= self.bytes;
            &mut state.bytes
        };
        let count = map.get_mut(self.owner.as_ref().unwrap()).unwrap();
        *count -= if self.waiting { 1 } else { self.bytes };
        if *count == 0 {
            map.remove(self.owner.as_ref().unwrap());
        }
        drop(state);
        if !self.waiting && self.bytes != 0 {
            self.budget.changed.notify_waiters();
        }
    }
}
