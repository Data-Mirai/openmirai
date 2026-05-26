//! LocalStorageResource -- filesystem-backed StorageResource.
//!
//! All operations are relative to a `base_path`. Path traversal is blocked
//! by canonicalizing the resolved path and verifying it stays within the
//! base directory (REGLA-505).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::core::context::{ResourceError, StorageResource};

/// Filesystem-backed object storage.
pub struct LocalStorageResource {
    base_path: PathBuf,
}

impl LocalStorageResource {
    /// Create a new storage resource rooted at `base_path`.
    ///
    /// Creates the directory if it doesn't exist.
    pub fn new(base_path: impl Into<PathBuf>) -> Result<Self, ResourceError> {
        let base_path = base_path.into();
        std::fs::create_dir_all(&base_path).map_err(|e| {
            ResourceError::Storage(format!(
                "cannot create base directory {}: {e}",
                base_path.display()
            ))
        })?;

        // Canonicalize so we have a stable base for traversal checks
        let base_path = std::fs::canonicalize(&base_path).map_err(|e| {
            ResourceError::Storage(format!(
                "cannot canonicalize {}: {e}",
                base_path.display()
            ))
        })?;

        Ok(Self { base_path })
    }

    /// Resolve a key to an absolute path, blocking path traversal.
    fn resolve(&self, key: &str) -> Result<PathBuf, ResourceError> {
        let joined = self.base_path.join(key);

        // For existing paths, canonicalize and check prefix
        if joined.exists() {
            let canonical = std::fs::canonicalize(&joined).map_err(|e| {
                ResourceError::Storage(format!("path resolution failed: {e}"))
            })?;
            if !canonical.starts_with(&self.base_path) {
                return Err(ResourceError::Storage(
                    "path traversal blocked: resolved path is outside base_path".into(),
                ));
            }
            return Ok(canonical);
        }

        // For new paths, verify the normalized path doesn't escape
        // by checking that no component is ".."
        for component in Path::new(key).components() {
            if let std::path::Component::ParentDir = component {
                return Err(ResourceError::Storage(
                    "path traversal blocked: '..' not allowed in key".into(),
                ));
            }
        }

        Ok(joined)
    }
}

#[async_trait]
impl StorageResource for LocalStorageResource {
    async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError> {
        let resolved = self.resolve(path)?;
        let resolved_clone = resolved.clone();

        tokio::task::spawn_blocking(move || {
            std::fs::read(&resolved_clone).map_err(|e| {
                ResourceError::NotFound(format!("{}: {e}", resolved_clone.display()))
            })
        })
        .await
        .map_err(|e| ResourceError::Storage(format!("spawn_blocking join: {e}")))?
    }

    async fn put(&self, path: &str, data: &[u8]) -> Result<(), ResourceError> {
        let resolved = self.resolve(path)?;
        let data = data.to_vec();

        tokio::task::spawn_blocking(move || {
            if let Some(parent) = resolved.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    ResourceError::Storage(format!("cannot create parent dirs: {e}"))
                })?;
            }
            std::fs::write(&resolved, &data)
                .map_err(|e| ResourceError::Storage(format!("write failed: {e}")))
        })
        .await
        .map_err(|e| ResourceError::Storage(format!("spawn_blocking join: {e}")))?
    }

    async fn delete(&self, path: &str) -> Result<(), ResourceError> {
        let resolved = self.resolve(path)?;

        tokio::task::spawn_blocking(move || {
            if resolved.exists() {
                std::fs::remove_file(&resolved)
                    .map_err(|e| ResourceError::Storage(format!("delete failed: {e}")))?;
            }
            Ok(())
        })
        .await
        .map_err(|e| ResourceError::Storage(format!("spawn_blocking join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        storage.put("test.txt", b"hello world").await.unwrap();
        let data = storage.get("test.txt").await.unwrap();
        assert_eq!(data, b"hello world");
    }

    #[tokio::test]
    async fn put_creates_subdirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        storage.put("a/b/c/deep.txt", b"deep").await.unwrap();
        let data = storage.get("a/b/c/deep.txt").await.unwrap();
        assert_eq!(data, b"deep");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        let result = storage.get("nope.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        storage.put("rm_me.txt", b"bye").await.unwrap();
        storage.delete("rm_me.txt").await.unwrap();
        let result = storage.get("rm_me.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn path_traversal_blocked() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        let result = storage.put("../../etc/passwd", b"evil").await;
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("traversal"));
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = LocalStorageResource::new(dir.path()).unwrap();

        // Should not error
        storage.delete("nonexistent.txt").await.unwrap();
    }
}
