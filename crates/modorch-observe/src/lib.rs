//! Snapshot adapters; applications install their own tracing subscriber and exporter.
#![forbid(unsafe_code)]
use modorch_core::*;
#[derive(Clone, Debug)]
pub struct OperationMetrics {
    pub in_flight: usize,
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
    pub fn capture(application: &modorch_runtime::ApplicationHandle) -> Self {
        Self(application.metrics())
    }
}
