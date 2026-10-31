use crate::*;
use std::time::Duration;

impl EventHub {
    /// Wait for capacity without holding the routing lock or allocating an unbounded
    /// mailbox. Cancellation drops the waiter without a late enqueue.
    async fn await_admission<T>(
        &self,
        deadline: tokio::time::Instant,
        kind: MessageKind,
        topic: &str,
        target: Option<&ComponentRef>,
        bytes: usize,
        mut submit: impl FnMut(tokio::time::Instant) -> FrameworkResult<T>,
    ) -> FrameworkResult<T> {
        if self.0.tracker.current().is_some() && EXECUTION.try_with(|_| ()).is_ok() {
            let error = FrameworkError::InvalidConfig(
                "waiting for mailbox capacity from a handler is forbidden; use immediate admission"
                    .into(),
            );
            self.record_route::<()>(target, kind, topic, &Err(error.clone()));
            return Err(error);
        }
        let timed_out = |target: Option<&ComponentRef>| {
            let error = FrameworkError::AdmissionTimeout;
            self.record_route::<()>(target, kind, topic, &Err(error.clone()));
            error
        };
        let mut waiting: Option<(ComponentRef, limits::BudgetGuard)> = None;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(timed_out(
                    waiting.as_ref().map(|(owner, _)| owner).or(target),
                ));
            }
            let endpoint_changed = waiting.as_ref().and_then(|(owner, _)| {
                self.0
                    .routing
                    .lock()
                    .unwrap()
                    .endpoints
                    .get(owner)
                    .map(|endpoint| endpoint.capacity_changed.clone())
            });
            let endpoint_changed =
                endpoint_changed.unwrap_or_else(|| self.0.capacity_changed.clone());
            let endpoint_notify = endpoint_changed.notified();
            tokio::pin!(endpoint_notify);
            endpoint_notify.as_mut().enable();
            let notified = self.0.capacity_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let gate = self.0.tracker.admission_notification();
            tokio::pin!(gate);
            gate.as_mut().enable();
            let last_target = match submit(deadline) {
                Ok(value) => return Ok(value),
                Err(FrameworkError::Backpressure(owner)) => owner,
                Err(error) => return Err(error),
            };
            if waiting
                .as_ref()
                .is_none_or(|(owner, _)| owner != &last_target)
            {
                waiting.take();
                let guard = self
                    .0
                    .budget
                    .acquire(&last_target, bytes, true)
                    .inspect_err(|error| {
                        self.record_route::<()>(
                            Some(&last_target),
                            kind,
                            topic,
                            &Err(error.clone()),
                        );
                    })?;
                waiting = Some((last_target.clone(), guard));
                continue;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(timed_out(Some(&last_target)));
            }
            tokio::select! {biased; _=tokio::time::sleep_until(deadline)=>return Err(timed_out(Some(&last_target))), _=notified=>{}, _=&mut endpoint_notify=>{},_=&mut gate=>{}}
        }
    }
    pub async fn command_wait<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        command: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<AdmissionReceipt> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| FrameworkError::InvalidConfig("admission timeout overflow".into()))?;
        let bytes = self.validate_message(MessageKind::Command, topic, &destination, &options)?;
        let payload: MessagePayload = Arc::new(command);
        self.await_admission(
            deadline,
            MessageKind::Command,
            topic,
            None,
            bytes,
            |deadline| {
                let (mut routing, target) = self.prepare_target(
                    MessageKind::Command,
                    topic,
                    &destination,
                    &options.lane_key,
                    None,
                    Some(deadline),
                )?;
                self.send(
                    &mut routing,
                    &target,
                    MessageKind::Command,
                    topic,
                    &destination,
                    payload.clone(),
                    options.clone(),
                    false,
                    Some(deadline),
                )
                .map(|(receipt, _)| receipt)
            },
        )
        .await
    }
    pub async fn request_wait<T: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        request: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<RequestHandle<R>> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| FrameworkError::InvalidConfig("admission timeout overflow".into()))?;
        let bytes = self.validate_message(MessageKind::Request, topic, &destination, &options)?;
        let payload: MessagePayload = Arc::new(request);
        self.await_admission(
            deadline,
            MessageKind::Request,
            topic,
            None,
            bytes,
            |deadline| {
                let (mut routing, target) = self.prepare_target(
                    MessageKind::Request,
                    topic,
                    &destination,
                    &options.lane_key,
                    None,
                    Some(deadline),
                )?;
                let (receipt, shared) = self.send(
                    &mut routing,
                    &target,
                    MessageKind::Request,
                    topic,
                    &destination,
                    payload.clone(),
                    options.clone(),
                    true,
                    Some(deadline),
                )?;
                let shared = shared.expect("request state");
                Ok(RequestHandle {
                    target: target.clone(),
                    id: receipt.operation_id,
                    shared,
                    edge: None,
                    marker: std::marker::PhantomData,
                })
            },
        )
        .await
    }
    /// Acknowledges targets independently. Already admitted fanout legs are never
    /// retried. Timed out targets remain explicit errors in the delivery report.
    pub async fn publish_wait<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        event: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<PublishReport> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| FrameworkError::InvalidConfig("admission timeout overflow".into()))?;
        let bytes = self.validate_message(MessageKind::Event, topic, &destination, &options)?;
        let payload: MessagePayload = Arc::new(event);
        let targets = {
            let routing = self.0.routing.lock().unwrap();
            if routing.terminated {
                self.record_route::<()>(
                    None,
                    MessageKind::Event,
                    topic,
                    &Err(FrameworkError::Closed),
                );
                return Err(FrameworkError::Closed);
            }
            Self::targets(&routing, topic, &destination)
        };
        if targets.is_empty() {
            self.record_route::<()>(
                None,
                MessageKind::Event,
                topic,
                &Err(FrameworkError::Unroutable(topic.into())),
            );
            return Err(FrameworkError::Unroutable(topic.into()));
        }
        let deliveries = futures_util::future::join_all(targets.into_iter().map(|target| {
            let payload = payload.clone();
            let destination = &destination;
            let options = &options;
            async move {
                let result = self
                    .await_admission(
                        deadline,
                        MessageKind::Event,
                        topic,
                        Some(&target),
                        bytes,
                        |deadline| {
                            let (mut routing, _) = self.prepare_target(
                                MessageKind::Event,
                                topic,
                                destination,
                                &options.lane_key,
                                Some(&target),
                                Some(deadline),
                            )?;
                            self.send(
                                &mut routing,
                                &target,
                                MessageKind::Event,
                                topic,
                                destination,
                                payload.clone(),
                                options.clone(),
                                false,
                                Some(deadline),
                            )
                            .map(|(receipt, _)| receipt)
                        },
                    )
                    .await;
                Delivery { target, result }
            }
        }))
        .await;
        Ok(PublishReport { deliveries })
    }
}
impl ExternalEventHub {
    pub async fn command_wait<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        command: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<AdmissionReceipt> {
        self.0
            .0
            .tracker
            .scope_root(
                self.0
                    .command_wait(topic, destination, command, options, timeout),
            )
            .await
    }
    pub async fn request_wait<T: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        request: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<RequestHandle<R>> {
        self.0
            .0
            .tracker
            .scope_root(
                self.0
                    .request_wait(topic, destination, request, options, timeout),
            )
            .await
    }
    pub async fn publish_wait<T: Send + Sync + 'static>(
        &self,
        topic: &str,
        destination: Destination,
        event: T,
        options: MessageOptions,
        timeout: Duration,
    ) -> FrameworkResult<PublishReport> {
        self.0
            .0
            .tracker
            .scope_root(
                self.0
                    .publish_wait(topic, destination, event, options, timeout),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn expired_waiter_never_retries_when_capacity_and_timer_are_both_ready() {
        for _ in 0..64 {
            let tracker = OperationTracker::new();
            tracker.open().unwrap();
            let hub = EventHub::new(tracker, FailureReporter::new());
            let owner = ComponentRef::module("full");
            let deadline = tokio::time::Instant::now() + Duration::from_millis(10);
            let calls = std::cell::Cell::new(0);
            let wait = hub.await_admission(
                deadline,
                MessageKind::Command,
                "test",
                Some(&owner),
                0,
                |_| {
                    calls.set(calls.get() + 1);
                    if tokio::time::Instant::now() >= deadline {
                        Ok(())
                    } else {
                        Err(FrameworkError::Backpressure(owner.clone()))
                    }
                },
            );
            tokio::pin!(wait);
            assert!(futures_util::poll!(&mut wait).is_pending());
            let before = calls.get();
            tokio::time::advance(Duration::from_millis(10)).await;
            hub.0.capacity_changed.notify_waiters();
            assert!(matches!(wait.await, Err(FrameworkError::AdmissionTimeout)));
            assert_eq!(calls.get(), before);
            assert_eq!(hub.metrics().admission.waiters, 0);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn zero_admission_window_never_submits() {
        let tracker = OperationTracker::new();
        tracker.open().unwrap();
        let hub = EventHub::new(tracker, FailureReporter::new());
        let result: FrameworkResult<()> = hub
            .await_admission(
                tokio::time::Instant::now(),
                MessageKind::Command,
                "test",
                None,
                0,
                |_| panic!("expired admission must not submit"),
            )
            .await;
        assert!(matches!(result, Err(FrameworkError::AdmissionTimeout)));
    }
}
