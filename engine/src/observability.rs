//! Observability — structured trace spans + aggregated metrics.
//!
//! Converts the flat TraceEntry/TranscriptEntry vectors from GraphRunner into
//! a hierarchical span tree and computes aggregated metrics.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::core::runner::{ExecutionResult, TraceEntry, TraceStatus};

// ---------------------------------------------------------------------------
// TraceSpan — hierarchical view of execution
// ---------------------------------------------------------------------------

/// A span in the execution trace tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    pub id: String,
    pub parent_id: Option<String>,
    pub node_id: String,
    pub operation: String,
    pub duration_ms: u64,
    pub status: TraceStatus,
    pub error: Option<String>,
    #[serde(default)]
    pub attributes: HashMap<String, Value>,
    #[serde(default)]
    pub children: Vec<TraceSpan>,
}

/// Build a trace span tree from an ExecutionResult.
///
/// The root span represents the full graph execution. Each traced node
/// becomes a child span.
pub fn build_trace_tree(result: &ExecutionResult, graph_name: &str) -> TraceSpan {
    let total_ms: u64 = result.trace.iter().map(|t| t.duration_ms).sum();
    let root_status = match result.status {
        crate::core::runner::ExecutionStatus::Completed => TraceStatus::Ok,
        _ => TraceStatus::Error,
    };

    let children: Vec<TraceSpan> = result
        .trace
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let mut attrs = HashMap::new();
            attrs.insert("tool_type".into(), Value::String(entry.tool_type.clone()));
            if entry.retries > 0 {
                attrs.insert("retries".into(), Value::Number(entry.retries.into()));
            }

            TraceSpan {
                id: format!("span-{}", i),
                parent_id: Some("root".into()),
                node_id: entry.node_id.clone(),
                operation: format!("node.{}", entry.node_id),
                duration_ms: entry.duration_ms,
                status: entry.status.clone(),
                error: entry.error.clone(),
                attributes: attrs,
                children: vec![],
            }
        })
        .collect();

    TraceSpan {
        id: "root".into(),
        parent_id: None,
        node_id: "graph".into(),
        operation: format!("graph.run({})", graph_name),
        duration_ms: total_ms,
        status: root_status,
        error: result.error.clone(),
        attributes: HashMap::new(),
        children,
    }
}

/// Render a trace tree as a human-readable ASCII tree (for CLI --trace).
pub fn render_trace_tree(span: &TraceSpan) -> String {
    let mut lines = Vec::new();
    render_span(&mut lines, span, "", true);
    lines.join("\n")
}

fn render_span(lines: &mut Vec<String>, span: &TraceSpan, prefix: &str, is_last: bool) {
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let status_icon = match span.status {
        TraceStatus::Ok => "✓",
        TraceStatus::Error => "✗",
        TraceStatus::Skipped => "⊘",
    };

    lines.push(format!(
        "{}{}{} {} ({}ms)",
        prefix, connector, status_icon, span.operation, span.duration_ms
    ));

    let child_prefix = if prefix.is_empty() {
        "".to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    for (i, child) in span.children.iter().enumerate() {
        let last = i == span.children.len() - 1;
        render_span(lines, child, &child_prefix, last);
    }
}

// ---------------------------------------------------------------------------
// Metrics — aggregated stats
// ---------------------------------------------------------------------------

/// Aggregated execution metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub total_nodes: usize,
    pub successful_nodes: usize,
    pub failed_nodes: usize,
    pub skipped_nodes: usize,
    pub total_duration_ms: u64,
    pub avg_node_duration_ms: f64,
    pub max_node_duration_ms: u64,
    pub min_node_duration_ms: u64,
    pub total_retries: u32,
    pub nodes_by_tool_type: HashMap<String, usize>,
    pub duration_by_tool_type: HashMap<String, u64>,
}

