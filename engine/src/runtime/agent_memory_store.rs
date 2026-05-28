//! Per-agent persistent memory store (PRD-008).
//!
//! Stores key-value data that persists between agent cycles (live) and
//! executions (managed), depending on the configured `MemoryPersistMode`.
//! V1: in-memory only — resets on server restart.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tracing::warn;

// ---------------------------------------------------------------------------
// AgentMemoryStore — per-agent KV store
// ---------------------------------------------------------------------------

/// In-memory store for agent memory, keyed by agent_id.
///
/// Each agent's memory is a `HashMap<String, Value>` of declared keys.
/// Cycle memory (volatile, within a play session) is separate from
/// execution memory (persistent across sessions).
#[derive(Debug, Clone)]
pub struct AgentMemoryStore {
    /// Execution-level memory: persists across sessions. Keyed by agent_id.
    execution_store: Arc<RwLock<HashMap<String, HashMap<String, Value>>>>,
    /// Cycle-level memory: volatile, within a play session. Keyed by agent_id.
    cycle_store: Arc<RwLock<HashMap<String, HashMap<String, Value>>>>,
}

impl AgentMemoryStore {
    pub fn new() -> Self {
        Self {
            execution_store: Arc::new(RwLock::new(HashMap::new())),
            cycle_store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // --- Execution memory (persist: execution) ---

    /// Read all execution-level memory for an agent.
    pub async fn get_execution_memory(&self, agent_id: &str) -> HashMap<String, Value> {
        let store = self.execution_store.read().await;
        store.get(agent_id).cloned().unwrap_or_default()
    }

    /// Write execution-level memory, filtering to only declared keys.
    pub async fn set_execution_memory(
        &self,
        agent_id: &str,
        data: HashMap<String, Value>,
        declared_keys: &HashMap<String, Value>,
    ) {
        let filtered = filter_to_declared(data, declared_keys);
        let mut store = self.execution_store.write().await;
        store.insert(agent_id.to_string(), filtered);
    }

    /// Clear execution memory for an agent (reset to initial values).
    pub async fn clear_execution_memory(
        &self,
        agent_id: &str,
        initial_values: &HashMap<String, Value>,
    ) {
        let mut store = self.execution_store.write().await;
        store.insert(agent_id.to_string(), initial_values.clone());
    }

    // --- Cycle memory (persist: cycle) ---

    /// Read cycle-level memory for an agent (within current play session).
    pub async fn get_cycle_memory(&self, agent_id: &str) -> HashMap<String, Value> {
        let store = self.cycle_store.read().await;
        store.get(agent_id).cloned().unwrap_or_default()
    }

    /// Write cycle-level memory, filtering to only declared keys.
    pub async fn set_cycle_memory(
        &self,
        agent_id: &str,
        data: HashMap<String, Value>,
        declared_keys: &HashMap<String, Value>,
    ) {
        let filtered = filter_to_declared(data, declared_keys);
        let mut store = self.cycle_store.write().await;
        store.insert(agent_id.to_string(), filtered);
    }

    /// Reset cycle memory for an agent (called on stop→play).
    pub async fn clear_cycle_memory(&self, agent_id: &str) {
        let mut store = self.cycle_store.write().await;
        store.remove(agent_id);
    }

    // --- Generic read (for API) ---

    /// Get current memory state for an agent (reads from appropriate store).
    /// Returns execution memory if it exists, else cycle memory, else empty.
    pub async fn get_all_memory(&self, agent_id: &str) -> HashMap<String, Value> {
        let exec = self.get_execution_memory(agent_id).await;
        if !exec.is_empty() {
            return exec;
        }
        self.get_cycle_memory(agent_id).await
    }

    /// Clear ALL memory for an agent (both execution and cycle).
    pub async fn clear_all_memory(
        &self,
        agent_id: &str,
        initial_values: Option<&HashMap<String, Value>>,
    ) {
        self.clear_cycle_memory(agent_id).await;
        if let Some(initials) = initial_values {
            self.clear_execution_memory(agent_id, initials).await;
        } else {
            let mut store = self.execution_store.write().await;
            store.remove(agent_id);
        }
    }
}

impl Default for AgentMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Filter data to only include keys declared in the memory spec.
/// Undeclared keys are logged as warnings and dropped.
fn filter_to_declared(
    data: HashMap<String, Value>,
    declared_keys: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    let mut filtered = HashMap::new();
    for (key, value) in data {
        if declared_keys.contains_key(&key) {
            filtered.insert(key, value);
        } else {
            warn!(key = %key, "state/memory: ignoring undeclared key");
        }
    }
    filtered
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declared() -> HashMap<String, Value> {
        let mut m = HashMap::new();
        m.insert("count".to_string(), json!(0));
        m.insert("data".to_string(), json!(null));
        m
    }

    #[tokio::test]
    async fn execution_memory_roundtrip() {
        let store = AgentMemoryStore::new();
        let dk = declared();

        // Initially empty
        let mem = store.get_execution_memory("a1").await;
        assert!(mem.is_empty());

        // Write
        let mut data = HashMap::new();
        data.insert("count".to_string(), json!(5));
        data.insert("data".to_string(), json!("hello"));
        store.set_execution_memory("a1", data, &dk).await;

        // Read back
        let mem = store.get_execution_memory("a1").await;
        assert_eq!(mem["count"], json!(5));
        assert_eq!(mem["data"], json!("hello"));
    }

    #[tokio::test]
    async fn undeclared_keys_filtered() {
        let store = AgentMemoryStore::new();
        let dk = declared();

        let mut data = HashMap::new();
        data.insert("count".to_string(), json!(1));
        data.insert("extra".to_string(), json!("should be dropped"));
        store.set_execution_memory("a1", data, &dk).await;

        let mem = store.get_execution_memory("a1").await;
        assert_eq!(mem.len(), 1);
        assert!(!mem.contains_key("extra"));
    }

    #[tokio::test]
    async fn cycle_memory_independent_from_execution() {
        let store = AgentMemoryStore::new();
        let dk = declared();

        let mut exec_data = HashMap::new();
        exec_data.insert("count".to_string(), json!(10));
        store.set_execution_memory("a1", exec_data, &dk).await;

        let mut cycle_data = HashMap::new();
        cycle_data.insert("count".to_string(), json!(3));
        store.set_cycle_memory("a1", cycle_data, &dk).await;

        assert_eq!(store.get_execution_memory("a1").await["count"], json!(10));
        assert_eq!(store.get_cycle_memory("a1").await["count"], json!(3));
    }

    #[tokio::test]
    async fn clear_cycle_memory_resets() {
        let store = AgentMemoryStore::new();
        let dk = declared();

        let mut data = HashMap::new();
        data.insert("count".to_string(), json!(5));
        store.set_cycle_memory("a1", data, &dk).await;

        store.clear_cycle_memory("a1").await;
        assert!(store.get_cycle_memory("a1").await.is_empty());
    }

    #[tokio::test]
    async fn clear_all_memory_resets_to_initials() {
        let store = AgentMemoryStore::new();
        let dk = declared();

        let mut data = HashMap::new();
        data.insert("count".to_string(), json!(99));
        store.set_execution_memory("a1", data.clone(), &dk).await;
        store.set_cycle_memory("a1", data, &dk).await;

        store.clear_all_memory("a1", Some(&dk)).await;

        let exec = store.get_execution_memory("a1").await;
        assert_eq!(exec["count"], json!(0));
        assert!(store.get_cycle_memory("a1").await.is_empty());
    }
}
