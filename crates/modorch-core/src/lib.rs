//! Shared identities, structured failures and causal operation accounting.
//!
//! # Ownership and errors
//! OperationTracker owns one application admission gate. A live OperationLease keeps
//! work counted until its final drop. Closing root admission preserves lawful causal
//! children; termination is irreversible. ExecutionFailed wraps the original error.
//! Contract: LIFE-003, MSG-REQ-004, SCH-001.
//!
//! # Examples
//!
//! ```
//! use modorch_core::{ComponentDescriptor, ComponentRef, ExecutionPolicy};
//! let descriptor = ComponentDescriptor::new(ComponentRef::module("orders"));
//! assert_eq!(descriptor.execution, ExecutionPolicy::Serial);
//! assert_eq!(descriptor.queue_capacity, 64);
//! ```
//!
//! # Stability
//! Development API 0.0.2. Lifecycle phase and generation identities are process-local,
//! not durable retry or transaction guarantees. The host controls actual wait budgets.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
mod operation;
pub use operation::*;
use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

pub use semver::Version;
/// Framework operation result with a structured FrameworkError.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type FrameworkResult<T> = Result<T, FrameworkError>;
/// Application-supplied JSON component configuration; factories validate its schema.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type ComponentConfig = serde_json::Value;
/// Component name, unique together with ComponentKind inside one application.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type ComponentId = String;
/// Process-local identity of an isolated application tracker.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type ApplicationId = u64;
/// Process-local identity of tracked work; not a durable idempotency key.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type OperationId = u64;
/// Owner-defined, versioned message topic string.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type Topic = String;
/// Logical ordering key; never selects a destination.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub type LaneKey = String;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
/// The only two framework-level component roles.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum ComponentKind {
    /// Shared-resource provider role.
    Initiator,
    /// Business behavior role; may hold private state.
    Module,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
