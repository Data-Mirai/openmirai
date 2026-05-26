//! Reflector -- LLM-powered execution analysis.
//!
//! Takes execution traces from completed sessions and produces structured
//! reflections: patterns (success/failure), optimizations, and anomalies.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::context::LLMResource;

use super::tracer::TraceRecord;

/// Maximum traces per batch to keep the LLM prompt manageable (REGLA-45).
const MAX_TRACES_PER_BATCH: usize = 50;

/// Valid reflection types.
const REFLECTION_TYPES: &[&str] = &[
    "success_pattern",
    "failure_pattern",
    "optimization",
    "anomaly",
];

/// Structured output from an LLM-powered trace analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reflection {
    /// Success/failure patterns detected.
    pub patterns: Vec<String>,
    /// Suggested improvements.
    pub optimizations: Vec<String>,
    /// Unusual behaviors.
    pub anomalies: Vec<String>,
    /// Executive summary.
    pub summary: String,
}

/// A single reflection entry parsed from LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub node_id: Option<String>,
    pub insight: String,
    pub confidence: f64,
}

/// Analyzes execution traces via LLM to extract actionable insights.
pub struct Reflector;

impl Reflector {
    pub fn new() -> Self {
        Self
    }

    /// Run reflection on traces using the provided LLM resource.
    ///
    /// Returns a `Reflection` with categorized insights. If traces is empty,
    /// returns a default empty reflection.
    pub async fn reflect(
        &self,
        trace: &[TraceRecord],
        llm: &dyn LLMResource,
    ) -> Result<Reflection, crate::core::ResourceError> {
        if trace.is_empty() {
            return Ok(Reflection {
                patterns: Vec::new(),
                optimizations: Vec::new(),
                anomalies: Vec::new(),
                summary: "No traces to analyze.".to_string(),
            });
        }

        let batch = Self::build_batch(trace);
        let prompt = Self::build_prompt(&batch);

        let response = llm
            .call("default", &prompt, &[], 0.3, 2048)
            .await?;

        Ok(Self::parse_response(&response.response))
    }

    /// Cap traces to `MAX_TRACES_PER_BATCH`, keeping the most recent.
    fn build_batch(traces: &[TraceRecord]) -> &[TraceRecord] {
        if traces.len() <= MAX_TRACES_PER_BATCH {
            traces
        } else {
            &traces[traces.len() - MAX_TRACES_PER_BATCH..]
        }
    }

    /// Build a structured prompt for the LLM to analyze traces.
    fn build_prompt(traces: &[TraceRecord]) -> String {
        let serialized: Vec<Value> = traces
            .iter()
            .map(|t| {
                serde_json::json!({
                    "node_id": t.node_id,
                    "tool_type": t.tool_type,
                    "status": t.status,
                    "duration_ms": t.duration_ms,
                    "tokens_input": t.tokens.as_ref().map(|tk| tk.input).unwrap_or(0),
                    "tokens_output": t.tokens.as_ref().map(|tk| tk.output).unwrap_or(0),
                    "error": t.error,
                })
            })
            .collect();

        let traces_json =
            serde_json::to_string_pretty(&serialized).unwrap_or_else(|_| "[]".to_string());

        format!(
            r#"Analyze the execution traces and extract insights.

TRACES:
{traces_json}

Respond ONLY with a JSON object with these keys:
- "patterns": array of strings describing success/failure patterns
- "optimizations": array of strings with suggested improvements
- "anomalies": array of strings with unusual behaviors
- "summary": a 1-2 sentence executive summary

Rules:
- Look for repeated errors on the same node (failure patterns)
- Look for nodes that consistently succeed fast (success patterns)
- Look for nodes with high token usage or slow duration (optimizations)
- Look for unusual patterns (anomalies)
- Return ONLY valid JSON, no markdown, no explanation"#
        )
    }

    /// Parse LLM response into a `Reflection`. Falls back to raw text on parse error.
    fn parse_response(llm_output: &str) -> Reflection {
        let text = strip_code_fences(llm_output);

        // Try JSON object parse first
        if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
            if let Some(obj) = parsed.as_object() {
                return Reflection {
                    patterns: extract_string_array(obj.get("patterns")),
                    optimizations: extract_string_array(obj.get("optimizations")),
                    anomalies: extract_string_array(obj.get("anomalies")),
                    summary: obj
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                };
            }
        }

