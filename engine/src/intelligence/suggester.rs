//! Suggester -- LLM-powered graph self-improvement suggestions.
//!
//! Takes a graph definition and execution traces, then produces structured
//! suggestions for improving the graph: prompt changes, adding/removing nodes,
//! modifying config or data maps.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::context::LLMResource;
use crate::core::graph::GraphDef;

use super::tracer::TraceRecord;

/// Maximum reflections per prompt to keep size manageable (REGLA-50).
const MAX_REFLECTIONS_PER_PROMPT: usize = 30;

/// Types of graph improvement suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionType {
    PromptChange,
    AddNode,
    ModifyConfig,
    RemoveNode,
}

/// A single graph improvement suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub suggestion_type: SuggestionType,
    pub target_node_id: Option<String>,
    pub description: String,
    pub confidence: f64,
    pub rationale: String,
}

/// Generates graph improvement suggestions via LLM analysis.
pub struct Suggester;

impl Suggester {
    pub fn new() -> Self {
        Self
    }

    /// Generate improvement suggestions for a graph based on execution traces.
    ///
    /// Returns up to 5 suggestions ordered by confidence.
    pub async fn suggest(
        &self,
        graph: &GraphDef,
        trace: &[TraceRecord],
        llm: &dyn LLMResource,
    ) -> Result<Vec<Suggestion>, crate::core::ResourceError> {
        if graph.nodes.is_empty() {
            return Ok(Vec::new());
        }

        let prompt = Self::build_prompt(graph, trace);

        let response = llm.call("default", &prompt, &[], 0.4, Some(2048)).await?;

        Ok(Self::parse_suggestions(&response.response))
    }

    /// Build a structured prompt for the LLM.
    fn build_prompt(graph: &GraphDef, trace: &[TraceRecord]) -> String {
        // Serialize graph structure
        let nodes: Vec<Value> = graph
            .nodes
            .iter()
            .map(|n| {
                let mut entry = serde_json::json!({
                    "id": n.id,
                    "tool_type": n.tool_type,
                });
                if !n.config.is_empty() {
                    entry["config"] = serde_json::to_value(&n.config).unwrap_or(Value::Null);
                }
                entry
            })
            .collect();

        let edges: Vec<Value> = graph
            .edges
            .iter()
            .map(|e| {
                let mut entry = serde_json::json!({
                    "source": e.source,
                    "target": e.target,
                });
                if let Some(ref dm) = e.data_map {
                    entry["data_map"] = serde_json::to_value(dm).unwrap_or(Value::Null);
                }
                entry
            })
            .collect();

        let graph_json = serde_json::json!({
            "nodes": nodes,
            "edges": edges,
        });
        let graph_str = serde_json::to_string_pretty(&graph_json).unwrap_or_else(|_| "{}".into());

        // Serialize trace stats (capped)
        let capped_trace = if trace.len() > MAX_REFLECTIONS_PER_PROMPT {
            &trace[trace.len() - MAX_REFLECTIONS_PER_PROMPT..]
        } else {
            trace
        };

        let trace_entries: Vec<Value> = capped_trace
            .iter()
            .map(|t| {
                serde_json::json!({
                    "node_id": t.node_id,
                    "tool_type": t.tool_type,
                    "status": t.status,
                    "duration_ms": t.duration_ms,
                    "error": t.error,
                })
            })
            .collect();
        let trace_str =
            serde_json::to_string_pretty(&trace_entries).unwrap_or_else(|_| "[]".into());

        format!(
            r#"Analyze this agent graph and suggest improvements based on execution traces.

GRAPH:
{graph_str}

EXECUTION TRACES:
{trace_str}

Respond ONLY with a JSON array. Each element must have:
- "suggestion_type": one of "prompt_change", "add_node", "modify_config", "remove_node"
- "target_node_id": the node id this suggestion affects (or null for add_node)
- "description": a concise description of the improvement (1-2 sentences)
- "rationale": why this change would improve the graph
- "confidence": a float between 0.0 and 1.0

Rules:
- Focus on the most impactful improvements
- Suggest prompt improvements for LLM nodes with low success rates
- Suggest adding error handling nodes where failures are common
- Suggest removing unnecessary nodes that add latency without value
- Return an empty array [] if no meaningful improvements can be suggested
- Return at most 5 suggestions, ordered by confidence (highest first)
- Return ONLY valid JSON, no markdown, no explanation"#
        )
    }