/// Component identity: role plus application-local name.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentRef {
    /// Framework component role or message intent, as indicated by this type.
    pub kind: ComponentKind,
    /// Identity local to its component or subscription scope.
    pub id: ComponentId,
}
impl ComponentRef {
    /// Create a initiator-role component identity from an application-local name.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn initiator(id: impl Into<String>) -> Self {
        Self {
            kind: ComponentKind::Initiator,
            id: id.into(),
        }
    }
    /// Create a module-role component identity from an application-local name.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn module(id: impl Into<String>) -> Self {
        Self {
            kind: ComponentKind::Module,
            id: id.into(),
        }
    }
}
impl std::fmt::Display for ComponentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}:{}", self.kind, self.id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Component execution domains and concurrency limits; Serial is the descriptor default.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum ExecutionPolicy {
    /// One mutable handler and bounded FIFO mailbox.
    Serial,
    /// Independent serial handlers on fixed shards.
    Partitioned {
        /// Nonzero fixed shard count; at most 4096.
        shards: NonZeroUsize,
    },
    /// Shared handler with a strict concurrency bound and no ordering guarantee.
    Concurrent {
        /// Maximum simultaneous work in this execution domain.
        max_in_flight: NonZeroUsize,
    },
    /// Bounded per-key serial lanes retired after idle timeout.
    Dynamic {
        /// Maximum combined active and preparing dynamic lanes; at most 4096.
        max_lanes: NonZeroUsize,
        /// Positive empty-lane retirement interval; does not preserve handler private state.
        idle_timeout: Duration,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
/// Supervision policy; Restart rebuilds generations but never replays messages.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum FailurePolicy {
    #[default]
    /// Default policy: fail the application and request checked shutdown.
    FailFast,
    /// Stop the failed component and its enabled dependent closure.
    StopDependents,
    /// Drain and teardown the old dependent closure before bounded generation rebuild.
    Restart {
        /// Cumulative rebuild attempt budget within this application lifetime.
        max_attempts: u32,
        /// Delay before a rebuild attempt; normal shutdown can interrupt it.
        backoff: Duration,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Stable metadata frozen at build time and checked against each created instance.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentDescriptor {
    /// Identity local to its component or subscription scope.
    pub id: ComponentId,
    /// Framework component role or message intent, as indicated by this type.
    pub kind: ComponentKind,
    /// Owner-declared component interface version.
    pub version: Version,
    /// Required registered and enabled dependencies.
    pub requires: Vec<ComponentRef>,
    /// Dependencies that participate in ordering only when already enabled.
    pub optional: Vec<ComponentRef>,
    /// Execution policy or sampled execution counters, as indicated by the type.
    pub execution: ExecutionPolicy,
    /// Per-shard mailbox capacity for Partitioned; per-component otherwise.
    pub queue_capacity: usize,
    /// Supervision policy or latest failure, as indicated by the type.
    pub failure: FailurePolicy,
}
impl ComponentDescriptor {
    /// Create metadata with Serial execution, mailbox capacity 64 and FailFast.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn new(component: ComponentRef) -> Self {
        Self {
            id: component.id,
            kind: component.kind,
            version: Version::new(0, 1, 0),
            requires: vec![],
            optional: vec![],
            execution: ExecutionPolicy::Serial,
            queue_capacity: 64,
            failure: FailurePolicy::FailFast,
        }
    }
    /// Return the component identity attached to this value, if available.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn component(&self) -> ComponentRef {
        ComponentRef {
            kind: self.kind,
            id: self.id.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Observable lifecycle or execution boundary used in errors and diagnostics.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum Phase {
    /// Resource setup boundary.
    Setup,
    /// Start hook boundary; long-running work belongs to the scheduler.
    Start,
    /// Ingress activation after all readiness barriers passed.
    ActivateIngress,
    /// All relevant readiness barriers passed.
    Ready,
    /// Message handler execution boundary.
    Handle,
    /// Managed background task execution boundary.
    Task,
    /// Root admission closure and producer cancellation boundary.
    BeginShutdown,
    /// Stop active production without prematurely releasing resources.
    Quiesce,
    /// Wait for tracked causal work and service leases to reach zero.
    Drain,
    /// Release owned resources; failures retain retryable ownership.
    Teardown,
    /// Internal executor termination boundary.
    Terminate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Application lifecycle state; a timeout is not proof of Stopped.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum ApplicationState {
    /// Built but not started.
    New,
    /// Creating and setting up selected components.
    Initializing,
    /// Running start and readiness hooks.
    Starting,
    /// All relevant readiness barriers passed.
    Ready,
    /// A dependent closure stopped while unrelated components remain active.
    Degraded,
    /// Rebuilding an affected component closure.
    Recovering,
    /// A supervised operation or lifecycle boundary failed.
    Failed,
    /// Stopping producers and ingress.
    Quiescing,
    /// Waiting for admitted work to finish.
    Draining,
    /// Releasing resources in reverse dependency order.
    Stopping,
    /// Shutdown is incomplete; resources must remain owned.
    StopIncomplete,
    /// Irreversible graceful terminal state.
    Stopped,
    /// Forced terminal outcome; pending work is not proof of safe resource release.
    Forced,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Snapshot of tracked operations and their component owners.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct PendingWork {
    /// Tracked operation count or owner snapshot.
    pub operations: usize,
    /// Pending work grouped by component owner.
    pub owners: BTreeMap<ComponentRef, usize>,
}
#[derive(Clone, Debug, thiserror::Error)]
/// Structured admission, execution, resource and lifecycle failures.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum FrameworkError {
    #[error("duplicate component: {0}")]
    /// The same role/name identity was registered more than once.
    DuplicateComponent(ComponentRef),
    #[error("component unavailable: {0}")]
    /// An enabled component was not registered in this build.
    ComponentUnavailable(ComponentRef),
    #[error("missing dependency {dependency} required by {component}")]
    /// A required component has no registered factory.
    MissingDependency {
        /// Component associated with this boundary; absent before routing when optional.
        component: ComponentRef,
        /// Required component that is missing or disabled.
        dependency: ComponentRef,
    },
    #[error("disabled dependency {dependency} required by {component}")]
    /// A registered required dependency was not explicitly enabled.
    DisabledDependency {
        /// Component associated with this boundary; absent before routing when optional.
        component: ComponentRef,
        /// Required component that is missing or disabled.
        dependency: ComponentRef,
    },
    #[error("dependency cycle: {0:?}")]
    /// Enabled dependencies contain a cycle.
    DependencyCycle(Vec<ComponentRef>),
    #[error("invalid configuration: {0}")]
    /// An identifier, descriptor, limit, policy or business setting is invalid.
    InvalidConfig(String),
    #[error("application is not ready")]
    /// Root admission is not open yet.
    NotReady,
    #[error("admission is closed")]
    /// Root/owner admission or a generation-bound capability is closed.
    Closed,
    #[error("queue full: {0}")]
    /// Immediate admission has no available bounded capacity.
    Backpressure(ComponentRef),
    #[error("resource limit: {0}")]
    /// An explicit waiter or retained-cost budget was exceeded.
    ResourceLimit(String),
    #[error("no route for {0}")]
    /// No subscribed target matches topic and destination.
    Unroutable(String),
    #[error("ambiguous destination: {0}")]
    /// A Command or Request matched multiple target components.
    AmbiguousDestination(String),
    #[error("circular wait: {0:?}")]
    /// The framework request wait graph would contain a cycle.
    CircularWait(Vec<ComponentRef>),
    #[error("admission timed out")]
    /// The separate admission wait budget expired.
    AdmissionTimeout,
    #[error("operation {0} canceled before execution")]
    /// The request was canceled before business execution began.
    CanceledBeforeStart(OperationId),
    #[error("operation {0} may have executed")]
    /// Execution began; caller timeout/cancellation cannot prove rollback.
    Indeterminate(OperationId),
    #[error("{component} failed in {phase:?}: {detail}")]
    /// A failed boundary retaining component, phase and original source.
    ExecutionFailed {
        /// Component associated with this boundary; absent before routing when optional.
        component: ComponentRef,
        /// Absent for lifecycle/factory work outside a counted operation.
        operation_id: Option<OperationId>,
        /// Lifecycle/execution phase associated with this boundary.
        phase: Phase,
        /// Human-readable failure diagnostic; never use as a stable machine error code.
        detail: String,
        #[source]
        /// Original error or framework-generated source component, as indicated by the type.
        source: Option<Box<FrameworkError>>,
    },
    #[error("shutdown incomplete in {phase:?}: {pending:?}; errors: {errors:?}")]
    /// Shutdown is incomplete; resources must remain owned.
    StopIncomplete {
        /// Lifecycle/execution phase associated with this boundary.
        phase: Phase,
        /// Work still counted; a nonempty value prevents graceful resource release.
        pending: PendingWork,
        /// Failures encountered while compensating or shutting down.
        errors: Vec<FrameworkError>,
    },
    #[error("service unavailable: {0}")]
    /// No currently available typed service exists for this generation.
    ServiceUnavailable(String),
    #[error("service access denied: {0}")]
    /// Service publication or dependency-scoped reading is not permitted.
    AccessDenied(ComponentRef),
    #[error("payload or reply type does not match: {0}")]
    /// The payload or reply has an incompatible concrete Rust type.
    TypeMismatch(String),
    #[error("task cancelled")]
    /// Managed work acknowledged cancellation or was aborted.
    Cancelled,
}

mod failure;
pub use failure::*;

impl FrameworkError {
    /// Technical retriability only; says nothing about business idempotency.
    pub fn retryable(&self) -> bool {
        if let Self::ExecutionFailed {
            source: Some(source),
            ..
        } = self
        {
            return source.retryable();
        }
        matches!(
            self,
            Self::Backpressure(_)
                | Self::ResourceLimit(_)
                | Self::NotReady
                | Self::AdmissionTimeout
                | Self::StopIncomplete { .. }
        )
    }
    /// Return the tracked operation identity, if this boundary has one.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn operation_id(&self) -> Option<OperationId> {
        match self {
            Self::CanceledBeforeStart(id) | Self::Indeterminate(id) => Some(*id),
            Self::ExecutionFailed { operation_id, .. } => *operation_id,
            _ => None,
        }
    }
    /// Return the component identity attached to this value, if available.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn component(&self) -> Option<&ComponentRef> {
        match self {
            Self::ExecutionFailed { component, .. }
            | Self::DuplicateComponent(component)
            | Self::ComponentUnavailable(component)
            | Self::Backpressure(component)
            | Self::AccessDenied(component)
            | Self::MissingDependency { component, .. }
            | Self::DisabledDependency { component, .. } => Some(component),
            _ => None,
        }
    }
    /// Return the recorded lifecycle or execution phase, if available.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn phase(&self) -> Option<Phase> {
        match self {
            Self::ExecutionFailed { phase, .. } | Self::StopIncomplete { phase, .. } => {
                Some(*phase)
            }
            _ => None,
        }
    }
    /// Adds the current execution boundary while retaining the original error chain.
    pub fn in_execution(
        self,
        component: ComponentRef,
        operation_id: Option<OperationId>,
        phase: Phase,
    ) -> Self {
        Self::ExecutionFailed {
            component,
            operation_id,
            phase,
            detail: self.to_string(),
            source: Some(Box::new(self)),
        }
    }
    /// Follow execution wrappers to the original FrameworkError without discarding context.
    ///
    /// Contract: LIFE-003. Development API: modorch 0.0.2; see crate-level examples and ownership limits.
    pub fn root_cause(&self) -> &Self {
        match self {
            Self::ExecutionFailed {
                source: Some(source),
                ..
            } => source.root_cause(),
            _ => self,
        }
    }
}
