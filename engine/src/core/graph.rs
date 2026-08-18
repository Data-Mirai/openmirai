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

    #[error("node `{node_id}` has only conditional outgoing edges: no path can be guaranteed. Add an unconditional edge as the default route.")]
    StrictDeadEnd { node_id: String },

    #[error("fan-out from `{node_id}` into [{}] never converges: work after the branches would never run. Route all branches to a common node.", branches.join(", "))]
    StrictFanOutWithoutJoin {
        node_id: String,
        branches: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// EdgeCondition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
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

/// Accept full words, abbreviations, and PascalCase in YAML/JSON.
///
/// Recommended (readable):
/// ```yaml
/// op: equals              # or: not_equals, greater_than, less_than,
///                         #     greater_or_equal, less_or_equal,
///                         #     in, contains
/// ```
///
/// Also accepted (short form): `eq`, `neq`, `gt`, `lt`, `gte`, `lte`
/// Also accepted (PascalCase): `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte`
impl<'de> serde::Deserialize<'de> for ComparisonOp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "equals" | "equal" | "Eq" | "eq" => Ok(ComparisonOp::Eq),
            "not_equals" | "not_equal" | "Neq" | "neq" => Ok(ComparisonOp::Neq),
            "greater_than" | "Gt" | "gt" => Ok(ComparisonOp::Gt),
            "less_than" | "Lt" | "lt" => Ok(ComparisonOp::Lt),
            "greater_or_equal" | "greater_than_or_equal" | "Gte" | "gte" => Ok(ComparisonOp::Gte),
            "less_or_equal" | "less_than_or_equal" | "Lte" | "lte" => Ok(ComparisonOp::Lte),
            "in" | "In" => Ok(ComparisonOp::In),
            "contains" | "Contains" => Ok(ComparisonOp::Contains),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "equals",
                    "not_equals",
                    "greater_than",
                    "less_than",
                    "greater_or_equal",
                    "less_or_equal",
                    "in",
                    "contains",
                    "eq",
                    "neq",
                    "gt",
                    "lt",
                    "gte",
                    "lte",
                ],
            )),
        }
    }
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
    /// Edge ID — optional. Auto-generated as `{source}__{target}` if empty (FEAT-034 / API-02).
    #[serde(default)]
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<EdgeCondition>,
    /// Maps output fields from the source node to input params of the target.
    /// When `None` and the edge is unconditional, the runner passes through
    /// the entire source output as inputs (FEAT-034 / API-03).
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

pub(crate) fn default_version() -> String {
    "1.0.0".to_string()
}

