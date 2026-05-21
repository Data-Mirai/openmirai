//! ExecutionTracer -- fire-and-forget trace capture.
//!
//! Captures rich execution data per node: inputs, outputs, data_map,
//! duration, tokens, status, errors. Never blocks the graph execution pipeline.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Token usage for a single trace record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}

/// A single trace record captured during node execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub node_id: String,
    pub tool_type: String,
    /// Input keys only; values truncated beyond 500 chars.
    pub inputs: HashMap<String, Value>,
    /// Keys present in the output.
    pub output_keys: Vec<String>,
    /// Data map entries used during this execution.
    pub data_map_used: HashMap<String, String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
    /// `"ok"`, `"error"`, or `"skipped"`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: f64,
}

/// Aggregate trace summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    pub total_blocks: usize,
    pub successful: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_ms: u64,
    pub tokens: TokenUsage,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Maximum characters for a serialized input value before truncation.
const MAX_VALUE_CHARS: usize = 500;

/// Truncate a JSON value's string representation if it exceeds `MAX_VALUE_CHARS`.
fn truncate_value(v: &Value) -> Value {
    let serialized = serde_json::to_string(v).unwrap_or_default();
    if serialized.len() <= MAX_VALUE_CHARS {
        v.clone()
    } else {
        let truncated = &serialized[..MAX_VALUE_CHARS];
        Value::String(format!("{}...[truncated]", truncated))
    }
}

/// Build inputs map with truncated values.
pub fn sanitize_inputs(raw: &HashMap<String, Value>) -> HashMap<String, Value> {
    raw.iter()
        .map(|(k, v)| (k.clone(), truncate_value(v)))
        .collect()
}

// ---------------------------------------------------------------------------
// ExecutionTracer
// ---------------------------------------------------------------------------

/// Passive trace collector. Records are stored in-memory and can be retrieved
/// after execution for reflection, persistence, or debugging.
#[derive(Debug, Default)]
pub struct ExecutionTracer {
    records: Vec<TraceRecord>,
}

impl ExecutionTracer {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Append a trace record.
    pub fn add(&mut self, record: TraceRecord) {
        self.records.push(record);
    }

    /// All recorded traces.
    pub fn get_all(&self) -> &[TraceRecord] {
        &self.records
    }

    /// Filter traces by node ID.
    pub fn get_by_node(&self, node_id: &str) -> Vec<&TraceRecord> {
        self.records
            .iter()
            .filter(|r| r.node_id == node_id)
            .collect()
    }

    /// Total execution time across all records.
    pub fn total_duration_ms(&self) -> u64 {
        self.records.iter().map(|r| r.duration_ms).sum()
    }

    /// Aggregate token usage across all records.
    pub fn total_tokens(&self) -> TokenUsage {
        let mut total = TokenUsage::default();
        for r in &self.records {
            if let Some(ref t) = r.tokens {
                total.input += t.input;
                total.output += t.output;
            }
        }
        total
    }

    /// Count of records with `status == "error"`.
    pub fn error_count(&self) -> usize {
        self.records.iter().filter(|r| r.status == "error").count()
    }

