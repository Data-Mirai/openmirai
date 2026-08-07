use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::core::agent_spec::AgentMcpServerSpec;
use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::mcp::MCPManager;
use crate::tools::base::{field, FieldType};
use crate::tools::registry::{Tool, ToolRegistry};

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
        inputs: HashMap<String, Value>,
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

        // Parse arguments from inputs or config.
        let arguments: HashMap<String, Value> = inputs
            .get("arguments")
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .or_else(|| {
                config
                    .get("arguments")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            })
            .unwrap_or_default();

        // Parse mcp_servers from config (injected by runner/CLI).
        let mcp_servers: Vec<AgentMcpServerSpec> = config
            .get("__mcp_servers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if mcp_servers.is_empty() {
            // Fallback: return descriptive error about missing config.
            let mut out = HashMap::new();
            out.insert("result".to_string(), Value::Null);
            out.insert("server_name".to_string(), json!(server_name));
            out.insert("tool_name".to_string(), json!(tool_name));
            out.insert("success".to_string(), json!(false));
            out.insert(
                "error".to_string(),
                json!(format!(
                    "No MCP servers configured. Add 'mcp_servers' to your agent spec config section."
                )),
            );
            return Ok(out);
        }

        // Create MCPManager and execute.
        let manager = MCPManager::new(&mcp_servers);

        info!(
            server = server_name,
            tool = tool_name,
            args_count = arguments.len(),
            "executing MCP tool call"
        );

        match manager.call_tool(server_name, tool_name, arguments).await {
            Ok(result) => {
                // MCP returns { "content": [{ "type": "text", "text": "..." }] }
                // Extract text content for convenience.
                let text_content = result
                    .get("content")
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| {
                                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                let mut out = HashMap::new();
                out.insert("result".to_string(), result);
                out.insert("text".to_string(), json!(text_content));
                out.insert("server_name".to_string(), json!(server_name));
                out.insert("tool_name".to_string(), json!(tool_name));
                out.insert("success".to_string(), json!(true));
                out.insert("error".to_string(), Value::Null);

                // Cleanup: close the manager's connections.
                manager.close_all().await;

                Ok(out)
            }
            Err(e) => {
                warn!(
                    server = server_name,
                    tool = tool_name,
                    error = %e,
                    "MCP tool call failed"
                );

                manager.close_all().await;

                let mut out = HashMap::new();
                out.insert("result".to_string(), Value::Null);
                out.insert("server_name".to_string(), json!(server_name));
                out.insert("tool_name".to_string(), json!(tool_name));
                out.insert("success".to_string(), json!(false));
                out.insert("error".to_string(), json!(e.to_string()));
                Ok(out)
            }
        }
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
    async fn mcp_call_without_servers_config_returns_descriptive_error() {
        let tool = McpCallTool;
        let mut config = HashMap::new();
        config.insert("server_name".to_string(), json!("my-server"));
        config.insert("tool_name".to_string(), json!("my-tool"));
        // No __mcp_servers → should return helpful error
        let result = tool.execute(HashMap::new(), &config, &ctx()).await.unwrap();
        assert_eq!(result["success"], json!(false));
        assert_eq!(result["server_name"], json!("my-server"));
        assert_eq!(result["tool_name"], json!("my-tool"));
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("No MCP servers configured"));
    }

    #[tokio::test]
    async fn mcp_call_with_nonexistent_server_returns_not_found() {
        let tool = McpCallTool;
        let servers = vec![AgentMcpServerSpec {
            name: "other-server".into(),
            transport: "stdio".into(),
            command: Some("echo".into()),
            args: vec![],
            url: None,
            credential_ref: None,
        }];

        let mut config = HashMap::new();
        config.insert("server_name".to_string(), json!("wrong-name"));
        config.insert("tool_name".to_string(), json!("tool"));
        config.insert(
            "__mcp_servers".to_string(),
            serde_json::to_value(&servers).unwrap(),
        );

        let result = tool.execute(HashMap::new(), &config, &ctx()).await.unwrap();
        assert_eq!(result["success"], json!(false));
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("not found in agent config"));
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
