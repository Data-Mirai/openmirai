//! SSE Streaming — Server-Sent Events for real-time execution updates.
//!
//! Provides types and helpers for streaming graph execution events to clients
//! via SSE (text/event-stream). Used by the HTTP server's /stream endpoint.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// SSE event types emitted during graph execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum StreamEvent {
    /// Graph execution started.
    #[serde(rename = "graph.started")]
    GraphStarted {
        graph_name: String,
        node_count: usize,
    },

    /// A node started executing.
    #[serde(rename = "node.started")]
    NodeStarted { node_id: String, tool_type: String },

    /// A token from LLM streaming (partial text).
    #[serde(rename = "node.token")]
    NodeToken { node_id: String, token: String },

    /// A node completed successfully.
    #[serde(rename = "node.completed")]
    NodeCompleted {
        node_id: String,
        tool_type: String,
        duration_ms: u64,
        output_keys: Vec<String>,
        /// The node's output map (field → value), so clients can show real results.
        output: serde_json::Value,
    },

    /// A node failed.
    #[serde(rename = "node.error")]
    NodeError {
        node_id: String,
        tool_type: String,
        error: String,
    },

    /// Fan-out started (parallel execution).
    #[serde(rename = "fanout.started")]
    FanoutStarted {
        source_node: String,
        parallel_nodes: Vec<String>,
    },

    /// Fan-out completed.
    #[serde(rename = "fanout.completed")]
    FanoutCompleted { succeeded: usize, failed: usize },

    /// Graph execution completed.
    #[serde(rename = "graph.completed")]
    GraphCompleted {
        status: String,
        total_duration_ms: u64,
        nodes_executed: usize,
        /// Final state snapshot (node_id → output map), so clients can show the result.
        output: serde_json::Value,
    },

    /// Graph execution failed.
    #[serde(rename = "graph.error")]
    GraphError { error: String },
}

impl StreamEvent {
    /// Format as SSE text line: `event: <type>\ndata: <json>\n\n`
    pub fn to_sse(&self) -> String {
        // The event name mirrors each variant's #[serde(rename)]; the data is the
        // variant serialized as JSON. Only the name varies per arm.
        let event_name = match self {
            Self::GraphStarted { .. } => "graph.started",
            Self::NodeStarted { .. } => "node.started",
            Self::NodeToken { .. } => "node.token",
            Self::NodeCompleted { .. } => "node.completed",
            Self::NodeError { .. } => "node.error",
            Self::FanoutStarted { .. } => "fanout.started",
            Self::FanoutCompleted { .. } => "fanout.completed",
            Self::GraphCompleted { .. } => "graph.completed",
            Self::GraphError { .. } => "graph.error",
        };
        let data = serde_json::to_string(self).unwrap_or_default();
        format!("event: {event_name}\ndata: {data}\n\n")
    }
}