        // Try JSON array parse (older format from Python reflector)
        if let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&text) {
            let mut patterns = Vec::new();
            let mut optimizations = Vec::new();
            let mut anomalies = Vec::new();

            for item in &parsed {
                let entry_type = item
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let insight = item
                    .get("insight")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !REFLECTION_TYPES.contains(&entry_type) || insight.is_empty() {
                    continue;
                }

                match entry_type {
                    "success_pattern" | "failure_pattern" => patterns.push(insight),
                    "optimization" => optimizations.push(insight),
                    "anomaly" => anomalies.push(insight),
                    _ => {}
                }
            }

            return Reflection {
                summary: "Parsed from array format.".to_string(),
                patterns,
                optimizations,
                anomalies,
            };
        }

        // Fallback: raw text
        Reflection {
            patterns: Vec::new(),
            optimizations: Vec::new(),
            anomalies: vec![format!(
                "LLM analysis could not be parsed. Raw snippet: {}",
                &text[..text.len().min(200)]
            )],
            summary: "Parse failed; see anomalies.".to_string(),
        }
    }
}

impl Default for Reflector {
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
    let start = if lines.first().map_or(false, |l| l.starts_with("```")) {
        1
    } else {
        0
    };
    let end = if lines.last().map_or(false, |l| l.trim() == "```") {
        lines.len() - 1
    } else {
        lines.len()
    };
    lines[start..end].join("\n").trim().to_string()
}

/// Extract a Vec<String> from a JSON array value.
fn extract_string_array(val: Option<&Value>) -> Vec<String> {
    val.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
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

    fn make_trace(node_id: &str, status: TraceStatus, duration: u64) -> TraceRecord {
        TraceRecord {
            node_id: node_id.to_string(),
            tool_type: "ai/llm_call".to_string(),
            inputs: HashMap::new(),
            output_keys: vec!["result".to_string()],
            data_map_used: HashMap::new(),
            duration_ms: duration,
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

    #[test]
    fn build_batch_under_limit() {
        let traces: Vec<TraceRecord> = (0..5).map(|i| make_trace(&format!("n{}", i), TraceStatus::Ok, 100)).collect();
        let batch = Reflector::build_batch(&traces);
        assert_eq!(batch.len(), 5);
    }

    #[test]
    fn build_batch_over_limit() {
        let traces: Vec<TraceRecord> = (0..100)
            .map(|i| make_trace(&format!("n{}", i), TraceStatus::Ok, 100))
            .collect();
        let batch = Reflector::build_batch(&traces);
        assert_eq!(batch.len(), MAX_TRACES_PER_BATCH);
        // Should keep the most recent (last 50)
        assert_eq!(batch[0].node_id, "n50");
    }

    #[test]
    fn parse_response_json_object() {
        let json = r#"{
            "patterns": ["nodes n1/n2 always succeed fast"],
            "optimizations": ["reduce token usage on n3"],
            "anomalies": [],
            "summary": "Mostly healthy execution."
        }"#;
        let r = Reflector::parse_response(json);
        assert_eq!(r.patterns.len(), 1);
        assert_eq!(r.optimizations.len(), 1);
        assert!(r.anomalies.is_empty());
        assert_eq!(r.summary, "Mostly healthy execution.");
    }

    #[test]
    fn parse_response_json_array_format() {
        let json = r#"[
            {"type": "success_pattern", "node_id": "n1", "insight": "fast node", "confidence": 0.9},
            {"type": "optimization", "node_id": "n2", "insight": "reduce tokens", "confidence": 0.7}
        ]"#;
        let r = Reflector::parse_response(json);
        assert_eq!(r.patterns.len(), 1);
        assert_eq!(r.optimizations.len(), 1);
    }

    #[test]
    fn parse_response_with_code_fences() {
        let json = "```json\n{\"patterns\": [\"ok\"], \"optimizations\": [], \"anomalies\": [], \"summary\": \"fine\"}\n```";
        let r = Reflector::parse_response(json);
        assert_eq!(r.patterns, vec!["ok"]);
        assert_eq!(r.summary, "fine");
    }

    #[test]
    fn parse_response_fallback() {
        let garbage = "this is not json at all";
        let r = Reflector::parse_response(garbage);
        assert_eq!(r.anomalies.len(), 1);
        assert!(r.anomalies[0].contains("could not be parsed"));
    }

    #[test]
    fn strip_code_fences_no_fences() {
        assert_eq!(strip_code_fences("hello"), "hello");
    }

    #[test]
    fn strip_code_fences_with_lang() {
        let input = "```json\n{\"a\":1}\n```";
        assert_eq!(strip_code_fences(input), "{\"a\":1}");
    }

    #[test]
    fn build_prompt_includes_traces() {
        let traces = vec![make_trace("n1", TraceStatus::Ok, 100)];
        let prompt = Reflector::build_prompt(&traces);
        assert!(prompt.contains("n1"));
        assert!(prompt.contains("TRACES:"));
        assert!(prompt.contains("Respond ONLY"));
    }
}
