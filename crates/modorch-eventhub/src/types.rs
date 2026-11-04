use modorch_core::*;
use std::{any::Any, collections::BTreeMap, sync::Arc, time::Instant};

/// Shared in-process payload; not a serialized or durable transport representation.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type MessagePayload = Arc<dyn Any + Send + Sync>;
/// Type-erased shared handler result, downcast by typed RequestHandle.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type HandlerReply = MessagePayload;
/// Hub-local unique subscription identity.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type SubscriptionId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Delivery intent: Event, Command or Request.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum MessageKind {
    /// Independent fanout admission report without a business reply.
    Event,
    /// Unique-target acceptance acknowledgement without a business reply.
    Command,
    /// Unique-target typed business response.
    Request,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Routing selector combined with the actual topic.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum Destination {
    /// Match all topic-compatible subscribed destinations.
    Broadcast,
    /// Restrict routing to one component identity.
    Component(ComponentRef),
    /// Owner-defined destination name, independent from the topic.
    Named(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Destination constraint on a subscription.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum DestinationPattern {
    /// Accept any destination selector.
    Any,
    /// Require an exact destination selector.
    Exact(Destination),
}
#[derive(Clone)]
/// Framework-generated message identity, causality, routing and shared payload.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MessageEnvelope {
    /// Framework-generated tracked operation identity.
    pub message_id: OperationId,
    /// Framework component role or message intent, as indicated by this type.
    pub kind: MessageKind,
    /// Validated MQTT Topic Name; no NUL or wildcards, at most 65535 UTF-8 bytes.
    pub topic: Topic,
    /// Original error or framework-generated source component, as indicated by the type.
    pub source: Option<ComponentRef>,
    /// Routing selector; lane keys do not alter it.
    pub destination: Destination,
    /// Optional local ordering key; does not select recipients.
    pub lane_key: Option<LaneKey>,
    /// Framework-supplied request correlation identity.
    pub correlation_id: Option<OperationId>,
    /// Framework-supplied parent operation identity.
    pub causation_id: Option<OperationId>,
    /// Optional absolute monotonic message deadline.
    pub deadline: Option<Instant>,
    /// Caller metadata bounded by HubLimits.
    pub headers: BTreeMap<String, String>,
    /// Shared, Send + Sync type-erased in-process data.
    pub payload: MessagePayload,
}
impl MessageEnvelope {
    /// Clone and downcast the shared payload; an incompatible type returns TypeMismatch.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn payload<T: Send + Sync + 'static>(&self) -> FrameworkResult<Arc<T>> {
        self.payload
            .clone()
            .downcast()
            .map_err(|_| FrameworkError::TypeMismatch(self.topic.clone()))
    }
}
#[derive(Clone, Debug)]
/// A target component, topic pattern and optional destination constraint.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct SubscriptionSpec {
    /// Identity local to its component or subscription scope.
    pub id: SubscriptionId,
    /// MQTT Topic Filter: whole-level +, final #, empty levels; :id is literal.
    pub topic_pattern: String,
    /// Additional destination matching constraint.
    pub destination_pattern: DestinationPattern,
    /// Resolved destination component.
    pub target: ComponentRef,
}
impl SubscriptionSpec {
    /// Create a topic subscription accepting any destination.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
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
/// Acknowledges that a subscription control-plane change already took effect.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct SubscriptionReceipt {
    /// Identity local to its component or subscription scope.
    pub id: SubscriptionId,
    /// Component generation used to reject stale capabilities.
    pub generation: u64,
}
#[derive(Clone, Debug)]
/// Acknowledges queue acceptance only; does not acknowledge handler completion.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct AdmissionReceipt {
    /// Tracked operation identity for diagnostics and causal accounting.
    pub operation_id: OperationId,
    /// Resolved destination component.
    pub target: ComponentRef,
}
#[derive(Clone, Debug)]
/// One fanout target and its admission outcome.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct Delivery {
    /// Resolved destination component.
    pub target: ComponentRef,
    /// Per-target admission result, not business completion.
    pub result: FrameworkResult<AdmissionReceipt>,
}
#[derive(Clone, Debug)]
/// Independent per-target admission outcomes; fanout is not atomic.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct PublishReport {
    /// Independent per-target admission results.
    pub deliveries: Vec<Delivery>,
}
impl PublishReport {
    /// Count successful per-target admissions, not completed handler executions.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn accepted(&self) -> usize {
        self.deliveries.iter().filter(|d| d.result.is_ok()).count()
    }
    /// Return true if at least one target accepted and at least one rejected.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn is_partial(&self) -> bool {
        self.accepted() > 0 && self.accepted() < self.deliveries.len()
    }
    /// Count per-target admission rejections.
    ///
    /// Contract: MSG-CMD-001 / MSG-REQ-004. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn rejected(&self) -> usize {
        self.deliveries.len() - self.accepted()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit nonblocking rejection/drop policy, preserving per-target outcomes.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum PublishPolicy {
    /// Explicitly report rejected admissions without waiting.
    Reject,
    /// Explicitly drop the newest rejected delivery and preserve its rejection report.
    DropNewest,
}
#[derive(Clone, Debug, Default)]
/// Caller metadata and declared payload cost; causal identity is framework-owned.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MessageOptions {
    /// Declared retained payload cost; required when byte budgets are configured.
    pub payload_bytes: Option<usize>,
    /// Optional local ordering key; does not select recipients.
    pub lane_key: Option<LaneKey>,
    /// Optional absolute monotonic message deadline.
    pub deadline: Option<Instant>,
    /// Caller metadata bounded by HubLimits.
    pub headers: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Execution progress independent of whether the caller is still waiting.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum RequestStatus {
    /// Accepted but business execution has not started.
    Queued,
    /// Business work has begun and remains tracked.
    Running,
    /// A terminal result is available.
    Completed,
    /// The request was canceled before business execution began.
    CanceledBeforeStart,
    /// Execution finished after the caller stopped waiting due to timeout.
    CompletedAfterCallerTimeout,
}

/// Match an MQTT Topic Filter against a Topic Name (MQTT 3.1.1 / 5.0 §4.7).
///
/// `+` consumes one level, including an empty level; final `#` consumes zero or
/// more. Root wildcards exclude names starting with `$`. `:id` is literal.
/// Both inputs must be nonempty, contain no NUL and fit 65535 UTF-8 bytes;
/// names cannot contain wildcards. Invalid input returns false. No trimming or
/// Unicode normalization occurs. This helper does not apply HubLimits budgets.
///
/// Contract: MSG-TOPIC-001 / MSG-TOPIC-002 / MSG-TOPIC-003.
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    crate::topic::validate_name(topic).is_ok()
        && crate::topic::TopicFilter::parse(pattern)
            .is_ok_and(|filter| filter.matches_validated(topic))
}