    /// Parse LLM response into validated `Suggestion` vec.
    fn parse_suggestions(llm_output: &str) -> Vec<Suggestion> {
        let text = strip_code_fences(llm_output);

        let parsed: Vec<Value> = match serde_json::from_str(&text) {
            Ok(arr) => arr,
            Err(_) => return Vec::new(),
        };

        let valid_types = ["prompt_change", "add_node", "modify_config", "remove_node"];

        let mut suggestions: Vec<Suggestion> = parsed
            .iter()
            .filter_map(|item| {
                let obj = item.as_object()?;
                let stype_str = obj.get("suggestion_type")?.as_str()?;
                if !valid_types.contains(&stype_str) {
                    return None;
                }
                let suggestion_type = match stype_str {
                    "prompt_change" => SuggestionType::PromptChange,
                    "add_node" => SuggestionType::AddNode,
                    "modify_config" => SuggestionType::ModifyConfig,
                    "remove_node" => SuggestionType::RemoveNode,
                    _ => return None,
                };

                let confidence = obj
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0);

                Some(Suggestion {
                    suggestion_type,
                    target_node_id: obj
                        .get("target_node_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    description: obj
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    confidence,
                    rationale: obj
                        .get("rationale")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();

        // Sort by confidence descending, keep max 5
        suggestions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        suggestions.truncate(5);
        suggestions
    }
}

impl Default for Suggester {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip markdown code fences if present.
fn strip_code_fences(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let start = if lines.first().is_some_and(|l| l.starts_with("```")) {
        1
    } else {
        0
    };
    let end = if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.len() - 1
    } else {
        lines.len()
    };
    lines[start..end].join("\n").trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runner::TraceStatus;
    use crate::intelligence::tracer::TokenUsage;
    use std::collections::HashMap;

    fn make_trace(node_id: &str, status: TraceStatus, duration_ms: u64) -> TraceRecord {
        TraceRecord {
            node_id: node_id.to_string(),
            tool_type: "ai/llm_call".to_string(),
            inputs: HashMap::new(),
            output_keys: vec![],
            data_map_used: HashMap::new(),
            duration_ms,
            tokens: Some(TokenUsage {
                input: 100,
                output: 50,
            }),
            status,
            error: if status == TraceStatus::Error {
                Some("timeout".to_string())
            } else {
                None
            },
            timestamp: 1700000000.0,
        }
    }

    fn sample_graph() -> GraphDef {
        use crate::core::graph::{EdgeDef, NodeDef};
        GraphDef {
            id: "g1".into(),
            name: "test".into(),
            version: "1.0.0".into(),
            nodes: vec![
                NodeDef {
                    id: "n1".into(),
                    tool_type: "ai/llm_call".into(),
                    version: "1.0.0".into(),
                    config: HashMap::new(),
                    position: None,
                },
                NodeDef {
                    id: "n2".into(),
                    tool_type: "logic/condition".into(),
                    version: "1.0.0".into(),
                    config: HashMap::new(),
                    position: None,
                },
            ],
            edges: vec![EdgeDef {
                id: "e1".into(),
                source: "n1".into(),
                target: "n2".into(),
                condition: None,
                data_map: None,
            }],
            metadata: HashMap::new(),
            strict_completion: false,
        }
    }

    #[test]
    fn parse_suggestions_valid() {
        let json = r#"[
            {
                "suggestion_type": "prompt_change",
                "target_node_id": "n1",
                "description": "Improve prompt clarity",
                "rationale": "Node n1 has 40% failure rate",
                "confidence": 0.85
            },
            {
                "suggestion_type": "add_node",
                "target_node_id": null,
                "description": "Add error handler",
                "rationale": "No error handling exists",
                "confidence": 0.7
            }
        ]"#;
        let suggestions = Suggester::parse_suggestions(json);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].suggestion_type, SuggestionType::PromptChange);
        assert_eq!(suggestions[0].confidence, 0.85);
        assert_eq!(suggestions[1].suggestion_type, SuggestionType::AddNode);
        assert!(suggestions[1].target_node_id.is_none());
    }

    #[test]
    fn parse_suggestions_with_code_fences() {
        let json = "```json\n[\n{\"suggestion_type\": \"modify_config\", \"target_node_id\": \"n1\", \"description\": \"increase timeout\", \"rationale\": \"too slow\", \"confidence\": 0.6}\n]\n```";
        let suggestions = Suggester::parse_suggestions(json);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].suggestion_type, SuggestionType::ModifyConfig);
    }

    #[test]
    fn parse_suggestions_invalid_type_filtered() {
        let json = r#"[
            {"suggestion_type": "invalid_type", "target_node_id": "n1", "description": "x", "rationale": "y", "confidence": 0.5},
            {"suggestion_type": "remove_node", "target_node_id": "n2", "description": "remove", "rationale": "unused", "confidence": 0.9}
        ]"#;
        let suggestions = Suggester::parse_suggestions(json);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].suggestion_type, SuggestionType::RemoveNode);
    }

    #[test]
    fn parse_suggestions_garbage() {
        let suggestions = Suggester::parse_suggestions("not json at all");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn parse_suggestions_max_five() {
        let items: Vec<String> = (0..10)
            .map(|i| {
                format!(
                    r#"{{"suggestion_type": "prompt_change", "target_node_id": "n1", "description": "suggestion {}", "rationale": "reason", "confidence": {}}}"#,
                    i,
                    0.1 * i as f64
                )
            })
            .collect();
        let json = format!("[{}]", items.join(","));
        let suggestions = Suggester::parse_suggestions(&json);
        assert_eq!(suggestions.len(), 5);
        // Sorted by confidence descending
        assert!(suggestions[0].confidence >= suggestions[1].confidence);
    }

    #[test]
    fn parse_suggestions_clamps_confidence() {
        let json = r#"[{"suggestion_type": "prompt_change", "target_node_id": "n1", "description": "x", "rationale": "y", "confidence": 1.5}]"#;
        let suggestions = Suggester::parse_suggestions(json);
        assert_eq!(suggestions[0].confidence, 1.0);
    }

    #[test]
    fn build_prompt_includes_graph_and_traces() {
        let graph = sample_graph();
        let traces = vec![make_trace("n1", TraceStatus::Ok, 100)];
        let prompt = Suggester::build_prompt(&graph, &traces);
        assert!(prompt.contains("n1"));
        assert!(prompt.contains("GRAPH:"));
        assert!(prompt.contains("EXECUTION TRACES:"));
        assert!(prompt.contains("suggestion_type"));
    }

    #[test]
    fn suggestion_type_serde_roundtrip() {
        let st = SuggestionType::PromptChange;
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(json, "\"prompt_change\"");
        let back: SuggestionType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SuggestionType::PromptChange);
    }

    #[test]
    fn suggestion_serde_roundtrip() {
        let s = Suggestion {
            suggestion_type: SuggestionType::AddNode,
            target_node_id: None,
            description: "Add error handler".to_string(),
            confidence: 0.8,
            rationale: "Errors are common".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Suggestion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.suggestion_type, SuggestionType::AddNode);
        assert_eq!(back.confidence, 0.8);
    }
}
