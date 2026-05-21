//! InMemoryBackend -- default MemoryBackend for dev/testing.
//!
//! No persistence -- data lives in RAM only. Thread-safe via `Arc<RwLock<_>>`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::core::RunnerError;

use super::long_term::{LongTermEntry, MemoryBackend};

/// In-memory backend backed by a shared `Vec`. Safe to clone and share across tasks.
#[derive(Debug, Clone)]
pub struct InMemoryBackend {
    entries: Arc<RwLock<Vec<LongTermEntry>>>,
}

impl InMemoryBackend {
    /// Create an empty backend.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MemoryBackend for InMemoryBackend {
    async fn save(&self, entry: &LongTermEntry) -> Result<String, RunnerError> {
        let id = Uuid::new_v4().to_string();
        let mut stored = entry.clone();
        stored.id = id.clone();

        // Assign wall-clock if the caller left it at zero.
        if stored.created_at == 0.0 {
            stored.created_at = now_secs();
        }

        self.entries.write().await.push(stored);
        Ok(id)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError> {
        let query_lower = query.to_lowercase();
        let guard = self.entries.read().await;

        let mut matches: Vec<LongTermEntry> = guard
            .iter()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .cloned()
            .collect();

        // Newest first.
        matches.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
        matches.truncate(limit);
        Ok(matches)
    }

    async fn get(&self, id: &str) -> Result<Option<LongTermEntry>, RunnerError> {
        let guard = self.entries.read().await;
        Ok(guard.iter().find(|e| e.id == id).cloned())
    }

    async fn delete(&self, id: &str) -> Result<(), RunnerError> {
        let mut guard = self.entries.write().await;
        guard.retain(|e| e.id != id);
        Ok(())
    }

    async fn list_recent(&self, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError> {
        let guard = self.entries.read().await;
        let mut all: Vec<LongTermEntry> = guard.clone();

        // Newest first.
        all.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(limit);
        Ok(all)
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
    use std::collections::HashMap;

    use super::*;

    fn entry(content: &str) -> LongTermEntry {
        LongTermEntry {
            id: String::new(),
            entry_type: "learning".into(),
            content: content.into(),
            tags: vec![],
            session_id: None,
            created_at: 0.0,
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn save_assigns_uuid() {
        let backend = InMemoryBackend::new();
        let id = backend.save(&entry("hello")).await.unwrap();
        assert!(!id.is_empty());
        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(id.len(), 36);
    }

    #[tokio::test]
    async fn search_is_case_insensitive() {
        let backend = InMemoryBackend::new();
        backend.save(&entry("UPPERCASE content")).await.unwrap();
        backend.save(&entry("lowercase content")).await.unwrap();
        backend.save(&entry("no match here")).await.unwrap();

        let results = backend.search("content", 10).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let backend = InMemoryBackend::new();
        for i in 0..5 {
            backend
                .save(&entry(&format!("entry {i}")))
                .await
                .unwrap();
        }

        let results = backend.search("entry", 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn get_returns_none_for_missing() {
        let backend = InMemoryBackend::new();
        assert!(backend.get("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let backend = InMemoryBackend::new();
        // Deleting a missing ID is not an error.
        backend.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn list_recent_respects_limit() {
        let backend = InMemoryBackend::new();
        for i in 0..5 {
            backend
                .save(&entry(&format!("entry {i}")))
                .await
                .unwrap();
        }

        let recent = backend.list_recent(3).await.unwrap();
        assert_eq!(recent.len(), 3);
    }
}