// ---------------------------------------------------------------------------
// GraphDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphDef {
    pub id: String,
    pub name: String,
    pub version: String,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// When true, the engine may only report `Completed` if the run ended on a
    /// node that produced a value (PRD-022). Opt-in, default `false`: a graph
    /// that does not declare it behaves exactly as before.
    ///
    /// Enforced in two layers — [`GraphDef::validate`] rejects graphs whose
    /// shape guarantees a silent end, and the runner turns a value-less or
    /// error-swallowing termination into `Failed`.
    #[serde(default)]
    pub strict_completion: bool,
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
    ///
    /// Auto-generate IDs for edges that have empty `id` fields (FEAT-034 / API-02).
    /// Format: `{source}__{target}` or `{source}__{target}__{n}` for duplicates.
    pub fn auto_generate_edge_ids(&mut self) {
        let mut pair_counts: HashMap<String, usize> = HashMap::new();
        for edge in &mut self.edges {
            if edge.id.is_empty() {
                let pair_key = format!("{}__{}", edge.source, edge.target);
                let count = pair_counts.entry(pair_key.clone()).or_insert(0);
                *count += 1;
                edge.id = if *count == 1 {
                    pair_key
                } else {
                    format!("{pair_key}__{count}")
                };
            }
        }
    }

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
            if edge.source == edge.target && edge.condition.is_none() {
                return Err(GraphError::SelfLoop(edge.source.clone()));
            }
        }

        if self.strict_completion {
            self.validate_strict()?;
        }

        Ok(())
    }

    /// Shape rules that only apply under `strict_completion` (PRD-022, layer 1).
    ///
    /// Only what is provable without executing:
    ///
    /// - **S1** — a node with outgoing edges must have at least one
    ///   unconditional edge, or a run reaching it with no condition matching
    ///   dies with the cursor at `None`.
    /// - **S2** — a fan-out whose branches do work afterwards must converge on
    ///   a common node, or everything hanging off the branches never runs.
    ///
    /// Conditions depend on data, so neither rule evaluates them — that is the
    /// runtime layer's job.
    fn validate_strict(&self) -> Result<(), GraphError> {
        for node in &self.nodes {
            let outgoing = self.outgoing_edges(&node.id);
            if outgoing.is_empty() {
                continue;
            }

            let branches = self.unconditional_targets(&node.id);

            // S1 — no default route.
            if branches.is_empty() {
                return Err(GraphError::StrictDeadEnd {
                    node_id: node.id.clone(),
                });
            }

            // S2 — fan-out that never converges. Branches with no outgoing
            // edges of their own are terminal: nothing is stranded behind them.
            if branches.len() > 1 {
                let any_branch_continues =
                    branches.iter().any(|b| !self.outgoing_edges(b).is_empty());
                if any_branch_continues && self.common_unconditional_successor(&branches).is_none()
                {
                    return Err(GraphError::StrictFanOutWithoutJoin {
                        node_id: node.id.clone(),
                        branches,
                    });
                }
            }
        }

        Ok(())
    }

    /// Targets a node reaches without a condition in the way — its guaranteed
    /// routes, in edge order.
    fn unconditional_targets(&self, node_id: &str) -> Vec<String> {
        self.outgoing_edges(node_id)
            .iter()
            .filter(|e| e.condition.is_none())
            .map(|e| e.target.clone())
            .collect()
    }

    /// The node every branch of a fan-out leads to unconditionally, if any.
    ///
    /// This is the join the runner resumes the walk on after a fan-out, and the
    /// convergence S2 requires — one definition, two callers.
    pub fn common_unconditional_successor(&self, branch_ids: &[String]) -> Option<String> {
        let others: Vec<HashSet<String>> = branch_ids
            .get(1..)?
            .iter()
            .map(|nid| self.unconditional_targets(nid).into_iter().collect())
            .collect();

        // Walk the first branch's targets in edge order so the answer is stable
        // when more than one node happens to be common.
        self.unconditional_targets(branch_ids.first()?)
            .into_iter()
            .find(|t| others.iter().all(|s| s.contains(t)))
    }

    /// Convenience: auto-generate IDs then validate.
    pub fn prepare(&mut self) -> Result<(), GraphError> {
        self.auto_generate_edge_ids();
        self.validate()
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
        self.edges.iter().filter(|e| e.source == node_id).collect()
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
            strict_completion: false,
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
            strict_completion: false,
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
            strict_completion: false,
        };
        let mut ids: Vec<&str> = g.entry_nodes().iter().map(|n| n.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["x", "y"]);
    }

    #[test]
    fn auto_gen_edge_ids() {
        let mut g = GraphDef {
            id: "g".into(),
            name: "auto".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "x"), make_node("b", "y")],
            edges: vec![EdgeDef {
                id: String::new(), // empty = auto-gen
                source: "a".into(),
                target: "b".into(),
                condition: None,
                data_map: None,
            }],
            metadata: HashMap::new(),
            strict_completion: false,
        };
        g.prepare().unwrap();
        assert_eq!(g.edges[0].id, "a__b");
    }

    #[test]
    fn auto_gen_edge_ids_collision() {
        let mut g = GraphDef {
            id: "g".into(),
            name: "collision".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "x"), make_node("b", "y")],
            edges: vec![
                EdgeDef {
                    id: String::new(),
                    source: "a".into(),
                    target: "b".into(),
                    condition: Some(EdgeCondition {
                        field: "x".into(),
                        op: ComparisonOp::Eq,
                        value: serde_json::json!(true),
                    }),
                    data_map: None,
                },
                EdgeDef {
                    id: String::new(),
                    source: "a".into(),
                    target: "b".into(),
                    condition: Some(EdgeCondition {
                        field: "x".into(),
                        op: ComparisonOp::Eq,
                        value: serde_json::json!(false),
                    }),
                    data_map: None,
                },
            ],
            metadata: HashMap::new(),
            strict_completion: false,
        };
        g.auto_generate_edge_ids();
        let ids: Vec<&str> = g.edges.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"a__b"));
        assert!(ids.contains(&"a__b__2"));
    }

    #[test]
    fn conditional_self_loop_allowed() {
        let g = GraphDef {
            id: "g".into(),
            name: "loop".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "x")],
            edges: vec![EdgeDef {
                id: "e1".into(),
                source: "a".into(),
                target: "a".into(),
                condition: Some(EdgeCondition {
                    field: "done".into(),
                    op: ComparisonOp::Eq,
                    value: serde_json::json!(false),
                }),
                data_map: None,
            }],
            metadata: HashMap::new(),
            strict_completion: false,
        };
        assert!(g.validate().is_ok());
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

    #[test]
    fn comparison_op_full_words() {
        for (input, expected) in [
            ("\"equals\"", ComparisonOp::Eq),
            ("\"equal\"", ComparisonOp::Eq),
            ("\"not_equals\"", ComparisonOp::Neq),
            ("\"not_equal\"", ComparisonOp::Neq),
            ("\"greater_than\"", ComparisonOp::Gt),
            ("\"less_than\"", ComparisonOp::Lt),
            ("\"greater_or_equal\"", ComparisonOp::Gte),
            ("\"greater_than_or_equal\"", ComparisonOp::Gte),
            ("\"less_or_equal\"", ComparisonOp::Lte),
            ("\"less_than_or_equal\"", ComparisonOp::Lte),
            ("\"in\"", ComparisonOp::In),
            ("\"contains\"", ComparisonOp::Contains),
        ] {
            let op: ComparisonOp = serde_json::from_str(input)
                .unwrap_or_else(|e| panic!("failed to parse {input}: {e}"));
            assert_eq!(op, expected, "input: {input}");
        }
    }

    #[test]
    fn comparison_op_abbreviations_still_work() {
        for (input, expected) in [
            ("\"eq\"", ComparisonOp::Eq),
            ("\"neq\"", ComparisonOp::Neq),
            ("\"gt\"", ComparisonOp::Gt),
            ("\"lt\"", ComparisonOp::Lt),
            ("\"gte\"", ComparisonOp::Gte),
            ("\"lte\"", ComparisonOp::Lte),
            ("\"Eq\"", ComparisonOp::Eq),
            ("\"Gt\"", ComparisonOp::Gt),
        ] {
            let op: ComparisonOp = serde_json::from_str(input).unwrap();
            assert_eq!(op, expected);
        }
    }

    #[test]
    fn comparison_op_rejects_invalid() {
        let result: Result<ComparisonOp, _> = serde_json::from_str("\"banana\"");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // strict_completion — layer 1 (PRD-022)
    // -----------------------------------------------------------------------

    fn make_cond_edge(id: &str, source: &str, target: &str, value: bool) -> EdgeDef {
        EdgeDef {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            condition: Some(EdgeCondition {
                field: "ok".into(),
                op: ComparisonOp::Eq,
                value: serde_json::json!(value),
            }),
            data_map: None,
        }
    }

    /// `route` can only leave through conditions — if neither matches, the run
    /// dies where nobody declared an end.
    fn only_conditional_exits() -> GraphDef {
        GraphDef {
            id: "g".into(),
            name: "dead-end".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("route", "logic/switch"),
                make_node("yes", "output/response"),
                make_node("no", "output/response"),
            ],
            edges: vec![
                make_cond_edge("e1", "route", "yes", true),
                make_cond_edge("e2", "route", "no", false),
            ],
            metadata: HashMap::new(),
            strict_completion: true,
        }
    }

    #[test]
    fn strict_s1_rejects_node_without_default_route() {
        let g = only_conditional_exits();
        match g.validate() {
            Err(GraphError::StrictDeadEnd { node_id }) => assert_eq!(node_id, "route"),
            other => panic!("expected StrictDeadEnd, got {other:?}"),
        }
    }

    /// TEST-208 — the same graph without the flag keeps loading, exactly as before.
    #[test]
    fn strict_off_accepts_the_same_graph() {
        let mut g = only_conditional_exits();
        g.strict_completion = false;
        assert!(g.validate().is_ok());
    }

    #[test]
    fn strict_s1_accepts_conditional_exits_with_a_default_edge() {
        let mut g = only_conditional_exits();
        g.nodes.push(make_node("fallback", "output/response"));
        g.edges.push(make_edge("e3", "route", "fallback"));
        assert!(g.validate().is_ok());
    }

    /// Fan-out whose branches keep working but never meet again: everything
    /// hanging off them would silently never run.
    fn fanout_without_join() -> GraphDef {
        GraphDef {
            id: "g".into(),
            name: "fanout".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("split", "logic/merge"),
                make_node("a", "logic/merge"),
                make_node("b", "logic/merge"),
                make_node("a_next", "output/response"),
                make_node("b_next", "output/response"),
            ],
            edges: vec![
                make_edge("e1", "split", "a"),
                make_edge("e2", "split", "b"),
                make_edge("e3", "a", "a_next"),
                make_edge("e4", "b", "b_next"),
            ],
            metadata: HashMap::new(),
            strict_completion: true,
        }
    }

    #[test]
    fn strict_s2_rejects_fanout_that_never_converges() {
        match fanout_without_join().validate() {
            Err(GraphError::StrictFanOutWithoutJoin { node_id, branches }) => {
                assert_eq!(node_id, "split");
                assert_eq!(branches, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected StrictFanOutWithoutJoin, got {other:?}"),
        }
    }

    #[test]
    fn strict_s2_accepts_fanout_with_a_join() {
        let mut g = fanout_without_join();
        g.nodes.retain(|n| n.id != "b_next");
        g.edges.retain(|e| e.id != "e4");
        g.edges.push(make_edge("e4", "b", "a_next"));
        assert!(g.validate().is_ok());
    }

    /// Branches that end where they are strand nothing — no join required.
    #[test]
    fn strict_s2_accepts_fanout_into_terminal_branches() {
        let mut g = fanout_without_join();
        g.nodes.retain(|n| n.id != "a_next" && n.id != "b_next");
        g.edges.retain(|e| e.id == "e1" || e.id == "e2");
        assert!(g.validate().is_ok());
    }

    #[test]
    fn common_successor_is_the_node_all_branches_reach() {
        let mut g = fanout_without_join();
        g.edges.retain(|e| e.id != "e4");
        g.edges.push(make_edge("e4", "b", "a_next"));
        let branches = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            g.common_unconditional_successor(&branches),
            Some("a_next".to_string())
        );
        assert_eq!(
            fanout_without_join().common_unconditional_successor(&branches),
            None
        );
    }

    /// TEST-216 — a graph serialized before the field existed still loads.
    #[test]
    fn strict_completion_defaults_to_false_when_absent() {
        let legacy = r#"{
            "id": "g",
            "name": "legacy",
            "version": "1.0.0",
            "nodes": [{"id": "a", "tool_type": "output/response", "version": "1.0.0", "config": {}}],
            "edges": []
        }"#;
        let g: GraphDef = serde_json::from_str(legacy).unwrap();
        assert!(!g.strict_completion);
        assert!(g.validate().is_ok());
    }

    #[test]
    fn strict_completion_survives_a_roundtrip() {
        let mut g = sample_graph();
        g.strict_completion = true;
        let back: GraphDef = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert!(back.strict_completion);
    }
}
