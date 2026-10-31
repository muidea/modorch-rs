use modorch_core::*;
use std::{
    any::{Any, TypeId},
    collections::{BTreeSet, HashMap},
    ops::Deref,
    sync::{Arc, Mutex},
};

/// Owned resources are closed by their Initiator; Borrowed resources remain
/// host-owned. Both lose application-local availability when their endpoint retires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceOwnership {
    Owned,
    Borrowed,
}
type Key = (ComponentRef, String, TypeId);
struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    available: bool,
    ownership: ResourceOwnership,
    generation: u64,
}
#[derive(Clone)]
pub struct ServiceRegistry {
    entries: Arc<Mutex<HashMap<Key, Entry>>>,
    tracker: OperationTracker,
}
impl ServiceRegistry {
    pub fn new(tracker: OperationTracker) -> Self {
        Self {
            entries: Arc::default(),
            tracker,
        }
    }
    pub fn publisher(&self, owner: ComponentRef) -> FrameworkResult<ServicePublisher> {
        if owner.kind != ComponentKind::Initiator {
            return Err(FrameworkError::AccessDenied(owner));
        }
        Ok(ServicePublisher {
            registry: self.clone(),
            owner,
        })
    }
    pub fn reader(
        &self,
        owner: ComponentRef,
        dependencies: impl IntoIterator<Item = ComponentRef>,
    ) -> ServiceReader {
        ServiceReader {
            borrower: owner.clone(),
            borrower_generation: self.tracker.generation(&owner),
            registry: self.clone(),
            allowed: dependencies
                .into_iter()
                .chain(std::iter::once(owner))
                .collect(),
        }
    }
    pub fn retire(&self, owner: &ComponentRef) {
        let mut entries = self.entries.lock().unwrap();
        for ((component, _, _), entry) in entries.iter_mut() {
            if component == owner {
                entry.available = false;
            }
        }
    }
    pub fn remove(&self, owner: &ComponentRef) {
        self.entries
            .lock()
            .unwrap()
            .retain(|(component, _, _), _| component != owner);
    }
}
pub struct ServicePublisher {
    registry: ServiceRegistry,
    owner: ComponentRef,
}
impl ServicePublisher {
    pub fn publish<T: Send + Sync + 'static>(
        &mut self,
        name: impl Into<String>,
        value: Arc<T>,
    ) -> FrameworkResult<()> {
        self.publish_with_ownership(name, value, ResourceOwnership::Owned)
    }
    pub fn publish_borrowed<T: Send + Sync + 'static>(
        &mut self,
        name: impl Into<String>,
        value: Arc<T>,
    ) -> FrameworkResult<()> {
        self.publish_with_ownership(name, value, ResourceOwnership::Borrowed)
    }
    pub fn publish_with_ownership<T: Send + Sync + 'static>(
        &mut self,
        name: impl Into<String>,
        value: Arc<T>,
        ownership: ResourceOwnership,
    ) -> FrameworkResult<()> {
        let key = (self.owner.clone(), name.into(), TypeId::of::<T>());
        let mut entries = self.registry.entries.lock().unwrap();
        if entries.contains_key(&key) {
            return Err(FrameworkError::InvalidConfig(format!(
                "duplicate service {}",
                key.1
            )));
        }
        entries.insert(
            key,
            Entry {
                value,
                available: true,
                ownership,
                generation: self.registry.tracker.generation(&self.owner),
            },
        );
        Ok(())
    }
}
#[derive(Clone)]
pub struct ServiceReader {
    registry: ServiceRegistry,
    allowed: BTreeSet<ComponentRef>,
    borrower: ComponentRef,
    borrower_generation: u64,
}
impl ServiceReader {
    pub fn get<T: Send + Sync + 'static>(
        &self,
        owner: &ComponentRef,
        name: &str,
    ) -> FrameworkResult<ServiceHandle<T>> {
        if !self
            .registry
            .tracker
            .generation_valid(&self.borrower, self.borrower_generation)
        {
            return Err(FrameworkError::ServiceUnavailable(name.into()));
        }
        if !self.allowed.contains(owner) {
            return Err(FrameworkError::AccessDenied(owner.clone()));
        }
        let key = (owner.clone(), name.to_owned(), TypeId::of::<T>());
        let entries = self.registry.entries.lock().unwrap();
        if !entries.get(&key).is_some_and(|e| e.available) {
            return Err(FrameworkError::ServiceUnavailable(name.into()));
        }
        Ok(ServiceHandle {
            registry: self.registry.clone(),
            borrower: self.borrower.clone(),
            borrower_generation: self.borrower_generation,
            generation: entries[&key].generation,
            ownership: entries[&key].ownership,
            key,
            marker: std::marker::PhantomData,
        })
    }
}
pub struct ServiceHandle<T> {
    registry: ServiceRegistry,
    key: Key,
    marker: std::marker::PhantomData<T>,
    generation: u64,
    ownership: ResourceOwnership,
    borrower: ComponentRef,
    borrower_generation: u64,
}
impl<T> Clone for ServiceHandle<T> {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            key: self.key.clone(),
            borrower: self.borrower.clone(),
            borrower_generation: self.borrower_generation,
            generation: self.generation,
            ownership: self.ownership,
            marker: std::marker::PhantomData,
        }
    }
}
impl<T: Send + Sync + 'static> ServiceHandle<T> {
    pub fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }
    pub fn acquire(&self) -> FrameworkResult<ServiceLease<T>> {
        let entries = self.registry.entries.lock().unwrap();
        let entry = entries
            .get(&self.key)
            .filter(|e| e.available && e.generation == self.generation)
            .ok_or_else(|| FrameworkError::ServiceUnavailable(self.key.1.clone()))?;
        let borrower = self
            .registry
            .tracker
            .admit_bound(self.borrower.clone(), self.borrower_generation)
            .map_err(|_| FrameworkError::ServiceUnavailable(self.key.1.clone()))?;
        let lease = self
            .registry
            .tracker
            .admit_background(self.key.0.clone())
            .map_err(|_| FrameworkError::ServiceUnavailable(self.key.1.clone()))?;
        let value = entry
            .value
            .clone()
            .downcast::<T>()
            .map_err(|_| FrameworkError::TypeMismatch(self.key.1.clone()))?;
        Ok(ServiceLease {
            value,
            _operation: lease,
            _borrower: borrower,
        })
    }
}
/// Deref exposes the narrow service interface, never an untracked Arc.
pub struct ServiceLease<T> {
    value: Arc<T>,
    _operation: OperationLease,
    _borrower: OperationLease,
}
impl<T> Deref for ServiceLease<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
