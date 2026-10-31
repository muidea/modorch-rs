use modorch_core::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Node(pub ComponentRef, pub usize);
tokio::task_local! { pub(crate) static EXECUTION: Node; }
type Edges = HashMap<(Node, Node), usize>;
#[derive(Clone, Default)]
pub(crate) struct WaitGraph(Arc<Mutex<Edges>>);
pub(crate) struct WaitEdge {
    graph: WaitGraph,
    from: Node,
    to: Node,
}
impl WaitGraph {
    pub fn insert(&self, from: Node, to: Node) -> FrameworkResult<WaitEdge> {
        let mut edges = self.0.lock().unwrap();
        let mut pending = vec![to.clone()];
        let mut visited = HashSet::new();
        while let Some(node) = pending.pop() {
            if node == from {
                return Err(FrameworkError::CircularWait(vec![from.0, to.0]));
            }
            if visited.insert(node.clone()) {
                for ((a, b), _) in edges.iter() {
                    if a == &node {
                        pending.push(b.clone());
                    }
                }
            }
        }
        *edges.entry((from.clone(), to.clone())).or_default() += 1;
        Ok(WaitEdge {
            graph: self.clone(),
            from,
            to,
        })
    }
}
impl Drop for WaitEdge {
    fn drop(&mut self) {
        let mut edges = self.graph.0.lock().unwrap();
        let key = (self.from.clone(), self.to.clone());
        if let Some(count) = edges.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                edges.remove(&key);
            }
        }
    }
}
