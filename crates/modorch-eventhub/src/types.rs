use modorch_core::*;
use std::{any::Any, collections::BTreeMap, sync::Arc, time::Instant};

pub type MessagePayload = Arc<dyn Any + Send + Sync>;
pub type HandlerReply = MessagePayload;
pub type SubscriptionId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageKind {
    Event,
    Command,
    Request,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    Broadcast,
    Component(ComponentRef),
    Named(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationPattern {
    Any,
    Exact(Destination),
}
#[derive(Clone)]
pub struct MessageEnvelope {
    pub message_id: OperationId,
    pub kind: MessageKind,
    pub topic: Topic,
    pub source: Option<ComponentRef>,
    pub destination: Destination,
    pub lane_key: Option<LaneKey>,
    pub correlation_id: Option<OperationId>,
    pub causation_id: Option<OperationId>,
    pub deadline: Option<Instant>,
    pub headers: BTreeMap<String, String>,
    pub payload: MessagePayload,
}
impl MessageEnvelope {
    pub fn payload<T: Send + Sync + 'static>(&self) -> FrameworkResult<Arc<T>> {
        self.payload
            .clone()
            .downcast()
            .map_err(|_| FrameworkError::TypeMismatch(self.topic.clone()))
    }
}
#[derive(Clone, Debug)]
pub struct SubscriptionSpec {
    pub id: SubscriptionId,
    pub topic_pattern: String,
    pub destination_pattern: DestinationPattern,
    pub target: ComponentRef,
}
impl SubscriptionSpec {
    pub fn new(id: SubscriptionId, target: ComponentRef, pattern: impl Into<String>) -> Self {
        Self {
            id,
            topic_pattern: pattern.into(),
            destination_pattern: DestinationPattern::Any,
            target,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubscriptionReceipt {
    pub id: SubscriptionId,
    pub generation: u64,
}
#[derive(Clone, Debug)]
pub struct AdmissionReceipt {
    pub operation_id: OperationId,
    pub target: ComponentRef,
}
#[derive(Clone, Debug)]
pub struct Delivery {
    pub target: ComponentRef,
    pub result: FrameworkResult<AdmissionReceipt>,
}
#[derive(Clone, Debug)]
pub struct PublishReport {
    pub deliveries: Vec<Delivery>,
}
impl PublishReport {
    pub fn accepted(&self) -> usize {
        self.deliveries.iter().filter(|d| d.result.is_ok()).count()
    }
    pub fn is_partial(&self) -> bool {
        self.accepted() > 0 && self.accepted() < self.deliveries.len()
    }
    pub fn rejected(&self) -> usize {
        self.deliveries.len() - self.accepted()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishPolicy {
    Reject,
    DropNewest,
}
#[derive(Clone, Debug, Default)]
pub struct MessageOptions {
    /// Declared retained payload cost; required when byte budgets are configured.
    pub payload_bytes: Option<usize>,
    pub lane_key: Option<LaneKey>,
    pub deadline: Option<Instant>,
    pub headers: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    Queued,
    Running,
    Completed,
    CanceledBeforeStart,
    CompletedAfterCallerTimeout,
}

pub(crate) fn validate_pattern(pattern: &str) -> FrameworkResult<()> {
    if pattern.is_empty()
        || pattern.split('/').enumerate().any(|(i, s)| {
            s.is_empty()
                || (s.contains('#') && (s != "#" || i + 1 != pattern.split('/').count()))
                || (s.contains('+') && s != "+")
                || s == ":"
        })
    {
        return Err(FrameworkError::InvalidConfig(format!(
            "invalid topic pattern {pattern}"
        )));
    }
    Ok(())
}
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    let mut actual = topic.split('/');
    for segment in pattern.split('/') {
        if segment == "#" {
            return true;
        }
        let Some(value) = actual.next() else {
            return false;
        };
        if value.is_empty() || !(segment == "+" || segment.starts_with(':') || segment == value) {
            return false;
        }
    }
    actual.next().is_none()
}
