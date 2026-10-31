//! Shared contracts and application-scoped operation accounting.
#![forbid(unsafe_code)]
mod operation;
pub use operation::*;
use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

pub use semver::Version;
pub type FrameworkResult<T> = Result<T, FrameworkError>;
pub type ComponentConfig = serde_json::Value;
pub type ComponentId = String;
pub type ApplicationId = u64;
pub type OperationId = u64;
pub type Topic = String;
pub type LaneKey = String;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ComponentKind {
    Initiator,
    Module,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ComponentRef {
    pub kind: ComponentKind,
    pub id: ComponentId,
}
impl ComponentRef {
    pub fn initiator(id: impl Into<String>) -> Self {
        Self {
            kind: ComponentKind::Initiator,
            id: id.into(),
        }
    }
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
pub enum ExecutionPolicy {
    Serial,
    Partitioned {
        shards: NonZeroUsize,
    },
    Concurrent {
        max_in_flight: NonZeroUsize,
    },
    Dynamic {
        max_lanes: NonZeroUsize,
        idle_timeout: Duration,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum FailurePolicy {
    #[default]
    FailFast,
    StopDependents,
    Restart {
        max_attempts: u32,
        backoff: Duration,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentDescriptor {
    pub id: ComponentId,
    pub kind: ComponentKind,
    pub version: Version,
    pub requires: Vec<ComponentRef>,
    pub optional: Vec<ComponentRef>,
    pub execution: ExecutionPolicy,
    /// Per-shard mailbox capacity for Partitioned; per-component otherwise.
    pub queue_capacity: usize,
    pub failure: FailurePolicy,
}
impl ComponentDescriptor {
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
    pub fn component(&self) -> ComponentRef {
        ComponentRef {
            kind: self.kind,
            id: self.id.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Setup,
    Start,
    ActivateIngress,
    Ready,
    Handle,
    Task,
    BeginShutdown,
    Quiesce,
    Drain,
    Teardown,
    Terminate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationState {
    New,
    Initializing,
    Starting,
    Ready,
    Degraded,
    Recovering,
    Failed,
    Quiescing,
    Draining,
    Stopping,
    StopIncomplete,
    Stopped,
    Forced,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWork {
    pub operations: usize,
    pub owners: BTreeMap<ComponentRef, usize>,
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum FrameworkError {
    #[error("duplicate component: {0}")]
    DuplicateComponent(ComponentRef),
    #[error("component unavailable: {0}")]
    ComponentUnavailable(ComponentRef),
    #[error("missing dependency {dependency} required by {component}")]
    MissingDependency {
        component: ComponentRef,
        dependency: ComponentRef,
    },
    #[error("disabled dependency {dependency} required by {component}")]
    DisabledDependency {
        component: ComponentRef,
        dependency: ComponentRef,
    },
    #[error("dependency cycle: {0:?}")]
    DependencyCycle(Vec<ComponentRef>),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("application is not ready")]
    NotReady,
    #[error("admission is closed")]
    Closed,
    #[error("queue full: {0}")]
    Backpressure(ComponentRef),
    #[error("resource limit: {0}")]
    ResourceLimit(String),
    #[error("no route for {0}")]
    Unroutable(String),
    #[error("ambiguous destination: {0}")]
    AmbiguousDestination(String),
    #[error("circular wait: {0:?}")]
    CircularWait(Vec<ComponentRef>),
    #[error("admission timed out")]
    AdmissionTimeout,
    #[error("operation {0} canceled before execution")]
    CanceledBeforeStart(OperationId),
    #[error("operation {0} may have executed")]
    Indeterminate(OperationId),
    #[error("{component} failed in {phase:?}: {detail}")]
    ExecutionFailed {
        component: ComponentRef,
        /// Absent for lifecycle/factory work outside a counted operation.
        operation_id: Option<OperationId>,
        phase: Phase,
        detail: String,
        #[source]
        source: Option<Box<FrameworkError>>,
    },
    #[error("shutdown incomplete in {phase:?}: {pending:?}; errors: {errors:?}")]
    StopIncomplete {
        phase: Phase,
        pending: PendingWork,
        errors: Vec<FrameworkError>,
    },
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("service access denied: {0}")]
    AccessDenied(ComponentRef),
    #[error("payload or reply type does not match: {0}")]
    TypeMismatch(String),
    #[error("task cancelled")]
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
    pub fn operation_id(&self) -> Option<OperationId> {
        match self {
            Self::CanceledBeforeStart(id) | Self::Indeterminate(id) => Some(*id),
            Self::ExecutionFailed { operation_id, .. } => *operation_id,
            _ => None,
        }
    }
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
