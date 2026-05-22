//! SimpleVectorResource -- text search using SQLite FTS5.
//!
//! Not a real vector store, but provides basic text similarity search
//! so graphs with search nodes can function. Uses FTS5 ranking for relevance.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde_json::{json, Value};


use crate::core::context::{ResourceError, VectorResource, VectorSearchResult};

/// Text-based search resource backed by SQLite FTS5.
pub struct SimpleVectorResource {
    conn: Arc<Mutex<Connection>>,
}

unsafe impl Send for SimpleVectorResource {}
unsafe impl Sync for SimpleVectorResource {}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS vector_docs (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE VIRTUAL TABLE IF NOT EXISTS vector_fts USING fts5(
    text, id UNINDEXED,
    content='vector_docs',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS vdocs_ai AFTER INSERT ON vector_docs BEGIN
    INSERT INTO vector_fts(rowid, text, id)
    VALUES (new.rowid, new.text, new.id);
END;

CREATE TRIGGER IF NOT EXISTS vdocs_ad AFTER DELETE ON vector_docs BEGIN
    INSERT INTO vector_fts(vector_fts, rowid, text, id)
    VALUES ('delete', old.rowid, old.text, old.id);
END;

CREATE TRIGGER IF NOT EXISTS vdocs_au AFTER UPDATE ON vector_docs BEGIN
    INSERT INTO vector_fts(vector_fts, rowid, text, id)
    VALUES ('delete', old.rowid, old.text, old.id);
    INSERT INTO vector_fts(rowid, text, id)
    VALUES (new.rowid, new.text, new.id);
END;
"#;

impl SimpleVectorResource {
    /// Open (or create) a vector store backed by SQLite at `path`.
    pub fn new(path: &str) -> Result<Self, ResourceError> {
        let conn = Connection::open(path).map_err(|e| {
            ResourceError::Other(format!("failed to open vector store: {e}"))
        })?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| ResourceError::Other(format!("PRAGMA failed: {e}")))?;

        conn.execute_batch(SCHEMA)
            .map_err(|e| ResourceError::Other(format!("schema init failed: {e}")))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// In-memory vector store for testing.
    pub fn new_in_memory() -> Result<Self, ResourceError> {
        Self::new(":memory:")
    }

    /// Prepare FTS5 query: each word gets quoted and prefix-matched.
    fn prepare_fts_query(query: &str) -> String {
        let cleaned = query.trim();
        if cleaned.is_empty() {
            return String::new();
        }
        cleaned
            .split_whitespace()
            .map(|token| format!("\"{token}\"*"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[async_trait]
impl VectorResource for SimpleVectorResource {
    async fn upsert(
        &self,
        id: &str,
        text: &str,
        metadata: Value,
    ) -> Result<(), ResourceError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        let text = text.to_string();
        let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".into());

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Other(format!("mutex poisoned: {e}")))?;

            conn.execute(
                "INSERT INTO vector_docs (id, text, metadata) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET text = excluded.text, metadata = excluded.metadata",
                params![id, text, metadata_json],
            )
            .map_err(|e| ResourceError::Other(format!("upsert failed: {e}")))?;

            Ok(())
        })
        .await
        .map_err(|e| ResourceError::Other(format!("spawn_blocking join: {e}")))?
    }

    async fn search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<VectorSearchResult>, ResourceError> {
        let fts_query = Self::prepare_fts_query(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }

        let conn = Arc::clone(&self.conn);
        let top_k = top_k as i64;

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Other(format!("mutex poisoned: {e}")))?;

            let mut stmt = conn
                .prepare(
                    "SELECT d.id, d.metadata, rank
                     FROM vector_fts f
                     JOIN vector_docs d ON d.rowid = f.rowid
                     WHERE vector_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )
                .map_err(|e| ResourceError::Other(format!("prepare failed: {e}")))?;

            let rows = stmt
                .query_map(params![fts_query, top_k], |row| {
                    let id: String = row.get(0).unwrap_or_default();
                    let metadata_json: String = row.get(1).unwrap_or_else(|_| "{}".into());
                    let rank: f64 = row.get(2).unwrap_or(0.0);
                    let metadata: Value =
                        serde_json::from_str(&metadata_json).unwrap_or(json!({}));
                    // FTS5 rank is negative (lower = better). Normalize to 0..1 range.
                    let score = (-rank).max(0.0);
                    Ok(VectorSearchResult { id, score, metadata })
                })
                .map_err(|e| ResourceError::Other(format!("query failed: {e}")))?;

            let mut results = Vec::new();
            for row in rows {
                results.push(
                    row.map_err(|e| ResourceError::Other(format!("row read: {e}")))?,
                );
            }
            Ok(results)
        })
        .await
        .map_err(|e| ResourceError::Other(format!("spawn_blocking join: {e}")))?
    }

    async fn delete(&self, id: &str) -> Result<(), ResourceError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Other(format!("mutex poisoned: {e}")))?;

            conn.execute("DELETE FROM vector_docs WHERE id = ?1", params![id])
                .map_err(|e| ResourceError::Other(format!("delete failed: {e}")))?;

            Ok(())
        })
        .await
        .map_err(|e| ResourceError::Other(format!("spawn_blocking join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_search() {
        let store = SimpleVectorResource::new_in_memory().unwrap();

        store
            .upsert("doc-1", "Rust is a systems programming language", json!({"source": "wiki"}))
            .await
            .unwrap();
        store
            .upsert("doc-2", "Python is great for data science", json!({"source": "blog"}))
            .await
            .unwrap();
        store
            .upsert("doc-3", "Rust and Python can work together via PyO3", json!({}))
            .await
            .unwrap();

        let results = store.search("Rust programming", 10).await.unwrap();
        assert!(!results.is_empty());
        // First result should be about Rust
        assert!(results[0].id == "doc-1" || results[0].id == "doc-3");
    }

    #[tokio::test]
    async fn upsert_updates_existing() {
        let store = SimpleVectorResource::new_in_memory().unwrap();

        store
            .upsert("doc-1", "original text", json!({}))
            .await
            .unwrap();
        store
            .upsert("doc-1", "updated text", json!({"version": 2}))
            .await
            .unwrap();

        let results = store.search("updated", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc-1");
    }

    #[tokio::test]
    async fn delete_removes_from_search() {
        let store = SimpleVectorResource::new_in_memory().unwrap();

        store
            .upsert("doc-1", "searchable content", json!({}))
            .await
            .unwrap();
        let before = store.search("searchable", 10).await.unwrap();
        assert_eq!(before.len(), 1);

        store.delete("doc-1").await.unwrap();
        let after = store.search("searchable", 10).await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn empty_search_returns_empty() {
        let store = SimpleVectorResource::new_in_memory().unwrap();
        store
            .upsert("doc-1", "some content", json!({}))
            .await
            .unwrap();

        let results = store.search("", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let store = SimpleVectorResource::new_in_memory().unwrap();
        for i in 0..10 {
            store
                .upsert(&format!("doc-{i}"), &format!("keyword content {i}"), json!({}))
                .await
                .unwrap();
        }

        let results = store.search("keyword", 3).await.unwrap();
        assert_eq!(results.len(), 3);
    }
}
