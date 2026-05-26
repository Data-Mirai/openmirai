use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{FieldType, ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
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

macro_rules! mcp_tool {
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
                        category: "mcp".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ===========================================================================
// McpCallTool
// ===========================================================================

mcp_tool! {
    struct McpCallTool, factory McpCallFactory;
    tool_type = "mcp/call",
    name = "MCP Call",
    description = "Invokes a tool on a Model Context Protocol server. Placeholder until MCP client is configured.",
    inputs = [
        field("arguments", FieldType::Object, false, "Arguments to pass to the MCP tool"),
    ],
    outputs = [
        field("result", FieldType::Object, true, "Tool execution result"),
        field("server_name", FieldType::String, true, "MCP server name"),
        field("tool_name", FieldType::String, true, "MCP tool name"),
        field("success", FieldType::Boolean, true, "Whether the call succeeded"),
        field("error", FieldType::String, false, "Error message if call failed"),
    ],
    config_fields = [
        field("server_name", FieldType::String, true, "Name of the MCP server"),
        field("tool_name", FieldType::String, true, "Name of the tool on the server"),
    ]
}

#[async_trait]
impl Tool for McpCallTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let server_name = config
            .get("server_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tool_name = config
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Placeholder: MCP client not implemented yet.
        let mut out = HashMap::new();
        out.insert("result".to_string(), Value::Null);
        out.insert("server_name".to_string(), json!(server_name));
        out.insert("tool_name".to_string(), json!(tool_name));
        out.insert("success".to_string(), json!(false));
        out.insert(
            "error".to_string(),
            json!("MCP client not configured"),
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all MCP tools into the given registry.
pub fn register_mcp_tools(registry: &mut ToolRegistry) {
    registry.register("mcp/call", Box::new(McpCallFactory::new()));
    // Legacy alias — remove in v0.3.0
    registry.register_alias("mcp/mcp_call", "mcp/call");
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
    async fn mcp_call_returns_not_configured() {
        let tool = McpCallTool;
        let mut config = HashMap::new();
        config.insert("server_name".to_string(), json!("my-server"));
        config.insert("tool_name".to_string(), json!("my-tool"));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["success"], json!(false));
        assert_eq!(result["server_name"], json!("my-server"));
        assert_eq!(result["tool_name"], json!("my-tool"));
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("not configured"));
    }

    #[test]
    fn register_mcp_tools_adds_one() {
        let mut reg = ToolRegistry::new();
        register_mcp_tools(&mut reg);
        assert!(reg.get("mcp/call").is_some());
        assert_eq!(reg.list_tools().len(), 1);
    }

    #[test]
    fn legacy_mcp_alias_resolves() {
        let mut reg = ToolRegistry::new();
        register_mcp_tools(&mut reg);
        assert!(reg.get("mcp/mcp_call").is_some());
    }
}