/// Compute metrics from a trace.
pub fn compute_metrics(trace: &[TraceEntry]) -> ExecutionMetrics {
    if trace.is_empty() {
        return ExecutionMetrics {
            total_nodes: 0,
            successful_nodes: 0,
            failed_nodes: 0,
            skipped_nodes: 0,
            total_duration_ms: 0,
            avg_node_duration_ms: 0.0,
            max_node_duration_ms: 0,
            min_node_duration_ms: 0,
            total_retries: 0,
            nodes_by_tool_type: HashMap::new(),
            duration_by_tool_type: HashMap::new(),
        };
    }

    let total_nodes = trace.len();
    let successful = trace.iter().filter(|t| t.status == TraceStatus::Ok).count();
    let failed = trace.iter().filter(|t| t.status == TraceStatus::Error).count();
    let skipped = trace.iter().filter(|t| t.status == TraceStatus::Skipped).count();
    let total_ms: u64 = trace.iter().map(|t| t.duration_ms).sum();
    let max_ms = trace.iter().map(|t| t.duration_ms).max().unwrap_or(0);
    let min_ms = trace.iter().map(|t| t.duration_ms).min().unwrap_or(0);
    let total_retries: u32 = trace.iter().map(|t| t.retries).sum();

    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut dur_by_type: HashMap<String, u64> = HashMap::new();
    for entry in trace {
        *by_type.entry(entry.tool_type.clone()).or_default() += 1;
        *dur_by_type.entry(entry.tool_type.clone()).or_default() += entry.duration_ms;
    }

    ExecutionMetrics {
        total_nodes,
        successful_nodes: successful,
        failed_nodes: failed,
        skipped_nodes: skipped,
        total_duration_ms: total_ms,
        avg_node_duration_ms: total_ms as f64 / total_nodes as f64,
        max_node_duration_ms: max_ms,
        min_node_duration_ms: min_ms,
        total_retries,
        nodes_by_tool_type: by_type,
        duration_by_tool_type: dur_by_type,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runner::{ExecutionStatus, TraceStatus};
    use crate::core::state::SharedState;

    fn make_trace(id: &str, tool: &str, ms: u64, status: TraceStatus) -> TraceEntry {
        TraceEntry {
            node_id: id.into(),
            tool_type: tool.into(),
            status,
            duration_ms: ms,
            retries: 0,
            error: None,
        }
    }

    #[test]
    fn build_trace_tree_creates_hierarchy() {
        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state: SharedState::new(),
            trace: vec![
                make_trace("trigger", "trigger/manual", 1, TraceStatus::Ok),
                make_trace("llm", "ai/llm_call", 1500, TraceStatus::Ok),
                make_trace("out", "output/response", 2, TraceStatus::Ok),
            ],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };

        let tree = build_trace_tree(&result, "test-agent");
        assert_eq!(tree.id, "root");
        assert_eq!(tree.children.len(), 3);
        assert_eq!(tree.duration_ms, 1503);
        assert_eq!(tree.children[1].node_id, "llm");
        assert_eq!(tree.children[1].duration_ms, 1500);
    }

    #[test]
    fn render_trace_tree_output() {
        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state: SharedState::new(),
            trace: vec![
                make_trace("a", "trigger/manual", 5, TraceStatus::Ok),
                make_trace("b", "ai/llm_call", 2000, TraceStatus::Ok),
                make_trace("c", "output/response", 3, TraceStatus::Ok),
            ],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };

        let tree = build_trace_tree(&result, "my-agent");
        let rendered = render_trace_tree(&tree);
        assert!(rendered.contains("graph.run(my-agent)"));
        assert!(rendered.contains("node.a"));
        assert!(rendered.contains("node.b"));
        assert!(rendered.contains("2000ms"));
    }

    #[test]
    fn compute_metrics_basic() {
        let trace = vec![
            make_trace("a", "trigger/manual", 5, TraceStatus::Ok),
            make_trace("b", "ai/llm_call", 2000, TraceStatus::Ok),
            make_trace("c", "ai/llm_call", 1500, TraceStatus::Ok),
            make_trace("d", "output/response", 3, TraceStatus::Error),
        ];

        let metrics = compute_metrics(&trace);
        assert_eq!(metrics.total_nodes, 4);
        assert_eq!(metrics.successful_nodes, 3);
        assert_eq!(metrics.failed_nodes, 1);
        assert_eq!(metrics.total_duration_ms, 3508);
        assert_eq!(metrics.max_node_duration_ms, 2000);
        assert_eq!(metrics.min_node_duration_ms, 3);
        assert_eq!(metrics.nodes_by_tool_type["ai/llm_call"], 2);
        assert_eq!(metrics.duration_by_tool_type["ai/llm_call"], 3500);
    }

    #[test]
    fn compute_metrics_empty() {
        let metrics = compute_metrics(&[]);
        assert_eq!(metrics.total_nodes, 0);
        assert_eq!(metrics.avg_node_duration_ms, 0.0);
    }
}
