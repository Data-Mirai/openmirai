//! State tools — persistent memory between agent cycles/executions (PRD-008).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::warn;

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// MemoryTool — persists declared keys from data_map
// ---------------------------------------------------------------------------

/// The `state/memory` tool receives inputs via data_map and persists them
/// to the agent's memory store. It only writes keys declared in graph.memory.
///
/// The tool receives `__memory_spec` and `__agent_id` via config injection
/// (done by run_agent_spec). It uses these to validate which keys are
/// declared and to determine the persist mode.
///
/// For `persist: none`, this tool is a no-op.
pub struct MemoryTool;

pub struct MemoryToolFactory {
    spec: ToolSpec,
}

impl Default for MemoryToolFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryToolFactory {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                tool_type: "state/memory".into(),
                name: "Memory".into(),
                description: "Persist agent memory between cycles/executions. Only writes keys declared in graph.memory.".into(),
                version: "1.0.0".into(),
                category: "state".into(),
                inputs: vec![],
                outputs: vec![
                    field("persisted_keys", FieldType::Array, true, "List of keys that were persisted"),
                    field("persist_mode", FieldType::String, true, "The persist mode used (none, cycle, execution)"),
                ],
                config_fields: vec![],
            },
        }
    }
}

impl ToolFactory for MemoryToolFactory {
    fn create(&self) -> Arc<dyn Tool> {
        Arc::new(MemoryTool)
    }
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
}

#[async_trait]
impl Tool for MemoryTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // Read injected memory spec from config
        let mem_spec = config.get("__memory_spec");

        // Determine persist mode
        let persist_mode = mem_spec
            .and_then(|m| m.get("persist"))
            .and_then(|v| v.as_str())
            .unwrap_or("cycle");

        // Get declared keys from spec
        let declared_keys: HashMap<String, Value> = mem_spec
            .and_then(|m| m.get("keys"))
            .and_then(|k| serde_json::from_value(k.clone()).ok())
            .unwrap_or_default();

        let mut persisted_keys = Vec::new();

        if persist_mode == "none" {
            // No-op for persist: none
            let mut out = HashMap::new();
            out.insert("persisted_keys".to_string(), json!([]));
            out.insert("persist_mode".to_string(), json!("none"));
            return Ok(out);
        }

        // Filter inputs to only declared keys, store in output for the
        // post-execution persistence layer to read from state.
        // The actual persistence happens externally (server/scheduler reads
        // state["memory_write"] after execution completes).
        let mut memory_write = HashMap::new();
        for (key, value) in &inputs {
            if declared_keys.contains_key(key) {
                memory_write.insert(key.clone(), value.clone());
                persisted_keys.push(Value::String(key.clone()));
            } else {
                warn!(key = %key, "state/memory: ignoring undeclared key");
            }
        }

        let mut out = HashMap::new();
        out.insert("persisted_keys".to_string(), Value::Array(persisted_keys));
        out.insert("persist_mode".to_string(), json!(persist_mode));
        // Store the filtered data as __memory_write for post-execution pickup
        out.insert(
            "__memory_write".to_string(),
            serde_json::to_value(&memory_write).unwrap_or_default(),
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_state_tools(registry: &mut ToolRegistry) {
    registry.register("state/memory", Box::new(MemoryToolFactory::new()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::InMemoryContext;

    fn ctx() -> InMemoryContext {
        InMemoryContext::new("test-run")
    }

    fn make_config(persist: &str, keys: &[&str]) -> HashMap<String, Value> {
        let mut declared = HashMap::new();
        for k in keys {
            declared.insert(k.to_string(), json!(null));
        }
        let spec = json!({
            "persist": persist,
            "keys": declared,
        });
        let mut config = HashMap::new();
        config.insert("__memory_spec".to_string(), spec);
        config.insert("__agent_id".to_string(), json!("test-agent"));
        config
    }

    #[tokio::test]
    async fn persist_none_is_noop() {
        let tool = MemoryTool;
        let config = make_config("none", &["count"]);
        let mut inputs = HashMap::new();
        inputs.insert("count".to_string(), json!(42));

        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        assert_eq!(result["persist_mode"], json!("none"));
        assert_eq!(result["persisted_keys"], json!([]));
    }

    #[tokio::test]
    async fn persist_cycle_filters_declared_keys() {
        let tool = MemoryTool;
        let config = make_config("cycle", &["count", "data"]);
        let mut inputs = HashMap::new();
        inputs.insert("count".to_string(), json!(5));
        inputs.insert("extra".to_string(), json!("should be dropped"));

        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        assert_eq!(result["persist_mode"], json!("cycle"));

        let keys: Vec<String> = result["persisted_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(keys.contains(&"count".to_string()));
        assert!(!keys.contains(&"extra".to_string()));

        let write_data: HashMap<String, Value> =
            serde_json::from_value(result["__memory_write"].clone()).unwrap();
        assert_eq!(write_data["count"], json!(5));
        assert!(!write_data.contains_key("extra"));
    }

    #[tokio::test]
    async fn persist_execution_writes_data() {
        let tool = MemoryTool;
        let config = make_config("execution", &["total"]);
        let mut inputs = HashMap::new();
        inputs.insert("total".to_string(), json!(100));

        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        assert_eq!(result["persist_mode"], json!("execution"));

        let write_data: HashMap<String, Value> =
            serde_json::from_value(result["__memory_write"].clone()).unwrap();
        assert_eq!(write_data["total"], json!(100));
    }

    #[test]
    fn register_state_tools_adds_one() {
        let mut reg = ToolRegistry::new();
        register_state_tools(&mut reg);
        assert!(reg.get("state/memory").is_some());
    }
}
