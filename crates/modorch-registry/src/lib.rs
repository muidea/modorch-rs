//! Composition validation, deterministic dependency ordering and leased services.
#![forbid(unsafe_code)]
mod services;
use modorch_core::*;
pub use services::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct ComponentSelection {
    enabled: BTreeMap<ComponentRef, ComponentConfig>,
}
impl ComponentSelection {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn enable_initiator(&mut self, id: impl Into<String>) -> &mut Self {
        self.enable(ComponentRef::initiator(id))
    }
    pub fn enable_module(&mut self, id: impl Into<String>) -> &mut Self {
        self.enable(ComponentRef::module(id))
    }
    pub fn enable(&mut self, component: ComponentRef) -> &mut Self {
        self.enabled
            .entry(component)
            .or_insert(ComponentConfig::Null);
        self
    }
    pub fn configure(&mut self, component: ComponentRef, config: ComponentConfig) -> &mut Self {
        self.enabled.insert(component, config);
        self
    }
    pub fn components(&self) -> impl Iterator<Item = &ComponentRef> {
        self.enabled.keys()
    }
    pub fn config(&self, component: &ComponentRef) -> ComponentConfig {
        self.enabled.get(component).cloned().unwrap_or_default()
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub enum DependencyResolution {
    #[default]
    RequireSelected,
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
