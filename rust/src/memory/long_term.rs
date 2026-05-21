//! LongTermMemory -- persistent learning across sessions.
//!
//! Supports pluggable backends via the [`MemoryBackend`] trait.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::RunnerError;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single persistent memory entry (learning, decision, pattern, error resolution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongTermEntry {
    /// Unique identifier (UUID).
    pub id: String,
    /// Entry kind: `"decision"`, `"learning"`, `"pattern"`, `"error_resolution"`.
    pub entry_type: String,
    /// Human-readable content.
    pub content: String,
    /// Searchable tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Session that produced this entry, if applicable.
    pub session_id: Option<String>,
    /// Unix timestamp (seconds since epoch).
    pub created_at: f64,
    /// Arbitrary extra data.
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// Abstract persistence backend for long-term memory.
///
/// Implementations: [`super::InMemoryBackend`] (dev/testing), future SQLite/Postgres backends.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// Persist an entry. Returns the assigned ID.
    async fn save(&self, entry: &LongTermEntry) -> Result<String, RunnerError>;

    /// Full-text search. Returns up to `limit` matching entries.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError>;

    /// Look up a single entry by ID.
    async fn get(&self, id: &str) -> Result<Option<LongTermEntry>, RunnerError>;

    /// Delete an entry by ID.
    async fn delete(&self, id: &str) -> Result<(), RunnerError>;

    /// Most recent entries (newest first), up to `limit`.
    async fn list_recent(&self, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError>;
}

// ---------------------------------------------------------------------------
// LongTermMemory facade
// ---------------------------------------------------------------------------

/// High-level handle that delegates every operation to its [`MemoryBackend`].
pub struct LongTermMemory {
    backend: Box<dyn MemoryBackend>,
}

impl LongTermMemory {
    /// Wrap a backend implementation.
    pub fn new(backend: Box<dyn MemoryBackend>) -> Self {
        Self { backend }
    }

    /// Persist an entry. Returns the assigned ID.
    pub async fn save(&self, entry: &LongTermEntry) -> Result<String, RunnerError> {
        self.backend.save(entry).await
    }

    /// Full-text search.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LongTermEntry>, RunnerError> {
        self.backend.search(query, limit).await
    }

    /// Look up a single entry by ID.
    pub async fn get(&self, id: &str) -> Result<Option<LongTermEntry>, RunnerError> {
        self.backend.get(id).await
    }

    /// Delete an entry by ID.
    pub async fn delete(&self, id: &str) -> Result<(), RunnerError> {
        self.backend.delete(id).await
    }

    /// Most recent entries (newest first).
    pub async fn list_recent(&self, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError> {
        self.backend.list_recent(limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryBackend;

    fn make_entry(content: &str, entry_type: &str, tags: Vec<&str>) -> LongTermEntry {
        LongTermEntry {
            id: String::new(), // backend assigns
            entry_type: entry_type.into(),
            content: content.into(),
            tags: tags.into_iter().map(Into::into).collect(),
            session_id: Some("sess-1".into()),
            created_at: 0.0, // backend may override sort by insert order
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn save_and_get() {
        let mem = LongTermMemory::new(Box::new(InMemoryBackend::new()));

        let entry = make_entry("always retry on 429", "learning", vec!["http"]);
        let id = mem.save(&entry).await.unwrap();

        let fetched = mem.get(&id).await.unwrap().expect("entry must exist");
        assert_eq!(fetched.content, "always retry on 429");
        assert_eq!(fetched.entry_type, "learning");
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let mem = LongTermMemory::new(Box::new(InMemoryBackend::new()));

        mem.save(&make_entry("Retry on HTTP 429", "learning", vec![]))
            .await
            .unwrap();
        mem.save(&make_entry("Use exponential backoff", "pattern", vec![]))
            .await
            .unwrap();

        let results = mem.search("retry", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("429"));
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let mem = LongTermMemory::new(Box::new(InMemoryBackend::new()));

        let id = mem
            .save(&make_entry("temp", "decision", vec![]))
            .await
            .unwrap();
        assert!(mem.get(&id).await.unwrap().is_some());

        mem.delete(&id).await.unwrap();
        assert!(mem.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_recent_returns_newest_first() {
        let mem = LongTermMemory::new(Box::new(InMemoryBackend::new()));

        mem.save(&make_entry("old", "learning", vec![]))
            .await
            .unwrap();
        mem.save(&make_entry("new", "learning", vec![]))
            .await
            .unwrap();

        let recent = mem.list_recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "new");
        assert_eq!(recent[1].content, "old");
    }
}
