//! MemoryFlusher -- auto-persist critical data before context compression.
//!
//! Monitors accumulated context size and flushes key decisions/learnings
//! to long-term memory when the threshold is reached.
//! Max 1 flush per session (REGLA-58).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::core::RunnerError;
use crate::memory::{LongTermEntry, LongTermMemory};

use super::tracer::TraceRecord;

/// Auto-persist critical data before context compression.
pub struct MemoryFlusher {
    /// Context-size ratio at which to trigger a flush (default 0.75).
    threshold_pct: f64,
    /// Maximum number of flushes per session (default 1).
    max_flushes_per_session: u32,
    /// Atomic counter of flushes performed.
    flush_count: AtomicU32,
}

impl MemoryFlusher {
    pub fn new() -> Self {
        Self {
            threshold_pct: 0.75,
            max_flushes_per_session: 1,
            flush_count: AtomicU32::new(0),
        }
    }

    /// Create with custom threshold and max flushes.
    pub fn with_config(threshold_pct: f64, max_flushes_per_session: u32) -> Self {
        Self {
            threshold_pct: threshold_pct.clamp(0.0, 1.0),
            max_flushes_per_session,
            flush_count: AtomicU32::new(0),
        }
    }

    /// Check whether a flush should occur.
    ///
    /// Returns `true` if `current_context_size / max_context_size >= threshold`
    /// AND `flush_count < max_flushes_per_session`.
    pub fn should_flush(&self, current_context_size: usize, max_context_size: usize) -> bool {
        if max_context_size == 0 {
            return false;
        }
        let current = self.flush_count.load(Ordering::Relaxed);
        if current >= self.max_flushes_per_session {
            return false;
        }
        let ratio = current_context_size as f64 / max_context_size as f64;
        ratio >= self.threshold_pct
    }

    /// Extract key decisions and learnings from traces and save to long-term memory.
    ///
    /// Returns the number of entries saved.
    pub async fn flush(
        &self,
        trace: &[TraceRecord],
        memory: &LongTermMemory,
    ) -> Result<usize, RunnerError> {
        let current = self.flush_count.load(Ordering::Relaxed);
        if current >= self.max_flushes_per_session {
            return Ok(0);
        }

        let entries = Self::extract_entries(trace);
        let count = entries.len();

        for entry in &entries {
            memory.save(entry).await?;
        }

        self.flush_count.fetch_add(1, Ordering::Relaxed);
        Ok(count)
    }

    /// Whether at least one flush has occurred.
    pub fn has_flushed(&self) -> bool {
        self.flush_count.load(Ordering::Relaxed) > 0
    }

    /// Current flush count.
    pub fn flush_count(&self) -> u32 {
        self.flush_count.load(Ordering::Relaxed)
    }

    /// Extract memory entries from trace records.
    ///
    /// Strategy: save error resolutions and patterns from failed/successful nodes.
    fn extract_entries(trace: &[TraceRecord]) -> Vec<LongTermEntry> {
        let mut entries = Vec::new();

        // Collect error nodes for error_resolution entries
        for record in trace
            .iter()
            .filter(|r| r.status == crate::core::runner::TraceStatus::Error)
        {
            let content = format!(
                "Node '{}' (type: {}) failed: {}",
                record.node_id,
                record.tool_type,
                record.error.as_deref().unwrap_or("unknown error")
            );
            entries.push(LongTermEntry {
                id: String::new(),
                entry_type: "error_resolution".to_string(),
                content,
                tags: vec!["auto-flush".to_string(), record.tool_type.clone()],
                session_id: None,
                created_at: record.timestamp,
                metadata: HashMap::new(),
            });
        }

        // Summarize overall execution as a learning
        let total = trace.len();
        let errors = trace
            .iter()
            .filter(|r| r.status == crate::core::runner::TraceStatus::Error)
            .count();
        let total_ms: u64 = trace.iter().map(|r| r.duration_ms).sum();

        if total > 0 {
            let summary = format!(
                "[Auto-flush] Session executed {} blocks ({} errors) in {}ms total",
                total, errors, total_ms
            );
            entries.push(LongTermEntry {
                id: String::new(),
                entry_type: "learning".to_string(),
                content: summary,
                tags: vec!["auto-flush".to_string()],
                session_id: None,
                created_at: 0.0,
                metadata: HashMap::new(),
            });
        }

        entries
    }
}

