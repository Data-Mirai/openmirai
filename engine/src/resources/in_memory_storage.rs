//! InMemoryStorageResource — implements `StorageResource` with an in-memory
//! `HashMap<String, Vec<u8>>`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::core::context::{ResourceError, StorageResource};

// ---------------------------------------------------------------------------
// InMemoryStorageResource
// ---------------------------------------------------------------------------

/// In-memory object storage for dev/testing.
#[derive(Debug)]
pub struct InMemoryStorageResource {
    files: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryStorageResource {
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// List all keys with the given prefix.
    pub async fn list_keys(&self, prefix: &str) -> Vec<String> {
        let files = self.files.read().await;
        files
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
}

impl Default for InMemoryStorageResource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StorageResource for InMemoryStorageResource {
    async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError> {
        let files = self.files.read().await;
        files
            .get(path)
            .cloned()
            .ok_or_else(|| ResourceError::NotFound(format!("object '{}' not found", path)))
    }

    async fn put(&self, path: &str, data: &[u8]) -> Result<(), ResourceError> {
        let mut files = self.files.write().await;
        files.insert(path.to_string(), data.to_vec());
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), ResourceError> {
        let mut files = self.files.write().await;
        files.remove(path);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let store = InMemoryStorageResource::new();
        store.put("a/b.txt", b"hello").await.unwrap();

        let data = store.get("a/b.txt").await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn get_missing_returns_error() {
        let store = InMemoryStorageResource::new();
        let err = store.get("nope").await;
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("not found"));
    }

    #[tokio::test]
    async fn delete_removes() {
        let store = InMemoryStorageResource::new();
        store.put("x", b"data").await.unwrap();
        store.delete("x").await.unwrap();
        assert!(store.get("x").await.is_err());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_noop() {
        let store = InMemoryStorageResource::new();
        store.delete("ghost").await.unwrap();
    }

    #[tokio::test]
    async fn put_overwrites() {
        let store = InMemoryStorageResource::new();
        store.put("k", b"old").await.unwrap();
        store.put("k", b"new").await.unwrap();
        assert_eq!(store.get("k").await.unwrap(), b"new");
    }

    #[tokio::test]
    async fn list_keys_with_prefix() {
        let store = InMemoryStorageResource::new();
        store.put("images/a.png", b"a").await.unwrap();
        store.put("images/b.png", b"b").await.unwrap();
        store.put("docs/c.txt", b"c").await.unwrap();

        let mut keys = store.list_keys("images/").await;
        keys.sort();
        assert_eq!(keys, vec!["images/a.png", "images/b.png"]);

        let all = store.list_keys("").await;
        assert_eq!(all.len(), 3);
    }
}
