//! ShortTermMemory -- in-memory session trace during graph execution.
//!
//! Lives in RAM during execution. Captures logs, decisions, and per-block metrics.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single entry in the short-term session trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortTermEntry {
    /// Unix timestamp (seconds since epoch).
    pub timestamp: f64,
    /// Entry kind: `"log"`, `"decision"`, or `"metric"`.
    pub entry_type: String,
    /// Block/node that produced the entry, if applicable.
    pub node_id: Option<String>,
    /// Human-readable content.
    pub content: String,
    /// Arbitrary extra data.
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

/// Per-session memory: logs, decisions, and per-block metrics.
///
/// Captures the step-by-step detail of the current execution.
#[derive(Debug, Default)]
pub struct ShortTermMemory {
    entries: Vec<ShortTermEntry>,
}

impl ShortTermMemory {
    /// Create an empty short-term memory store.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record an entry.
    pub fn add(
        &mut self,
        entry_type: impl Into<String>,
        content: impl Into<String>,
        node_id: Option<String>,
        metadata: HashMap<String, Value>,
    ) {
        self.entries.push(ShortTermEntry {
            timestamp: now_secs(),
            entry_type: entry_type.into(),
            content: content.into(),
            node_id,
            metadata,
        });
    }

    /// Return a slice over all entries.
    pub fn get_all(&self) -> &[ShortTermEntry] {
        &self.entries
    }

    /// Filter entries by `entry_type`.
    pub fn get_by_type(&self, entry_type: &str) -> Vec<&ShortTermEntry> {
        self.entries
            .iter()
            .filter(|e| e.entry_type == entry_type)
            .collect()
    }

    /// Filter entries by `node_id`.
    pub fn get_by_node(&self, node_id: &str) -> Vec<&ShortTermEntry> {
        self.entries
            .iter()
            .filter(|e| e.node_id.as_deref() == Some(node_id))
            .collect()
    }

    /// Drop all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Current wall-clock as fractional seconds (UTC).
fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query() {
        let mut mem = ShortTermMemory::new();
        assert!(mem.is_empty());

        mem.add("log", "started graph", None, HashMap::new());
        mem.add(
            "decision",
            "took branch A",
            Some("node-1".into()),
            HashMap::new(),
        );
        mem.add(
            "metric",
            "duration 42ms",
            Some("node-1".into()),
            HashMap::new(),
        );

        assert_eq!(mem.len(), 3);
        assert_eq!(mem.get_by_type("log").len(), 1);
        assert_eq!(mem.get_by_type("decision").len(), 1);
        assert_eq!(mem.get_by_node("node-1").len(), 2);
        assert_eq!(mem.get_by_node("missing").len(), 0);
    }

    #[test]
    fn clear_resets() {
        let mut mem = ShortTermMemory::new();
        mem.add("log", "hello", None, HashMap::new());
        assert_eq!(mem.len(), 1);

        mem.clear();
        assert!(mem.is_empty());
    }

    #[test]
    fn get_all_returns_ordered() {
        let mut mem = ShortTermMemory::new();
        mem.add("log", "first", None, HashMap::new());
        mem.add("log", "second", None, HashMap::new());

        let all = mem.get_all();
        assert_eq!(all[0].content, "first");
        assert_eq!(all[1].content, "second");
    }
}
