//! SqliteBackend -- persistent MemoryBackend with FTS5 full-text search.
//!
//! Uses `rusqlite` for storage. Since rusqlite is synchronous, all DB calls
//! are wrapped in `tokio::task::spawn_blocking` to avoid blocking the async
//! runtime.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::core::RunnerError;

use super::long_term::{LongTermEntry, MemoryBackend};

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS long_term_memory (
    id TEXT PRIMARY KEY,
    entry_type TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    session_id TEXT,
    created_at REAL NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content, tags, entry_type,
    content='long_term_memory',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS memory_ai AFTER INSERT ON long_term_memory BEGIN
    INSERT INTO memory_fts(rowid, content, tags, entry_type)
    VALUES (new.rowid, new.content, new.tags, new.entry_type);
END;

CREATE TRIGGER IF NOT EXISTS memory_ad AFTER DELETE ON long_term_memory BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, tags, entry_type)
    VALUES ('delete', old.rowid, old.content, old.tags, old.entry_type);
END;
"#;

// ---------------------------------------------------------------------------
// SqliteBackend
// ---------------------------------------------------------------------------

/// Persistent memory backend backed by SQLite with FTS5.
pub struct SqliteBackend {
    conn: Arc<Mutex<Connection>>,
}

// rusqlite::Connection is Send but not Sync. We guard it with a Mutex, which
// makes the wrapper Send + Sync as required by MemoryBackend.
unsafe impl Send for SqliteBackend {}
unsafe impl Sync for SqliteBackend {}

impl SqliteBackend {
    /// Open (or create) a SQLite database at `path` and initialise the schema.
    pub fn new(path: &str) -> Result<Self, RunnerError> {
        let conn = Connection::open(path).map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("failed to open SQLite database: {e}"),
        })?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("failed to set PRAGMAs: {e}"),
            })?;

        conn.execute_batch(SCHEMA).map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("failed to initialise schema: {e}"),
        })?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database (for testing).
    pub fn new_in_memory() -> Result<Self, RunnerError> {
        Self::new(":memory:")
    }

    /// Prepare an FTS5 MATCH query from human-readable text.
    ///
    /// Each word gets wrapped in quotes and suffixed with `*` for prefix matching.
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
impl MemoryBackend for SqliteBackend {
    async fn save(&self, entry: &LongTermEntry) -> Result<String, RunnerError> {
        let id = Uuid::new_v4().to_string();
        let entry_type = entry.entry_type.clone();
        let content = entry.content.clone();
        let tags_json = serde_json::to_string(&entry.tags).unwrap_or_else(|_| "[]".into());
        let session_id = entry.session_id.clone();
        let metadata_json =
            serde_json::to_string(&entry.metadata).unwrap_or_else(|_| "{}".into());

        let created_at = if entry.created_at == 0.0 {
            now_secs()
        } else {
            entry.created_at
        };

        let conn = Arc::clone(&self.conn);
        let id_clone = id.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("mutex poisoned: {e}"),
            })?;

            conn.execute(
                "INSERT INTO long_term_memory (id, entry_type, content, tags, session_id, created_at, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id_clone, entry_type, content, tags_json, session_id, created_at, metadata_json],
            )
            .map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("INSERT failed: {e}"),
            })?;

            Ok(id_clone)
        })
        .await
        .map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("spawn_blocking join error: {e}"),
        })?
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError> {
        let fts_query = Self::prepare_fts_query(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }

        let conn = Arc::clone(&self.conn);
        let limit = limit as i64;

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("mutex poisoned: {e}"),
            })?;

            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.entry_type, m.content, m.tags, m.session_id, m.created_at, m.metadata
                     FROM long_term_memory m
                     JOIN memory_fts f ON m.rowid = f.rowid
                     WHERE memory_fts MATCH ?1
                     ORDER BY m.created_at DESC
                     LIMIT ?2",
                )
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("prepare search failed: {e}"),
                })?;

            let rows = stmt
                .query_map(params![fts_query, limit], |row| {
                    Ok(row_to_entry(row))
                })
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("search query failed: {e}"),
                })?;

            let mut entries = Vec::new();
            for row in rows {
                entries.push(row.map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("row read failed: {e}"),
                })?);
            }
            Ok(entries)
        })
        .await
        .map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("spawn_blocking join error: {e}"),
        })?
    }

    async fn get(&self, id: &str) -> Result<Option<LongTermEntry>, RunnerError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("mutex poisoned: {e}"),
            })?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, entry_type, content, tags, session_id, created_at, metadata
                     FROM long_term_memory WHERE id = ?1",
                )
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("prepare get failed: {e}"),
                })?;

            let mut rows = stmt
                .query_map(params![id], |row| Ok(row_to_entry(row)))
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("get query failed: {e}"),
                })?;

            match rows.next() {
                Some(Ok(entry)) => Ok(Some(entry)),
                Some(Err(e)) => Err(RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("row read failed: {e}"),
                }),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("spawn_blocking join error: {e}"),
        })?
    }

    async fn delete(&self, id: &str) -> Result<(), RunnerError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("mutex poisoned: {e}"),
            })?;

            conn.execute("DELETE FROM long_term_memory WHERE id = ?1", params![id])
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("DELETE failed: {e}"),
                })?;

            Ok(())
        })
        .await
        .map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("spawn_blocking join error: {e}"),
        })?
    }

    async fn list_recent(&self, limit: usize) -> Result<Vec<LongTermEntry>, RunnerError> {
        let conn = Arc::clone(&self.conn);
        let limit = limit as i64;

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| RunnerError::ExecutionFailed {
                node_id: "sqlite_backend".into(),
                message: format!("mutex poisoned: {e}"),
            })?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, entry_type, content, tags, session_id, created_at, metadata
                     FROM long_term_memory
                     ORDER BY created_at DESC
                     LIMIT ?1",
                )
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("prepare list_recent failed: {e}"),
                })?;

            let rows = stmt
                .query_map(params![limit], |row| Ok(row_to_entry(row)))
                .map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("list_recent query failed: {e}"),
                })?;

            let mut entries = Vec::new();
            for row in rows {
                entries.push(row.map_err(|e| RunnerError::ExecutionFailed {
                    node_id: "sqlite_backend".into(),
                    message: format!("row read failed: {e}"),
                })?);
            }
            Ok(entries)
        })
        .await
        .map_err(|e| RunnerError::ExecutionFailed {
            node_id: "sqlite_backend".into(),
            message: format!("spawn_blocking join error: {e}"),
        })?
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a rusqlite Row into a LongTermEntry.
fn row_to_entry(row: &rusqlite::Row<'_>) -> LongTermEntry {
    let tags_json: String = row.get(3).unwrap_or_else(|_| "[]".into());
    let metadata_json: String = row.get(6).unwrap_or_else(|_| "{}".into());

    LongTermEntry {
        id: row.get(0).unwrap_or_default(),
        entry_type: row.get(1).unwrap_or_default(),
        content: row.get(2).unwrap_or_default(),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        session_id: row.get(4).unwrap_or(None),
        created_at: row.get(5).unwrap_or(0.0),
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
    }
}