impl Default for MemoryFlusher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runner::TraceStatus;
    use crate::intelligence::tracer::TokenUsage;
    use crate::memory::InMemoryBackend;

    fn make_trace(node_id: &str, status: TraceStatus) -> TraceRecord {
        TraceRecord {
            node_id: node_id.to_string(),
            tool_type: "ai/llm_call".to_string(),
            inputs: HashMap::new(),
            output_keys: vec![],
            data_map_used: HashMap::new(),
            duration_ms: 100,
            tokens: Some(TokenUsage {
                input: 50,
                output: 25,
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
    fn should_flush_below_threshold() {
        let flusher = MemoryFlusher::new(); // 0.75 threshold
        assert!(!flusher.should_flush(500, 1000)); // 50% < 75%
    }

    #[test]
    fn should_flush_at_threshold() {
        let flusher = MemoryFlusher::new();
        assert!(flusher.should_flush(750, 1000)); // 75% == 75%
    }

    #[test]
    fn should_flush_above_threshold() {
        let flusher = MemoryFlusher::new();
        assert!(flusher.should_flush(900, 1000));
    }

    #[test]
    fn should_flush_zero_max() {
        let flusher = MemoryFlusher::new();
        assert!(!flusher.should_flush(100, 0));
    }

    #[test]
    fn should_flush_respects_max_flushes() {
        let flusher = MemoryFlusher::new();
        // Simulate having already flushed
        flusher.flush_count.store(1, Ordering::Relaxed);
        assert!(!flusher.should_flush(900, 1000));
    }

    #[tokio::test]
    async fn flush_saves_entries() {
        let flusher = MemoryFlusher::new();
        let memory = LongTermMemory::new(Box::new(InMemoryBackend::new()));
        let traces = vec![
            make_trace("n1", TraceStatus::Ok),
            make_trace("n2", TraceStatus::Error),
            make_trace("n3", TraceStatus::Ok),
        ];

        let saved = flusher.flush(&traces, &memory).await.unwrap();
        // 1 error entry + 1 summary = 2
        assert_eq!(saved, 2);
        assert!(flusher.has_flushed());
    }

    #[tokio::test]
    async fn flush_respects_max() {
        let flusher = MemoryFlusher::new();
        let memory = LongTermMemory::new(Box::new(InMemoryBackend::new()));
        let traces = vec![make_trace("n1", TraceStatus::Ok)];

        let first = flusher.flush(&traces, &memory).await.unwrap();
        assert!(first > 0);

        // Second flush should be blocked
        let second = flusher.flush(&traces, &memory).await.unwrap();
        assert_eq!(second, 0);
        assert_eq!(flusher.flush_count(), 1);
    }

    #[tokio::test]
    async fn flush_empty_traces() {
        let flusher = MemoryFlusher::new();
        let memory = LongTermMemory::new(Box::new(InMemoryBackend::new()));

        let saved = flusher.flush(&[], &memory).await.unwrap();
        assert_eq!(saved, 0);
    }

    #[test]
    fn extract_entries_errors() {
        let traces = vec![
            make_trace("n1", TraceStatus::Error),
            make_trace("n2", TraceStatus::Ok),
        ];
        let entries = MemoryFlusher::extract_entries(&traces);
        // 1 error entry + 1 summary
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_type, "error_resolution");
        assert!(entries[0].content.contains("n1"));
        assert!(entries[0].content.contains("timeout"));
        assert_eq!(entries[1].entry_type, "learning");
    }

    #[test]
    fn custom_config() {
        let flusher = MemoryFlusher::with_config(0.5, 3);
        assert!(flusher.should_flush(500, 1000)); // 50% == 50%
        assert!(!flusher.should_flush(400, 1000)); // 40% < 50%
    }
}
