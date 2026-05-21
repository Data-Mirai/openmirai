use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("duplicate node id: {0}")]
    DuplicateNodeId(String),

    #[error("duplicate edge id: {0}")]
    DuplicateEdgeId(String),

    #[error("edge `{edge_id}` references unknown source node `{node_id}`")]
    UnknownSourceNode { edge_id: String, node_id: String },

    #[error("edge `{edge_id}` references unknown target node `{node_id}`")]
    UnknownTargetNode { edge_id: String, node_id: String },

    #[error("graph has no nodes")]
    EmptyGraph,

    #[error("self-loop detected on node `{0}`")]
    SelfLoop(String),
}

// ---------------------------------------------------------------------------
// EdgeCondition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ComparisonOp {
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
    In,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeCondition {
    pub field: String,
    pub op: ComparisonOp,
    pub value: serde_json::Value,
}

// ---------------------------------------------------------------------------
// EdgeDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDef {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<EdgeCondition>,
    /// Maps output fields from the source node to input params of the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_map: Option<HashMap<String, String>>,
}

// ---------------------------------------------------------------------------
// NodeDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: String,
    /// Block type identifier, e.g. "ai/llm_call", "logic/condition".
    pub tool_type: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    /// Optional (x, y) position for the visual editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f64, f64)>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

// ---------------------------------------------------------------------------
// GraphDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDef {
    pub id: String,
    pub name: String,
    pub version: String,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl GraphDef {
    /// Validate structural integrity of the graph.
    ///
    /// Checks:
    /// - Graph is non-empty
    /// - No duplicate node IDs
    /// - No duplicate edge IDs
    /// - All edge sources/targets reference existing nodes
    /// - No self-loops
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.nodes.is_empty() {
            return Err(GraphError::EmptyGraph);
        }

        // Check duplicate node IDs
        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if !node_ids.insert(&node.id) {
                return Err(GraphError::DuplicateNodeId(node.id.clone()));
            }
        }

        // Check duplicate edge IDs and referential integrity
        let mut edge_ids = HashSet::with_capacity(self.edges.len());
        for edge in &self.edges {
            if !edge_ids.insert(&edge.id) {
                return Err(GraphError::DuplicateEdgeId(edge.id.clone()));
            }
            if !node_ids.contains(&edge.source) {
                return Err(GraphError::UnknownSourceNode {
                    edge_id: edge.id.clone(),
                    node_id: edge.source.clone(),
                });
            }
            if !node_ids.contains(&edge.target) {
                return Err(GraphError::UnknownTargetNode {
                    edge_id: edge.id.clone(),
                    node_id: edge.target.clone(),
                });
            }
            if edge.source == edge.target {
                return Err(GraphError::SelfLoop(edge.source.clone()));
            }
        }

        Ok(())
    }

    /// Return nodes that have no incoming edges (graph entry points).
    pub fn entry_nodes(&self) -> Vec<&NodeDef> {
        let targets: HashSet<&str> = self.edges.iter().map(|e| e.target.as_str()).collect();
        self.nodes
            .iter()
            .filter(|n| !targets.contains(n.id.as_str()))
            .collect()
    }

    /// Return all edges whose source matches `node_id`.
    pub fn outgoing_edges(&self, node_id: &str) -> Vec<&EdgeDef> {
        self.edges
            .iter()
            .filter(|e| e.source == node_id)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, tool_type: &str) -> NodeDef {
        NodeDef {
            id: id.to_string(),
            tool_type: tool_type.to_string(),
            version: "1.0.0".to_string(),
            config: HashMap::new(),
            position: None,
        }
    }

    fn make_edge(id: &str, source: &str, target: &str) -> EdgeDef {
        EdgeDef {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            condition: None,
            data_map: None,
        }
    }

    fn sample_graph() -> GraphDef {
        GraphDef {
            id: "g1".into(),
            name: "test".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "ai/llm_call"),
                make_node("b", "logic/condition"),
                make_node("c", "ai/llm_call"),
            ],
            edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn validate_ok() {
        assert!(sample_graph().validate().is_ok());
    }

    #[test]
    fn validate_empty() {
        let g = GraphDef {
            id: "g".into(),
            name: "empty".into(),
            version: "1.0.0".into(),
            nodes: vec![],
            edges: vec![],
            metadata: HashMap::new(),
        };
        assert!(matches!(g.validate(), Err(GraphError::EmptyGraph)));
    }

    #[test]
    fn validate_duplicate_node() {
        let mut g = sample_graph();
        g.nodes.push(make_node("a", "ai/llm_call"));
        assert!(matches!(g.validate(), Err(GraphError::DuplicateNodeId(_))));
    }

    #[test]
    fn validate_duplicate_edge() {
        let mut g = sample_graph();
        g.edges.push(make_edge("e1", "a", "c"));
        assert!(matches!(g.validate(), Err(GraphError::DuplicateEdgeId(_))));
    }

    #[test]
    fn validate_unknown_source() {
        let mut g = sample_graph();
        g.edges.push(make_edge("e3", "ghost", "a"));
        assert!(matches!(
            g.validate(),
            Err(GraphError::UnknownSourceNode { .. })
        ));
    }

    #[test]
    fn validate_unknown_target() {
        let mut g = sample_graph();
        g.edges.push(make_edge("e3", "a", "ghost"));
        assert!(matches!(
            g.validate(),
            Err(GraphError::UnknownTargetNode { .. })
        ));
    }

    #[test]
    fn validate_self_loop() {
        let mut g = sample_graph();
        g.edges.push(make_edge("e3", "a", "a"));
        assert!(matches!(g.validate(), Err(GraphError::SelfLoop(_))));
    }

    #[test]
    fn entry_nodes_linear() {
        let g = sample_graph();
        let entries = g.entry_nodes();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "a");
    }

    #[test]
    fn entry_nodes_multiple() {
        let g = GraphDef {
            id: "g".into(),
            name: "multi".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("x", "ai/llm_call"),
                make_node("y", "ai/llm_call"),
                make_node("z", "ai/llm_call"),
            ],
            edges: vec![make_edge("e1", "x", "z"), make_edge("e2", "y", "z")],
            metadata: HashMap::new(),
        };
        let mut ids: Vec<&str> = g.entry_nodes().iter().map(|n| n.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["x", "y"]);
    }

    #[test]
    fn outgoing_edges_filters_correctly() {
        let g = sample_graph();
        let from_a = g.outgoing_edges("a");
        assert_eq!(from_a.len(), 1);
        assert_eq!(from_a[0].target, "b");

        let from_c = g.outgoing_edges("c");
        assert!(from_c.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let g = sample_graph();
        let json = serde_json::to_string(&g).unwrap();
        let g2: GraphDef = serde_json::from_str(&json).unwrap();
        assert_eq!(g2.id, g.id);
        assert_eq!(g2.nodes.len(), g.nodes.len());
        assert_eq!(g2.edges.len(), g.edges.len());
    }
}