/// Current wall-clock as fractional seconds (UTC).
fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    fn entry_with_type(content: &str, entry_type: &str) -> LongTermEntry {
        LongTermEntry {
            id: String::new(),
            entry_type: entry_type.into(),
            content: content.into(),
            tags: vec!["test".into()],
            session_id: Some("sess-1".into()),
            created_at: 0.0,
            metadata: HashMap::new(),
        }
    }

    // 1. Save and get entry
    #[tokio::test]
    async fn save_and_get_entry() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        let e = entry_with_type("always retry on 429", "learning");
        let id = backend.save(&e).await.unwrap();

        let fetched = backend.get(&id).await.unwrap().expect("entry must exist");
        assert_eq!(fetched.content, "always retry on 429");
        assert_eq!(fetched.entry_type, "learning");
        assert_eq!(fetched.id, id);
    }

    // 2. Search by content (FTS)
    #[tokio::test]
    async fn search_by_content_fts() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        backend.save(&entry("retry on HTTP 429 errors")).await.unwrap();
        backend.save(&entry("use exponential backoff")).await.unwrap();
        backend.save(&entry("cache responses locally")).await.unwrap();

        let results = backend.search("retry", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("429"));
    }

    // 3. Search is case-insensitive
    #[tokio::test]
    async fn search_is_case_insensitive() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        backend.save(&entry("UPPERCASE content here")).await.unwrap();
        backend.save(&entry("lowercase content here")).await.unwrap();
        backend.save(&entry("no match whatsoever")).await.unwrap();

        let results = backend.search("content", 10).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    // 4. Delete removes entry
    #[tokio::test]
    async fn delete_removes_entry() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        let id = backend.save(&entry("temporary")).await.unwrap();
        assert!(backend.get(&id).await.unwrap().is_some());

        backend.delete(&id).await.unwrap();
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    // 5. List recent respects limit and ordering
    #[tokio::test]
    async fn list_recent_respects_limit_and_ordering() {
        let backend = SqliteBackend::new_in_memory().unwrap();

        // Insert with explicit timestamps so order is deterministic
        let mut e1 = entry("old entry");
        e1.created_at = 1000.0;
        let mut e2 = entry("middle entry");
        e2.created_at = 2000.0;
        let mut e3 = entry("new entry");
        e3.created_at = 3000.0;

        backend.save(&e1).await.unwrap();
        backend.save(&e2).await.unwrap();
        backend.save(&e3).await.unwrap();

        let recent = backend.list_recent(2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "new entry");
        assert_eq!(recent[1].content, "middle entry");
    }

    // 6. Empty search returns empty
    #[tokio::test]
    async fn empty_search_returns_empty() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        backend.save(&entry("some content")).await.unwrap();

        let results = backend.search("", 10).await.unwrap();
        assert!(results.is_empty());
    }

    // 7. Get non-existent returns None
    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let backend = SqliteBackend::new_in_memory().unwrap();
        let result = backend.get("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }
}
