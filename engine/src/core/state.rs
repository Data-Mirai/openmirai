use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Backward-compat alias.
pub type SharedState = ExecutionState;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("output already set for node `{0}` (use overwrite=true to replace)")]
    AlreadySet(String),

    #[error("lock poisoned: {0}")]
    LockPoisoned(String),
}

// ---------------------------------------------------------------------------
// ExecutionState
// ---------------------------------------------------------------------------

/// Thread-safe execution state container.
///
/// Each node writes its output exactly once (unless overwrite is enabled).
/// Multiple readers can access state concurrently via `Arc<RwLock<...>>`.
#[derive(Debug, Clone)]
pub struct ExecutionState {
    inner: Arc<RwLock<HashMap<String, HashMap<String, serde_json::Value>>>>,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store the output for a node. Fails if already set unless `overwrite` is
    /// true.
    pub fn set(
        &self,
        node_id: &str,
        output: HashMap<String, serde_json::Value>,
        overwrite: bool,
    ) -> Result<(), StateError> {
        let mut map = self
            .inner
            .write()
            .map_err(|e| StateError::LockPoisoned(e.to_string()))?;

        if !overwrite && map.contains_key(node_id) {
            return Err(StateError::AlreadySet(node_id.to_string()));
        }

        map.insert(node_id.to_string(), output);
        Ok(())
    }

    /// Get the full output map for a node (cloned).
    pub fn get(&self, node_id: &str) -> Option<HashMap<String, serde_json::Value>> {
        let map = self.inner.read().ok()?;
        map.get(node_id).cloned()
    }

    /// Acquire a read guard for the entire state.
    ///
    /// Useful when you need to read multiple fields without cloning.
    /// The guard holds the read lock — drop it when done.
    pub fn read(
        &self,
    ) -> Option<RwLockReadGuard<'_, HashMap<String, HashMap<String, serde_json::Value>>>> {
        self.inner.read().ok()
    }

    /// Get a single field from a node's output.
    ///
    /// Supports nested traversal via dot-separated paths:
    /// - `get_field("trigger", "payload")` → direct key lookup
    /// - `get_field("trigger", "payload.question")` → traverses into nested JSON
    /// - `get_field("n1", "a.b.c.d")` → N levels deep, returns None if any level missing
    pub fn get_field(&self, node_id: &str, field: &str) -> Option<serde_json::Value> {
        let map = self.inner.read().ok()?;
        let node_output = map.get(node_id)?;

        let parts: Vec<&str> = field.split('.').collect();
        if parts.is_empty() {
            return None;
        }

        let mut current = node_output.get(parts[0])?;
        for part in &parts[1..] {
            current = current.get(part)?;
        }

        Some(current.clone())
    }

    /// Deep-clone the entire state into a plain HashMap.
    pub fn snapshot(&self) -> HashMap<String, HashMap<String, serde_json::Value>> {
        self.inner.read().map(|m| m.clone()).unwrap_or_default()
    }

    /// Returns true when no node outputs have been recorded.
    pub fn is_empty(&self) -> bool {
        self.inner.read().map(|m| m.is_empty()).unwrap_or(true)
    }

    /// Create a deep clone (fork) of this execution state.
    /// Modifying the fork will not affect the parent/other forks.
    pub fn fork(&self) -> Self {
        let map = self.snapshot();
        Self {
            inner: Arc::new(RwLock::new(map)),
        }
    }

    /// Merges another ExecutionState into this one.
    /// In case of overlapping keys, the conflict resolution strategy is applied.
    pub fn merge(&self, other: &Self) -> Result<(), StateError> {
        let mut this_map = self
            .inner
            .write()
            .map_err(|e| StateError::LockPoisoned(e.to_string()))?;
        let other_map = other
            .inner
            .read()
            .map_err(|e| StateError::LockPoisoned(e.to_string()))?;

        for (node_id, other_output) in other_map.iter() {
            if let Some(this_output) = this_map.get_mut(node_id) {
                // Merge nested maps
                for (k, v) in other_output {
                    this_output.insert(k.clone(), v.clone());
                }
            } else {
                this_map.insert(node_id.clone(), other_output.clone());
            }
        }
        Ok(())
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

// -- Serde: serialize as the plain HashMap snapshot, deserialize back. ------

impl Serialize for ExecutionState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let map = self
            .inner
            .read()
            .map_err(|e| serde::ser::Error::custom(e.to_string()))?;
        let mut ser_map = serializer.serialize_map(Some(map.len()))?;
        for (k, v) in map.iter() {
            ser_map.serialize_entry(k, v)?;
        }
        ser_map.end()
    }
}

impl<'de> Deserialize<'de> for ExecutionState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = HashMap::<String, HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
        })
    }
}