/// Convert an ExecutionResult's trace into a stream of events (for replay).
pub fn trace_to_events(
    result: &crate::core::runner::ExecutionResult,
    graph_name: &str,
    node_count: usize,
) -> Vec<StreamEvent> {
    let mut events = vec![StreamEvent::GraphStarted {
        graph_name: graph_name.into(),
        node_count,
    }];

    // Real outputs come from the final state snapshot (node_id → field → value).
    let snapshot = result.state.snapshot();

    for entry in &result.trace {
        events.push(StreamEvent::NodeStarted {
            node_id: entry.node_id.clone(),
            tool_type: entry.tool_type.clone(),
        });

        match entry.status {
            crate::core::runner::TraceStatus::Ok => {
                let node_out = snapshot.get(&entry.node_id);
                events.push(StreamEvent::NodeCompleted {
                    node_id: entry.node_id.clone(),
                    tool_type: entry.tool_type.clone(),
                    duration_ms: entry.duration_ms,
                    output_keys: node_out
                        .map(|m| m.keys().cloned().collect())
                        .unwrap_or_default(),
                    output: node_out
                        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                        .unwrap_or(serde_json::Value::Null),
                });
            }
            crate::core::runner::TraceStatus::Error => {
                events.push(StreamEvent::NodeError {
                    node_id: entry.node_id.clone(),
                    tool_type: entry.tool_type.clone(),
                    error: entry.error.clone().unwrap_or_default(),
                });
            }
            crate::core::runner::TraceStatus::Skipped => {}
        }
    }

    let total_ms: u64 = result.trace.iter().map(|t| t.duration_ms).sum();
    let status = format!("{:?}", result.status);
    events.push(StreamEvent::GraphCompleted {
        status,
        total_duration_ms: total_ms,
        nodes_executed: result.trace.len(),
        output: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    });

    events
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_event_to_sse_format() {
        let event = StreamEvent::GraphStarted {
            graph_name: "test".into(),
            node_count: 3,
        };
        let sse = event.to_sse();
        assert!(sse.starts_with("event: graph.started\n"));
        assert!(sse.contains("data: "));
        assert!(sse.ends_with("\n\n"));
    }

    #[test]
    fn node_completed_sse() {
        let event = StreamEvent::NodeCompleted {
            node_id: "llm".into(),
            tool_type: "ai/llm_call".into(),
            duration_ms: 1500,
            output_keys: vec!["response".into()],
            output: serde_json::json!({ "response": "hi" }),
        };
        let sse = event.to_sse();
        assert!(sse.contains("node.completed"));
        assert!(sse.contains("1500"));
        assert!(sse.contains("response"));
    }

    #[test]
    fn trace_to_events_basic() {
        use crate::core::runner::*;
        use crate::core::state::SharedState;

        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state: SharedState::new(),
            trace: vec![
                TraceEntry {
                    node_id: "a".into(),
                    tool_type: "trigger/manual".into(),
                    status: TraceStatus::Ok,
                    duration_ms: 1,
                    retries: 0,
                    started_at: 0.0,
                    finished_at: 0.0,
                    error: None,
                },
                TraceEntry {
                    node_id: "b".into(),
                    tool_type: "ai/llm_call".into(),
                    status: TraceStatus::Ok,
                    duration_ms: 2000,
                    retries: 0,
                    started_at: 0.0,
                    finished_at: 0.0,
                    error: None,
                },
            ],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };

        let events = trace_to_events(&result, "test", 2);
        // graph.started + (node.started + node.completed) * 2 + graph.completed = 6
        assert_eq!(events.len(), 6);
    }

    #[test]
    fn trace_to_events_populates_outputs_from_state() {
        use crate::core::runner::*;
        use crate::core::state::SharedState;
        use std::collections::HashMap;

        // Final state holds the node's real output (PRD-015 TEST-183).
        let state = SharedState::new();
        let mut out = HashMap::new();
        out.insert("response".to_string(), serde_json::json!("hello world"));
        state.set("b", out, true).unwrap();

        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state,
            trace: vec![TraceEntry {
                node_id: "b".into(),
                tool_type: "ai/llm_call".into(),
                status: TraceStatus::Ok,
                duration_ms: 5,
                retries: 0,
                started_at: 0.0,
                finished_at: 0.0,
                error: None,
            }],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };

        let events = trace_to_events(&result, "g", 1);

        // node.completed carries the real output + populated keys
        let (output, keys) = events
            .iter()
            .find_map(|e| match e {
                StreamEvent::NodeCompleted {
                    node_id,
                    output,
                    output_keys,
                    ..
                } if node_id == "b" => Some((output.clone(), output_keys.clone())),
                _ => None,
            })
            .expect("node.completed for b");
        assert_eq!(output["response"], serde_json::json!("hello world"));
        assert!(keys.contains(&"response".to_string()));

        // graph.completed carries the final snapshot
        let snapshot = events
            .iter()
            .find_map(|e| match e {
                StreamEvent::GraphCompleted { output, .. } => Some(output.clone()),
                _ => None,
            })
            .expect("graph.completed");
        assert_eq!(snapshot["b"]["response"], serde_json::json!("hello world"));
    }

    #[test]
    fn all_event_variants_serialize() {
        let events = vec![
            StreamEvent::GraphStarted {
                graph_name: "g".into(),
                node_count: 1,
            },
            StreamEvent::NodeStarted {
                node_id: "n".into(),
                tool_type: "t".into(),
            },
            StreamEvent::NodeToken {
                node_id: "n".into(),
                token: "hi".into(),
            },
            StreamEvent::NodeCompleted {
                node_id: "n".into(),
                tool_type: "t".into(),
                duration_ms: 10,
                output_keys: vec![],
                output: serde_json::Value::Null,
            },
            StreamEvent::NodeError {
                node_id: "n".into(),
                tool_type: "t".into(),
                error: "e".into(),
            },
            StreamEvent::FanoutStarted {
                source_node: "a".into(),
                parallel_nodes: vec!["b".into()],
            },
            StreamEvent::FanoutCompleted {
                succeeded: 1,
                failed: 0,
            },
            StreamEvent::GraphCompleted {
                status: "ok".into(),
                total_duration_ms: 100,
                nodes_executed: 2,
                output: serde_json::Value::Null,
            },
            StreamEvent::GraphError {
                error: "fail".into(),
            },
        ];
        for event in events {
            let sse = event.to_sse();
            assert!(sse.contains("event: "));
            assert!(sse.contains("data: "));
        }
    }
}
