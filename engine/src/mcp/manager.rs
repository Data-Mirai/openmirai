//! MCPManager — lazy connection pool for MCP servers.
//!
//! Creates and caches MCPClient connections per server_name.
//! Connections are established lazily on first use and cleaned up
//! when the manager is dropped or close_all() is called.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::client::{HttpTransport, MCPClient, MCPError, StdioTransport};
use crate::core::agent_spec::AgentMcpServerSpec;

/// Manages MCP server connections for a single agent execution.
///
/// Thread-safe via internal Mutex. Connections are lazy — only created
/// when first requested via `get_or_connect()`.
pub struct MCPManager {
    configs: HashMap<String, AgentMcpServerSpec>,
    connections: Arc<Mutex<HashMap<String, MCPClient>>>,
}

impl MCPManager {
    /// Create a new manager from the agent's MCP server configs.
    pub fn new(servers: &[AgentMcpServerSpec]) -> Self {
        let configs: HashMap<String, AgentMcpServerSpec> = servers
            .iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();

        Self {
            configs,
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create an empty manager (no MCP servers configured).
    pub fn empty() -> Self {
        Self {
            configs: HashMap::new(),
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a server is configured.
    pub fn has_server(&self, name: &str) -> bool {
        self.configs.contains_key(name)
    }

    /// Get server names.
    pub fn server_names(&self) -> Vec<&str> {
        self.configs.keys().map(|s| s.as_str()).collect()
    }

    /// Get or create a connection to the named MCP server.
    ///
    /// On first call for a given name: creates transport, spawns process
    /// (if stdio), runs initialize handshake, caches the client.
    /// Subsequent calls return the cached client.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: HashMap<String, Value>,
    ) -> Result<Value, MCPError> {
        // Check config exists.
        let config = self.configs.get(server_name).ok_or_else(|| {
            MCPError::ProtocolError(format!(
                "MCP server '{}' not found in agent config. Available: {:?}",
                server_name,
                self.configs.keys().collect::<Vec<_>>()
            ))
        })?;

        let mut connections = self.connections.lock().await;

        // Create connection if not cached.
        if !connections.contains_key(server_name) {
            info!(server = server_name, "creating MCP connection (lazy)");

            let mut client = match config.transport.as_str() {
                "stdio" => {
                    let command = config.command.as_deref().ok_or_else(|| {
                        MCPError::ProcessError(format!(
                            "MCP server '{}' transport=stdio but no 'command' specified",
                            server_name
                        ))
                    })?;

                    let mut cmd_vec = vec![command.to_string()];
                    cmd_vec.extend(config.args.iter().cloned());

                    let transport = StdioTransport::new(cmd_vec);
                    MCPClient::new(Box::new(transport))
                }
                "http" => {
                    let url = config.url.as_deref().ok_or_else(|| {
                        MCPError::ProcessError(format!(
                            "MCP server '{}' transport=http but no 'url' specified",
                            server_name
                        ))
                    })?;

                    let transport = HttpTransport::new(url);
                    MCPClient::new(Box::new(transport))
                }
                other => {
                    return Err(MCPError::ProcessError(format!(
                        "MCP server '{}' has unsupported transport: '{}'. Use 'stdio' or 'http'.",
                        server_name, other
                    )));
                }
            };

            // Initialize handshake.
            client.initialize().await.map_err(|e| {
                MCPError::ProtocolError(format!(
                    "MCP handshake failed for '{}': {}",
                    server_name, e
                ))
            })?;

            connections.insert(server_name.to_string(), client);
        }

        // Execute tool call.
        let client = connections.get(server_name).unwrap();
        client.call_tool(tool_name, arguments).await
    }

    /// Close all active connections and kill spawned processes.
    pub async fn close_all(&self) {
        let mut connections = self.connections.lock().await;
        let names: Vec<String> = connections.keys().cloned().collect();

        for name in names {
            if let Some(client) = connections.remove(&name) {
                info!(server = name, "closing MCP connection");
                if let Err(e) = client.close().await {
                    warn!(server = name, error = %e, "error closing MCP connection");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manager() {
        let mgr = MCPManager::empty();
        assert!(!mgr.has_server("anything"));
        assert!(mgr.server_names().is_empty());
    }

    #[test]
    fn manager_from_configs() {
        let configs = vec![
            AgentMcpServerSpec {
                name: "server-a".into(),
                transport: "stdio".into(),
                command: Some("node".into()),
                args: vec!["server.js".into()],
                url: None,
                credential_ref: None,
            },
            AgentMcpServerSpec {
                name: "server-b".into(),
                transport: "http".into(),
                command: None,
                args: vec![],
                url: Some("http://localhost:8080".into()),
                credential_ref: None,
            },
        ];

        let mgr = MCPManager::new(&configs);
        assert!(mgr.has_server("server-a"));
        assert!(mgr.has_server("server-b"));
        assert!(!mgr.has_server("server-c"));
        assert_eq!(mgr.server_names().len(), 2);
    }

    #[tokio::test]
    async fn call_tool_unknown_server_returns_error() {
        let mgr = MCPManager::empty();
        let result = mgr
            .call_tool("nonexistent", "some_tool", HashMap::new())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found in agent config"));
    }

    #[tokio::test]
    async fn call_tool_stdio_missing_command_returns_error() {
        let configs = vec![AgentMcpServerSpec {
            name: "bad".into(),
            transport: "stdio".into(),
            command: None, // Missing!
            args: vec![],
            url: None,
            credential_ref: None,
        }];

        let mgr = MCPManager::new(&configs);
        let result = mgr.call_tool("bad", "tool", HashMap::new()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no 'command' specified"));
    }

    #[tokio::test]
    async fn call_tool_http_missing_url_returns_error() {
        let configs = vec![AgentMcpServerSpec {
            name: "bad-http".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: None, // Missing!
            credential_ref: None,
        }];

        let mgr = MCPManager::new(&configs);
        let result = mgr.call_tool("bad-http", "tool", HashMap::new()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no 'url' specified"));
    }

    #[tokio::test]
    async fn call_tool_unsupported_transport() {
        let configs = vec![AgentMcpServerSpec {
            name: "weird".into(),
            transport: "grpc".into(),
            command: None,
            args: vec![],
            url: None,
            credential_ref: None,
        }];

        let mgr = MCPManager::new(&configs);
        let result = mgr.call_tool("weird", "tool", HashMap::new()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported transport"));
    }

    #[tokio::test]
    async fn close_all_on_empty_is_noop() {
        let mgr = MCPManager::empty();
        mgr.close_all().await; // Should not panic
    }
}
