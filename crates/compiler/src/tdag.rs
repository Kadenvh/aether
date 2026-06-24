//! Temporal DAG construction and validation (R2, KTD9).
//!
//! Turns the `serde`-level [`TDag`] into an executable graph: every node id
//! must be unique, every edge must reference real nodes, and the whole graph
//! must be acyclic. Construction fails if any of these hold, so an
//! [`ExecutionGraph`] only ever exists for a valid, topologically-orderable
//! t-DAG. `toposort` is deterministic for a graph built the same way (node
//! indices follow insertion order), giving a stable execution order.

use std::collections::HashMap;

use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};

use aether_sdk::types::{EdgeKind, NodeId, TDag};
use aether_sdk::{AetherError, Result};

/// A validated, acyclic execution graph with a precomputed topological order.
pub struct ExecutionGraph {
    graph: DiGraph<NodeId, EdgeKind>,
    order: Vec<NodeId>,
}

impl ExecutionGraph {
    /// Build and validate an execution graph from a synthesized t-DAG.
    pub fn build(tdag: &TDag) -> Result<Self> {
        let mut graph = DiGraph::<NodeId, EdgeKind>::new();
        let mut index: HashMap<&str, NodeIndex> = HashMap::new();

        for node in &tdag.nodes {
            if index.contains_key(node.id.as_str()) {
                return Err(AetherError::IntentInvalid(format!(
                    "duplicate t-DAG node id '{}'",
                    node.id
                )));
            }
            let idx = graph.add_node(node.id.clone());
            index.insert(node.id.as_str(), idx);
        }

        for edge in &tdag.edges {
            let from = *index.get(edge.from.as_str()).ok_or_else(|| {
                AetherError::IntentInvalid(format!("edge references unknown node '{}'", edge.from))
            })?;
            let to = *index.get(edge.to.as_str()).ok_or_else(|| {
                AetherError::IntentInvalid(format!("edge references unknown node '{}'", edge.to))
            })?;
            graph.add_edge(from, to, edge.kind);
        }

        let sorted = toposort(&graph, None).map_err(|cycle| {
            let id = graph[cycle.node_id()].clone();
            AetherError::IntentInvalid(format!("t-DAG contains a cycle involving node '{id}'"))
        })?;
        let order = sorted.into_iter().map(|idx| graph[idx].clone()).collect();

        Ok(ExecutionGraph { graph, order })
    }

    /// Node ids in dependency-respecting execution order.
    pub fn topo_order(&self) -> &[NodeId] {
        &self.order
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_sdk::types::{NodeKind, TDagEdge, TDagNode};

    fn node(id: &str, kind: NodeKind) -> TDagNode {
        TDagNode {
            id: id.into(),
            kind,
            spec: serde_json::Value::Null,
        }
    }

    fn edge(from: &str, to: &str) -> TDagEdge {
        TDagEdge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::DataFlow,
        }
    }

    fn diamond() -> TDag {
        // a -> b, a -> c, b -> d, c -> d
        TDag {
            nodes: vec![
                node("a", NodeKind::Ingest),
                node("b", NodeKind::Transform),
                node("c", NodeKind::Transform),
                node("d", NodeKind::Persist),
            ],
            edges: vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "d"),
                edge("c", "d"),
            ],
        }
    }

    fn position(order: &[NodeId], id: &str) -> usize {
        order.iter().position(|n| n == id).unwrap()
    }

    #[test]
    fn orders_dependencies_before_dependents() {
        let g = ExecutionGraph::build(&diamond()).unwrap();
        let order = g.topo_order();
        assert_eq!(g.node_count(), 4);
        assert_eq!(order.first().unwrap(), "a");
        assert_eq!(order.last().unwrap(), "d");
        assert!(position(order, "a") < position(order, "b"));
        assert!(position(order, "b") < position(order, "d"));
        assert!(position(order, "c") < position(order, "d"));
    }

    #[test]
    fn rejects_cycle_at_construction() {
        let tdag = TDag {
            nodes: vec![node("a", NodeKind::Ingest), node("b", NodeKind::Transform)],
            edges: vec![edge("a", "b"), edge("b", "a")],
        };
        assert!(matches!(
            ExecutionGraph::build(&tdag),
            Err(AetherError::IntentInvalid(_))
        ));
    }

    #[test]
    fn topo_order_is_stable_across_builds() {
        let a = ExecutionGraph::build(&diamond()).unwrap();
        let b = ExecutionGraph::build(&diamond()).unwrap();
        assert_eq!(a.topo_order(), b.topo_order());
    }

    #[test]
    fn rejects_edge_to_unknown_node() {
        let tdag = TDag {
            nodes: vec![node("a", NodeKind::Ingest)],
            edges: vec![edge("a", "ghost")],
        };
        assert!(matches!(
            ExecutionGraph::build(&tdag),
            Err(AetherError::IntentInvalid(_))
        ));
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let tdag = TDag {
            nodes: vec![node("a", NodeKind::Ingest), node("a", NodeKind::Transform)],
            edges: vec![],
        };
        assert!(matches!(
            ExecutionGraph::build(&tdag),
            Err(AetherError::IntentInvalid(_))
        ));
    }
}
