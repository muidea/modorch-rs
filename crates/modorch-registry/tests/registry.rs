use modorch_core::*;
use modorch_registry::*;
use std::{collections::BTreeMap, sync::Arc};
fn descriptors(items: Vec<ComponentDescriptor>) -> BTreeMap<ComponentRef, ComponentDescriptor> {
    items.into_iter().map(|d| (d.component(), d)).collect()
}
#[test]
fn validates_dependencies_and_orders_optional_selected_dependencies() {
    let db = ComponentDescriptor::new(ComponentRef::initiator("db"));
    let mut a = ComponentDescriptor::new(ComponentRef::module("a"));
    a.requires.push(db.component());
    let mut b = ComponentDescriptor::new(ComponentRef::module("b"));
    b.optional.push(a.component());
    let ds = descriptors(vec![db.clone(), a.clone(), b.clone()]);
    let mut selection = ComponentSelection::new();
    selection.enable_module("a");
    assert!(matches!(
        resolve(&ds, &selection, DependencyResolution::RequireSelected),
        Err(FrameworkError::DisabledDependency { .. })
    ));
    assert_eq!(
        resolve(&ds, &selection, DependencyResolution::IncludeRequired).unwrap(),
        vec![db.component(), a.component()]
    );
    selection.enable_initiator("db").enable_module("b");
    assert_eq!(
        resolve(&ds, &selection, DependencyResolution::RequireSelected).unwrap(),
        vec![db.component(), a.component(), b.component()]
    );
}
#[test]
fn rejects_cycles_cross_layer_dependencies_and_invalid_capacities() {
    let mut a = ComponentDescriptor::new(ComponentRef::module("a"));
    a.requires.push(a.component());
    let mut s = ComponentSelection::new();
    s.enable_module("a");
    assert!(matches!(
        resolve(
            &descriptors(vec![a]),
            &s,
            DependencyResolution::RequireSelected
        ),
        Err(FrameworkError::DependencyCycle(_))
    ));
    let mut d = ComponentDescriptor::new(ComponentRef::initiator("db"));
    d.requires.push(ComponentRef::module("a"));
    s.enable_initiator("db");
    assert!(matches!(
        resolve(
            &descriptors(vec![d.clone()]),
            &s,
            DependencyResolution::IncludeRequired
        ),
        Err(FrameworkError::ComponentUnavailable(_))
    ));
    d.queue_capacity = 0;
    assert!(validate(&d).is_err());
    let mut selection = ComponentSelection::new();
    selection.enable_initiator("db");
    d.queue_capacity = 1;
    assert!(matches!(
        resolve(
            &descriptors(vec![d]),
            &selection,
            DependencyResolution::RequireSelected
        ),
        Err(FrameworkError::InvalidConfig(_))
    ));
}
#[tokio::test]
async fn services_enforce_declared_dependencies_isolation_and_leases() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let registry = ServiceRegistry::new(tracker.clone());
    let owner = ComponentRef::initiator("db");
    registry
        .publisher(owner.clone())
        .unwrap()
        .publish("count", Arc::new(42usize))
        .unwrap();
    assert!(registry.publisher(ComponentRef::module("bad")).is_err());
    let denied = registry.reader(ComponentRef::module("bad"), vec![]);
    assert!(matches!(
        denied.get::<usize>(&owner, "count"),
        Err(FrameworkError::AccessDenied(_))
    ));
    let reader = registry.reader(ComponentRef::module("reader"), vec![owner.clone()]);
    let handle = reader.get::<usize>(&owner, "count").unwrap();
    let lease = handle.acquire().unwrap();
    assert_eq!(*lease, 42);
    assert_eq!(tracker.pending().operations, 2);
    tracker.close();
    assert!(matches!(
        handle.acquire(),
        Err(FrameworkError::ServiceUnavailable(_))
    ));
    drop(lease);
    tracker.drain().await;
    registry.retire(&owner);
    assert!(handle.acquire().is_err());
    let other = ServiceRegistry::new(OperationTracker::new());
    assert!(
        other
            .reader(owner.clone(), vec![])
            .get::<usize>(&owner, "count")
            .is_err()
    );
}

#[test]
fn missing_dependencies_are_reported_before_instantiation() {
    let mut a = ComponentDescriptor::new(ComponentRef::module("a"));
    a.requires.push(ComponentRef::initiator("missing"));
    let mut selection = ComponentSelection::new();
    selection.enable_module("a");
    assert!(matches!(
        resolve(
            &descriptors(vec![a]),
            &selection,
            DependencyResolution::RequireSelected
        ),
        Err(FrameworkError::MissingDependency { .. })
    ));
}

#[tokio::test]
async fn borrowed_resources_remain_host_owned_but_handles_are_generation_scoped() {
    let tracker = OperationTracker::new();
    tracker.open().unwrap();
    let registry = ServiceRegistry::new(tracker.clone());
    let owner = ComponentRef::initiator("host");
    let reader = ComponentRef::module("reader");
    let host = Arc::new(42usize);
    registry
        .publisher(owner.clone())
        .unwrap()
        .publish_borrowed("value", host.clone())
        .unwrap();
    let handle = registry
        .reader(reader.clone(), [owner.clone()])
        .get::<usize>(&owner, "value")
        .unwrap();
    assert_eq!(handle.ownership(), ResourceOwnership::Borrowed);
    let lease = handle.acquire().unwrap();
    assert_eq!(*lease, 42);
    let owners = std::collections::BTreeSet::from([owner.clone(), reader.clone()]);
    let cleanup = tracker.close_owners(&owners);
    drop(cleanup);
    assert!(tracker.advance_owners(&owners).is_err());
    drop(lease);
    tracker.drain_owners(&owners).await;
    registry.retire(&owner);
    registry.remove(&owner);
    tracker.advance_owners(&owners).unwrap();
    registry
        .publisher(owner.clone())
        .unwrap()
        .publish_borrowed("value", host.clone())
        .unwrap();
    tracker.open_owners(&owners).unwrap();
    assert!(handle.acquire().is_err());
    let replacement = registry
        .reader(reader, [owner.clone()])
        .get::<usize>(&owner, "value")
        .unwrap();
    assert_eq!(*replacement.acquire().unwrap(), 42);
    assert_eq!(*host, 42);
    tracker.close();
    tracker.drain().await;
}
