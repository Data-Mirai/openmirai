//! MemoryFactory -- creates the appropriate MemoryBackend based on config.
//!
//! If a `db_path` is provided, returns a persistent [`SqliteBackend`].
//! Otherwise returns an [`InMemoryBackend`] (suitable for dev/testing).

use crate::core::RunnerError;

use super::in_memory_backend::InMemoryBackend;
use super::long_term::MemoryBackend;
use super::sqlite_backend::SqliteBackend;

/// Factory for creating [`MemoryBackend`] instances.
pub struct MemoryFactory;

impl MemoryFactory {
    /// Create a memory backend from optional config.
    ///
    /// - `Some(path)` → persistent SQLite backend at the given path.
    /// - `None` → in-memory backend (no persistence).
    pub fn create(db_path: Option<&str>) -> Result<Box<dyn MemoryBackend>, RunnerError> {
        match db_path {
            Some(path) => Ok(Box::new(SqliteBackend::new(path)?)),
            None => Ok(Box::new(InMemoryBackend::new())),
        }
    }

    /// Convenience: always returns an in-memory backend.
    pub fn default() -> Box<dyn MemoryBackend> {
        Box::new(InMemoryBackend::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::long_term::LongTermEntry;
    use std::collections::HashMap;

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

    #[test]
    fn default_returns_in_memory() {
        let _backend = MemoryFactory::default();
        // If it doesn't panic, it works.
    }

    #[tokio::test]
    async fn create_none_returns_in_memory() {
        let backend = MemoryFactory::create(None).unwrap();
        let id = backend.save(&entry("test")).await.unwrap();
        assert!(!id.is_empty());
        let fetched = backend.get(&id).await.unwrap();
        assert!(fetched.is_some());
    }

    #[tokio::test]
    async fn create_sqlite_path_works() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let backend = MemoryFactory::create(Some(db_path.to_str().unwrap())).unwrap();

        let id = backend.save(&entry("persistent")).await.unwrap();
        let fetched = backend.get(&id).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().content, "persistent");
    }
}