    /// Produce an aggregate summary of all captured traces.
    pub fn summary(&self) -> TraceSummary {
        let total_blocks = self.records.len();
        let successful = self.records.iter().filter(|r| r.status == "ok").count();
        let failed = self.error_count();
        let skipped = self
            .records
            .iter()
            .filter(|r| r.status == "skipped")
            .count();
        TraceSummary {
            total_blocks,
            successful,
            failed,
            skipped,
            duration_ms: self.total_duration_ms(),
            tokens: self.total_tokens(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(node_id: &str, status: &str, duration_ms: u64) -> TraceRecord {
        TraceRecord {
            node_id: node_id.to_string(),
            tool_type: "ai/llm_call".to_string(),
            inputs: HashMap::new(),
            output_keys: vec!["result".to_string()],
            data_map_used: HashMap::new(),
            duration_ms,
            tokens: Some(TokenUsage {
                input: 100,
                output: 50,
            }),
            status: status.to_string(),
            error: if status == "error" {
                Some("boom".to_string())
            } else {
                None
            },
            timestamp: 1700000000.0,
        }
    }

    #[test]
    fn empty_tracer() {
        let tracer = ExecutionTracer::new();
        assert!(tracer.get_all().is_empty());
        assert_eq!(tracer.total_duration_ms(), 0);
        assert_eq!(tracer.error_count(), 0);

        let s = tracer.summary();
        assert_eq!(s.total_blocks, 0);
        assert_eq!(s.successful, 0);
        assert_eq!(s.failed, 0);
        assert_eq!(s.skipped, 0);
    }

    #[test]
    fn add_and_retrieve() {
        let mut tracer = ExecutionTracer::new();
        tracer.add(make_record("n1", "ok", 100));
        tracer.add(make_record("n2", "error", 200));
        tracer.add(make_record("n1", "ok", 50));

        assert_eq!(tracer.get_all().len(), 3);
        assert_eq!(tracer.get_by_node("n1").len(), 2);
        assert_eq!(tracer.get_by_node("n2").len(), 1);
        assert_eq!(tracer.get_by_node("n3").len(), 0);
    }

    #[test]
    fn duration_and_tokens() {
        let mut tracer = ExecutionTracer::new();
        tracer.add(make_record("n1", "ok", 100));
        tracer.add(make_record("n2", "ok", 200));

        assert_eq!(tracer.total_duration_ms(), 300);
        let tok = tracer.total_tokens();
        assert_eq!(tok.input, 200);
        assert_eq!(tok.output, 100);
    }

    #[test]
    fn error_count() {
        let mut tracer = ExecutionTracer::new();
        tracer.add(make_record("n1", "ok", 100));
        tracer.add(make_record("n2", "error", 50));
        tracer.add(make_record("n3", "skipped", 0));
        tracer.add(make_record("n4", "error", 30));

        assert_eq!(tracer.error_count(), 2);
    }

    #[test]
    fn summary_counts() {
        let mut tracer = ExecutionTracer::new();
        tracer.add(make_record("n1", "ok", 100));
        tracer.add(make_record("n2", "error", 50));
        tracer.add(make_record("n3", "skipped", 10));

        let s = tracer.summary();
        assert_eq!(s.total_blocks, 3);
        assert_eq!(s.successful, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.duration_ms, 160);
    }

    #[test]
    fn truncate_value_short() {
        let v = Value::String("hello".to_string());
        let result = truncate_value(&v);
        assert_eq!(result, v);
    }

    #[test]
    fn truncate_value_long() {
        let long = "x".repeat(1000);
        let v = Value::String(long);
        let result = truncate_value(&v);
        match result {
            Value::String(s) => assert!(s.contains("[truncated]")),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn sanitize_inputs_truncates() {
        let mut raw = HashMap::new();
        raw.insert("short".to_string(), Value::String("hi".to_string()));
        raw.insert("long".to_string(), Value::String("y".repeat(1000)));

        let sanitized = sanitize_inputs(&raw);
        assert_eq!(sanitized.len(), 2);

        // Short value unchanged
        assert_eq!(sanitized["short"], Value::String("hi".to_string()));

        // Long value truncated
        match &sanitized["long"] {
            Value::String(s) => assert!(s.contains("[truncated]")),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn tokens_none_handled() {
        let mut tracer = ExecutionTracer::new();
        let mut rec = make_record("n1", "ok", 100);
        rec.tokens = None;
        tracer.add(rec);

        let tok = tracer.total_tokens();
        assert_eq!(tok.input, 0);
        assert_eq!(tok.output, 0);
    }

    #[test]
    fn trace_record_serde_roundtrip() {
        let record = make_record("n1", "ok", 100);
        let json = serde_json::to_string(&record).unwrap();
        let back: TraceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.node_id, "n1");
        assert_eq!(back.status, "ok");
        assert_eq!(back.duration_ms, 100);
    }
}
