use modorch::{prelude::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Factory {
    id: &'static str,
    dependency: &'static str,
    created: Arc<AtomicUsize>,
}
impl ModuleFactory for Factory {
    fn descriptor(&self) -> ComponentDescriptor {
        let mut d = ComponentDescriptor::new(ComponentRef::module(self.id));
        d.requires.push(ComponentRef::module(self.dependency));
        d
    }
    fn create(&self, _: &ComponentConfig) -> FrameworkResult<Box<dyn Module>> {
        self.created.fetch_add(1, Ordering::SeqCst);
        panic!("invalid dependency graph must be rejected before create")
    }
}
fn run() {
    let created = Arc::new(AtomicUsize::new(0));
    for scenario in ["missing", "disabled", "cycle"] {
        let mut builder = ApplicationBuilder::new("invalid-graph");
        builder
            .register_module(Factory {
                id: "a",
                dependency: "b",
                created: created.clone(),
            })
            .unwrap();
        if scenario != "missing" {
            builder
                .register_module(Factory {
                    id: "b",
                    dependency: "a",
                    created: created.clone(),
                })
                .unwrap();
        }
        let mut selection = ComponentSelection::new();
        selection.enable_module("a");
        if scenario == "cycle" {
            selection.enable_module("b");
        }
        let error = builder
            .selection(selection)
            .build()
            .err()
            .expect("graph must be rejected");
        assert!(matches!(
            (scenario, error),
            ("missing", FrameworkError::MissingDependency { .. })
                | ("disabled", FrameworkError::DisabledDependency { .. })
                | ("cycle", FrameworkError::DependencyCycle(_))
        ));
    }
    assert_eq!(created.load(Ordering::SeqCst), 0);
    println!("dependency: missing/disabled/cycle rejected before create");
}
fn main() {
    run();
}
#[test]
fn invalid_graph_never_creates_a_component() {
    run();
}
