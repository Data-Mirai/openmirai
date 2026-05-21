use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: &str, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type: field_type.into(),
        required,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.into())
        },
        default: None,
    }
}

// ---------------------------------------------------------------------------
// Macro
// ---------------------------------------------------------------------------

macro_rules! agent_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: "agent".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Box<dyn Tool> {
                Box::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_NESTING_DEPTH: usize = 3;

// ===========================================================================
// RunAgentTool
// ===========================================================================

agent_tool! {
    struct RunAgentTool, factory RunAgentFactory;
    tool_type = "agent/run_agent",
    name = "Run Agent",
    description = "Executes another agent as a synchronous sub-task. Max nesting depth = 3.",
    inputs = [
        field("agent_id", "string", false, "ID of the agent to execute (can come from config)"),
        field("input_data", "object", false, "Input data for the sub-agent"),
    ],
    outputs = [
        field("agent_id", "string", true, "ID of the executed agent"),
        field("session_id", "string", true, "Session ID of the child execution"),
        field("status", "string", true, "Execution status"),
        field("result", "object", true, "Sub-agent result"),
        field("duration_ms", "number", true, "Total duration in ms"),
    ],
    config_fields = [
        field("agent_id", "string", false, "ID of the agent to execute"),
        field("timeout_seconds", "number", false, "Timeout for sub-agent execution (default 300)"),
    ]
}

#[async_trait]
impl Tool for RunAgentTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // Resolve agent_id: input takes priority, fallback to config
        let agent_id = inputs
            .get("agent_id")
            .and_then(|v| v.as_str())
            .or_else(|| config.get("agent_id").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "agent/run_agent".into(),
                message: "agent_id is required (via input or config)".into(),
            })?;

        let input_data = inputs.get("input_data").cloned().unwrap_or(json!({}));

        // Check nesting depth from input metadata (injected by app layer)
        let call_chain: Vec<String> = inputs
            .get("__agent_call_chain__")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if call_chain.len() >= MAX_NESTING_DEPTH {
            return Err(ToolError::ExecutionFailed {
                tool_type: "agent/run_agent".into(),
                message: format!(
                    "Sub-agent nesting depth exceeded (max={}). Chain: {} -> {}",
                    MAX_NESTING_DEPTH,
                    call_chain.join(" -> "),
                    agent_id
                ),
            });
        }

        // Check for circular references
        if call_chain.contains(&agent_id.to_string()) {
            return Err(ToolError::ExecutionFailed {
                tool_type: "agent/run_agent".into(),
                message: format!(
                    "Circular agent reference detected: {} -> {}",
                    call_chain.join(" -> "),
                    agent_id
                ),
            });
        }

        // Placeholder: the real execution happens in the app layer.
        let mut new_chain = call_chain;
        new_chain.push(agent_id.to_string());

        let mut out = HashMap::new();
        out.insert("agent_id".to_string(), json!(agent_id));
        out.insert("session_id".to_string(), json!(""));
        out.insert("status".to_string(), json!("placeholder"));
        out.insert("result".to_string(), json!({}));
        out.insert("duration_ms".to_string(), json!(0));
        out.insert("_input_data".to_string(), input_data);
        out.insert("_call_chain".to_string(), json!(new_chain));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all agent tools into the given registry.
pub fn register_agent_tools(registry: &mut ToolRegistry) {
    registry.register("agent/run_agent", Box::new(RunAgentFactory::new()));
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

    #[tokio::test]
    async fn run_agent_placeholder() {
        let tool = RunAgentTool;
        let mut inputs = HashMap::new();
        inputs.insert("agent_id".to_string(), json!("agent-123"));
        let result = tool
            .execute(inputs, HashMap::new(), &ctx())
            .await
            .unwrap();
        assert_eq!(result["agent_id"], json!("agent-123"));
        assert_eq!(result["status"], json!("placeholder"));
    }

    #[tokio::test]
    async fn run_agent_missing_id_fails() {
        let tool = RunAgentTool;
        let result = tool
            .execute(HashMap::new(), HashMap::new(), &ctx())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_agent_nesting_depth_exceeded() {
        let tool = RunAgentTool;
        let mut inputs = HashMap::new();
        inputs.insert("agent_id".to_string(), json!("agent-d"));
        inputs.insert(
            "__agent_call_chain__".to_string(),
            json!(["agent-a", "agent-b", "agent-c"]),
        );
        let result = tool
            .execute(inputs, HashMap::new(), &ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nesting depth exceeded"));
    }

    #[tokio::test]
    async fn run_agent_circular_reference() {
        let tool = RunAgentTool;
        let mut inputs = HashMap::new();
        inputs.insert("agent_id".to_string(), json!("agent-a"));
        inputs.insert(
            "__agent_call_chain__".to_string(),
            json!(["agent-a", "agent-b"]),
        );
        let result = tool
            .execute(inputs, HashMap::new(), &ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular agent reference"));
    }

    #[test]
    fn register_agent_tools_adds_one() {
        let mut reg = ToolRegistry::new();
        register_agent_tools(&mut reg);
        assert!(reg.get("agent/run_agent").is_some());
        assert_eq!(reg.list_tools().len(), 1);
    }
}
