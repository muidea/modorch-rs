//! Explicit composition, dependency ordering and typed leased services.
//!
//! # Ownership and lifecycle
//! Only Initiators publish services. Readers are bound to the borrower generation and
//! allowed dependency set. A ServiceLease protects both borrower and resource Owner;
//! final closure belongs to the Initiator for Owned, and to the host for Borrowed.
//! Contract: COMP-002, DEP-001, RES-001 / RES-002 / RES-003.
//!
//! # Errors
//! MissingDependency and DisabledDependency are distinct. Optional dependencies never
//! auto-enable. Duplicate owner/name/type service keys are InvalidConfig; inaccessible
//! services return AccessDenied or ServiceUnavailable. No implicit blocking timeout.
//!
//! # Examples
//!
//! ```
//! use modorch_registry::ComponentSelection;
//! use modorch_core::ComponentRef;
//! let mut enabled = ComponentSelection::new();
//! assert_eq!(enabled.components().count(), 0);
//! enabled.enable_module("orders");
//! assert_eq!(enabled.components().next(), Some(&ComponentRef::module("orders")));
//! ```
//!
//! # Stability
//! Development API 0.0.3. Business applications use the modorch Facade.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
mod services;
use modorch_core::*;
pub use services::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
/// Explicit enabled components and their configuration; empty by default.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentSelection {
    enabled: BTreeMap<ComponentRef, ComponentConfig>,
}
impl ComponentSelection {
    /// Create an empty explicit selection.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn new() -> Self {
        Self::default()
    }
    /// Explicitly select an Initiator; registration is still required at build.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn enable_initiator(&mut self, id: impl Into<String>) -> &mut Self {
        self.enable(ComponentRef::initiator(id))
    }
    /// Explicitly select a Module; registration is still required at build.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn enable_module(&mut self, id: impl Into<String>) -> &mut Self {
        self.enable(ComponentRef::module(id))
    }
    /// Explicitly select a component without replacing existing configuration.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn enable(&mut self, component: ComponentRef) -> &mut Self {
        self.enabled
            .entry(component)
            .or_insert(ComponentConfig::Null);
        self
    }
    /// Select a component and replace its application-supplied configuration.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn configure(&mut self, component: ComponentRef, config: ComponentConfig) -> &mut Self {
        self.enabled.insert(component, config);
        self
    }
    /// Iterate selected component identities in deterministic order.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn components(&self) -> impl Iterator<Item = &ComponentRef> {
        self.enabled.keys()
    }
    /// Clone a selected component's configuration, defaulting to JSON null if absent.
    ///
    /// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn config(&self, component: &ComponentRef) -> ComponentConfig {
        self.enabled.get(component).cloned().unwrap_or_default()
    }
}
#[derive(Clone, Copy, Debug, Default)]
/// Whether required dependencies must be selected or are explicitly included.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub enum DependencyResolution {
    #[default]
    /// Default: every registered required dependency must already be selected.
    RequireSelected,
    /// Explicitly extend selection with registered required dependencies only.
    IncludeRequired,
}

/// Performs all validation before invoking any factory. Optional selected dependencies
/// order startup, but never enable otherwise disabled components.
pub fn resolve(
    descriptors: &BTreeMap<ComponentRef, ComponentDescriptor>,
    selection: &ComponentSelection,
    policy: DependencyResolution,
) -> FrameworkResult<Vec<ComponentRef>> {
    let mut enabled: BTreeSet<_> = selection.components().cloned().collect();
    for component in &enabled {
        if !descriptors.contains_key(component) {
            return Err(FrameworkError::ComponentUnavailable(component.clone()));
        }
    }
    loop {
        let mut extra = vec![];
        for component in &enabled {
            let d = &descriptors[component];
            validate(d)?;
            for dep in d.requires.iter().chain(d.optional.iter()) {
                if component.kind == ComponentKind::Initiator && dep.kind == ComponentKind::Module {
                    return Err(FrameworkError::InvalidConfig(format!(
                        "{component} cannot depend on {dep}"
                    )));
                }
            }
            for dep in &d.requires {
                if !descriptors.contains_key(dep) {
                    return Err(FrameworkError::MissingDependency {
                        component: component.clone(),
                        dependency: dep.clone(),
                    });
                }
                if !enabled.contains(dep) {
                    match policy {
                        DependencyResolution::RequireSelected => {
                            return Err(FrameworkError::DisabledDependency {
                                component: component.clone(),
                                dependency: dep.clone(),
                            });
                        }
                        DependencyResolution::IncludeRequired => extra.push(dep.clone()),
                    }
                }
            }
        }
        if extra.is_empty() {
            break;
        }
        enabled.extend(extra);
    }
    fn visit(
        c: &ComponentRef,
        descriptors: &BTreeMap<ComponentRef, ComponentDescriptor>,
        enabled: &BTreeSet<ComponentRef>,
        visiting: &mut Vec<ComponentRef>,
        done: &mut BTreeSet<ComponentRef>,
        result: &mut Vec<ComponentRef>,
    ) -> FrameworkResult<()> {
        if done.contains(c) {
            return Ok(());
        }
        if visiting.contains(c) {
            let mut cycle = visiting.clone();
            cycle.push(c.clone());
            return Err(FrameworkError::DependencyCycle(cycle));
        }
        visiting.push(c.clone());
        let d = &descriptors[c];
        let deps: BTreeSet<_> = d
            .requires
            .iter()
            .chain(&d.optional)
            .filter(|d| enabled.contains(*d))
            .collect();
        for dep in deps {
            visit(dep, descriptors, enabled, visiting, done, result)?;
        }
        visiting.pop();
        done.insert(c.clone());
        result.push(c.clone());
        Ok(())
    }
    let mut result = vec![];
    let mut done = BTreeSet::new();
    for c in &enabled {
        visit(
            c,
            descriptors,
            &enabled,
            &mut vec![],
            &mut done,
            &mut result,
        )?;
    }
    Ok(result)
}
/// Reject invalid or overflowing resource limits before any work is created.
///
/// Contract: RES-001 / DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
pub fn validate(d: &ComponentDescriptor) -> FrameworkResult<()> {
    if d.id.trim().is_empty() || d.queue_capacity == 0 || d.queue_capacity > tokio_capacity_limit()
    {
        return Err(FrameworkError::InvalidConfig(format!(
            "invalid ID or capacity for {}",
            d.component()
        )));
    }
    if let FailurePolicy::Restart {
        max_attempts,
        backoff,
    } = d.failure
    {
        if max_attempts == 0 || std::time::Instant::now().checked_add(backoff).is_none() {
            return Err(FrameworkError::InvalidConfig(
                "invalid restart budget or backoff".into(),
            ));
        }
    }
    match d.execution {
        ExecutionPolicy::Dynamic {
            max_lanes,
            idle_timeout,
        } if max_lanes.get() > 4096
            || idle_timeout.is_zero()
            || std::time::Instant::now()
                .checked_add(idle_timeout)
                .is_none() =>
        {
            return Err(FrameworkError::InvalidConfig(
                "invalid dynamic lane limit or idle timeout".into(),
            ));
        }
        ExecutionPolicy::Partitioned { shards } if shards.get() > 4096 => {
            return Err(FrameworkError::InvalidConfig("shards exceed 4096".into()));
        }
        ExecutionPolicy::Concurrent { max_in_flight } if max_in_flight.get() > 4096 => {
            return Err(FrameworkError::InvalidConfig(
                "concurrent workers exceed 4096".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}
fn tokio_capacity_limit() -> usize {
    usize::MAX >> 3
}
