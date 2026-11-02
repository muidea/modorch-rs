//! Exporter-neutral snapshots for host-controlled observability.
//!
//! # Ownership and lifecycle
//! Snapshots contain counts and bounded framework diagnostics. The host installs the
//! Subscriber, exporter and sampling policy. Cross-subsystem snapshots are not atomic.
//! ApplicationMetrics is available only when runtime support is enabled.
//! Contract: LIFE-001, VAL-001.
//!
//! # Examples
//!
//! ```
//! use modorch_observe::OperationMetrics;
//! use modorch_core::PendingWork;
//! let metrics = OperationMetrics::from(PendingWork { operations: 0, owners: Default::default() });
//! assert_eq!(metrics.in_flight, 0);
//! ```
//!
//! # Errors and stability
//! Development API 0.0.1. Capture is a snapshot conversion, not a completion barrier
//! or an end-to-end business SLO. Errors remain in the original runtime result chain.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
use modorch_core::*;
#[derive(Clone, Debug)]
/// Exporter-neutral operation-count snapshot; exporters belong to the host.
///
/// Development API for modorch 0.0.1. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct OperationMetrics {
    /// Current tracked operations.
    pub in_flight: usize,
    /// Pending work grouped by component owner.
    pub owners: std::collections::BTreeMap<ComponentRef, usize>,
}
impl From<PendingWork> for OperationMetrics {
    fn from(p: PendingWork) -> Self {
        Self {
            in_flight: p.operations,
            owners: p.owners,
        }
    }
}

/// Complete application-scoped inputs for a host-owned metrics exporter.
#[cfg(feature = "runtime")]
#[derive(Clone, Debug)]
pub struct ApplicationMetrics(pub modorch_runtime::RuntimeMetrics);
#[cfg(feature = "runtime")]
impl ApplicationMetrics {
    /// Capture the runtime metrics snapshot without installing a global exporter.
    ///
    /// Contract: VAL-001. Development API: modorch 0.0.1; see crate-level examples and ownership limits.
    pub fn capture(application: &modorch_runtime::ApplicationHandle) -> Self {
        Self(application.metrics())
    }
}