// Compile-time assertions that ExecutionState is Send + Sync.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<ExecutionState>();
    assert_sync::<ExecutionState>();
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn output(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn new_state_is_empty() {
        let s = ExecutionState::new();
        assert!(s.is_empty());
    }

    #[test]
    fn set_and_get() {
        let s = ExecutionState::new();
        let data = output(&[("text", json!("hello"))]);
        s.set("n1", data.clone(), false).unwrap();

        assert!(!s.is_empty());
        assert_eq!(s.get("n1").unwrap(), data);
    }

    #[test]
    fn get_field_returns_value() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("score", json!(42))]), false).unwrap();
        assert_eq!(s.get_field("n1", "score"), Some(json!(42)));
    }

    #[test]
    fn get_field_missing_node() {
        let s = ExecutionState::new();
        assert_eq!(s.get_field("ghost", "x"), None);
    }

    #[test]
    fn get_field_missing_field() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("a", json!(1))]), false).unwrap();
        assert_eq!(s.get_field("n1", "b"), None);
    }

    #[test]
    fn write_once_semantics() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("v", json!(1))]), false).unwrap();
        let err = s.set("n1", output(&[("v", json!(2))]), false).unwrap_err();
        assert!(matches!(err, StateError::AlreadySet(_)));
        // Value unchanged
        assert_eq!(s.get_field("n1", "v"), Some(json!(1)));
    }

    #[test]
    fn overwrite_allowed() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("v", json!(1))]), false).unwrap();
        s.set("n1", output(&[("v", json!(2))]), true).unwrap();
        assert_eq!(s.get_field("n1", "v"), Some(json!(2)));
    }

    #[test]
    fn snapshot_is_independent() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("x", json!(1))]), false).unwrap();
        let snap = s.snapshot();

        // Mutate original
        s.set("n2", output(&[("y", json!(2))]), false).unwrap();

        // Snapshot unchanged
        assert_eq!(snap.len(), 1);
        assert!(snap.contains_key("n1"));
    }

    #[test]
    fn clone_shares_arc() {
        let s1 = ExecutionState::new();
        let s2 = s1.clone();

        s1.set("n1", output(&[("a", json!(1))]), false).unwrap();
        assert_eq!(s2.get_field("n1", "a"), Some(json!(1)));
    }

    // --- PRD-004: Nested field traversal ---

    #[test]
    fn get_field_nested_traversal() {
        let s = ExecutionState::new();
        s.set(
            "trigger",
            output(&[(
                "payload",
                json!({"question": "hola", "context": "sobre IA"}),
            )]),
            false,
        )
        .unwrap();

        // Nested: trigger.payload.question
        assert_eq!(
            s.get_field("trigger", "payload.question"),
            Some(json!("hola"))
        );
        assert_eq!(
            s.get_field("trigger", "payload.context"),
            Some(json!("sobre IA"))
        );
        // Direct (no dots) still works
        assert_eq!(
            s.get_field("trigger", "payload"),
            Some(json!({"question": "hola", "context": "sobre IA"}))
        );
    }

    #[test]
    fn get_field_deep_nesting() {
        let s = ExecutionState::new();
        s.set(
            "n1",
            output(&[("a", json!({"b": {"c": {"d": "deep"}}}))]),
            false,
        )
        .unwrap();
        assert_eq!(s.get_field("n1", "a.b.c.d"), Some(json!("deep")));
        assert_eq!(s.get_field("n1", "a.b.c"), Some(json!({"d": "deep"})));
        assert_eq!(s.get_field("n1", "a.b"), Some(json!({"c": {"d": "deep"}})));
    }

    #[test]
    fn get_field_nested_missing_returns_none() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("data", json!("just a string"))]), false)
            .unwrap();
        // Path goes through a non-object → None, no panic
        assert_eq!(s.get_field("n1", "data.subfield"), None);
    }

    #[test]
    fn get_field_nested_nonexistent_intermediate() {
        let s = ExecutionState::new();
        s.set("n1", output(&[("a", json!({"b": 42}))]), false)
            .unwrap();
        // b is a number, can't traverse into it
        assert_eq!(s.get_field("n1", "a.b.c"), None);
        // x doesn't exist at all
        assert_eq!(s.get_field("n1", "a.x.y"), None);
    }

    #[test]
    fn concurrent_reads() {
        use std::thread;

        let s = ExecutionState::new();
        s.set("n1", output(&[("v", json!(42))]), false).unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let state = s.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        assert_eq!(state.get_field("n1", "v"), Some(json!(42)));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_fork_isolated() {
        let parent = ExecutionState::new();
        parent.set("n1", output(&[("v", json!(10))]), false).unwrap();

        let child = parent.fork();
        // Child should have parent's values
        assert_eq!(child.get_field("n1", "v"), Some(json!(10)));

        // Modifying child should not affect parent
        child.set("n2", output(&[("v", json!(20))]), false).unwrap();
        assert_eq!(child.get_field("n2", "v"), Some(json!(20)));
        assert_eq!(parent.get_field("n2", "v"), None);

        // Modifying parent should not affect child
        parent.set("n1", output(&[("v", json!(15))]), true).unwrap();
        assert_eq!(parent.get_field("n1", "v"), Some(json!(15)));
        assert_eq!(child.get_field("n1", "v"), Some(json!(10)));
    }

    #[test]
    fn test_merge_states() {
        let state_a = ExecutionState::new();
        state_a.set("n1", output(&[("x", json!(1))]), false).unwrap();

        let state_b = ExecutionState::new();
        state_b.set("n2", output(&[("y", json!(2))]), false).unwrap();
        // Overlap on n1, different keys
        state_b.set("n1", output(&[("z", json!(3))]), false).unwrap();

        state_a.merge(&state_b).unwrap();

        // Check merged keys
        assert_eq!(state_a.get_field("n2", "y"), Some(json!(2)));
        assert_eq!(state_a.get_field("n1", "x"), Some(json!(1)));
        assert_eq!(state_a.get_field("n1", "z"), Some(json!(3)));
    }
}

