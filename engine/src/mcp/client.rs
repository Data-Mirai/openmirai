//! MCP client -- JSON-RPC 2.0 transport + high-level operations.
//!
//! Implements the Model Context Protocol (2024-11-05) with two transports:
//! [`StdioTransport`] (subprocess via stdin/stdout) and [`HttpTransport`] (HTTP POST).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// MCP protocol version this client speaks.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Default timeout for the connect/spawn phase (seconds).
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;

/// Default timeout for reading a response line from stdio (seconds).
const DEFAULT_READ_TIMEOUT_SECS: u64 = 600;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors raised by MCP transport or protocol operations.
#[derive(Debug, thiserror::Error)]
pub enum MCPError {
    #[error("transport error: {0}")]
    TransportError(String),

    #[error("protocol error: {0}")]
    ProtocolError(String),

    #[error("request timed out")]
    TimeoutError,

    #[error("process error: {0}")]
    ProcessError(String),
}

// ---------------------------------------------------------------------------
// JSON-RPC structs
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

/// Abstract async transport for JSON-RPC 2.0 communication with an MCP server.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a JSON-RPC request and return the response.
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, MCPError>;

    /// Send a JSON-RPC notification (fire-and-forget, no response expected).
    ///
    /// Per JSON-RPC 2.0 spec, notifications are requests without a response.
    /// The server MUST NOT reply.  This method writes to the transport
    /// without waiting to read anything back.
    async fn send_notification(&self, request: JsonRpcRequest) -> Result<(), MCPError>;

    /// Tear down the connection and release resources.
    async fn close(&mut self) -> Result<(), MCPError>;
}

// ---------------------------------------------------------------------------
// StdioTransport
// ---------------------------------------------------------------------------

/// Launch an MCP server as a subprocess; communicate via stdin/stdout.
///
/// Each JSON-RPC message is a single line terminated by `\n`.
pub struct StdioTransport {
    command: Vec<String>,
    env: Option<HashMap<String, String>>,
    child: Arc<Mutex<Option<Child>>>,
    connect_timeout_secs: u64,
    read_timeout_secs: u64,
}

impl StdioTransport {
    /// Create a new stdio transport that will spawn the given command.
    ///
    /// The process is not started until [`StdioTransport::spawn`] is called
    /// (or implicitly on first [`Transport::send`]).
    pub fn new(command: Vec<String>) -> Self {
        Self {
            command,
            env: None,
            child: Arc::new(Mutex::new(None)),
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            read_timeout_secs: DEFAULT_READ_TIMEOUT_SECS,
        }
    }

    /// Set custom environment variables for the subprocess.
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = Some(env);
        self
    }

    /// Override the connect timeout (default 30 s).
    pub fn with_connect_timeout(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    /// Override the read timeout (default 600 s).
    pub fn with_read_timeout(mut self, secs: u64) -> Self {
        self.read_timeout_secs = secs;
        self
    }

    /// Spawn the subprocess. Idempotent -- does nothing if already running.
    pub async fn spawn(&self) -> Result<(), MCPError> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        if self.command.is_empty() {
            return Err(MCPError::ProcessError(
                "command must be a non-empty list".into(),
            ));
        }

        info!(cmd = ?self.command, "spawning MCP subprocess");

        let mut cmd = Command::new(&self.command[0]);
        if self.command.len() > 1 {
            cmd.args(&self.command[1..]);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(env) = &self.env {
            cmd.envs(env);
        }

        let spawn_fut = cmd.spawn();

        let child = tokio::time::timeout(
            std::time::Duration::from_secs(self.connect_timeout_secs),
            async { spawn_fut },
        )
        .await
        .map_err(|_| MCPError::TimeoutError)?
        .map_err(|e| MCPError::ProcessError(format!("failed to spawn: {e}")))?;

        info!(pid = ?child.id(), "MCP subprocess started");
        *guard = Some(child);
        Ok(())
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, MCPError> {
        // Ensure the process is running.
        self.spawn().await?;

        let mut guard = self.child.lock().await;
        let child = guard
            .as_mut()
            .ok_or_else(|| MCPError::TransportError("subprocess not running".into()))?;

        // Serialize request to a single JSON line.
        let payload = serde_json::to_string(&request)
            .map_err(|e| MCPError::TransportError(format!("serialize error: {e}")))?;
        debug!(payload = %payload, "stdio >>>");

        // Write to stdin.
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| MCPError::TransportError("stdin not available".into()))?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin write: {e}")))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin write newline: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin flush: {e}")))?;

        // Read one line from stdout with timeout.
        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| MCPError::TransportError("stdout not available".into()))?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        let read_result = tokio::time::timeout(
            std::time::Duration::from_secs(self.read_timeout_secs),
            reader.read_line(&mut line),
        )
        .await;

        match read_result {
            Err(_) => return Err(MCPError::TimeoutError),
            Ok(Err(e)) => {
                return Err(MCPError::TransportError(format!("stdout read: {e}")));
            }
            Ok(Ok(0)) => {
                return Err(MCPError::TransportError(
                    "subprocess closed stdout unexpectedly".into(),
                ));
            }
            Ok(Ok(_)) => {}
        }

        debug!(line = %line.trim(), "stdio <<<");

        let response: JsonRpcResponse = serde_json::from_str(line.trim())
            .map_err(|e| MCPError::TransportError(format!("invalid JSON from server: {e}")))?;

        Ok(response)
    }

    async fn send_notification(&self, request: JsonRpcRequest) -> Result<(), MCPError> {
        self.spawn().await?;

        let mut guard = self.child.lock().await;
        let child = guard
            .as_mut()
            .ok_or_else(|| MCPError::TransportError("subprocess not running".into()))?;

        let payload = serde_json::to_string(&request)
            .map_err(|e| MCPError::TransportError(format!("serialize error: {e}")))?;
        debug!(payload = %payload, "stdio >>> (notification)");

        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| MCPError::TransportError("stdin not available".into()))?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin write: {e}")))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin newline: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| MCPError::TransportError(format!("stdin flush: {e}")))?;

        Ok(())
    }

    async fn close(&mut self) -> Result<(), MCPError> {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            info!(pid = ?child.id(), "closing MCP subprocess");

            // Close stdin to signal EOF.
            drop(child.stdin.take());

            // Wait up to 5 s for a clean exit, then kill.
            let wait_result =
                tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;

            match wait_result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    warn!(error = %e, "error waiting for subprocess");
                }
                Err(_) => {
                    warn!("subprocess did not exit within 5 s; killing");
                    let _ = child.kill().await;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HttpTransport
// ---------------------------------------------------------------------------

/// Connect to an MCP server over HTTP POST (JSON-RPC over HTTP).
pub struct HttpTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl HttpTransport {
    /// Create a new HTTP transport pointing at the given URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    /// Add custom headers sent with every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, MCPError> {
        let mut builder = self.client.post(&self.url).json(&request);

        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        debug!(url = %self.url, method = %request.method, "http >>>");

        let resp = builder
            .send()
            .await
            .map_err(|e| MCPError::TransportError(format!("HTTP request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MCPError::TransportError(format!(
                "HTTP {status} from MCP server: {body}"
            )));
        }

        let response: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| MCPError::TransportError(format!("invalid JSON response: {e}")))?;

        debug!(id = ?response.id, "http <<<");
        Ok(response)
    }

    async fn send_notification(&self, request: JsonRpcRequest) -> Result<(), MCPError> {
        // For HTTP, fire-and-forget: send the request, ignore the response body.
        let mut builder = self.client.post(&self.url).json(&request);
        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        debug!(url = %self.url, method = %request.method, "http >>> (notification)");
        let _ = builder.send().await;
        Ok(())
    }

    async fn close(&mut self) -> Result<(), MCPError> {
        // HTTP is stateless -- nothing to close.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MCPClient
// ---------------------------------------------------------------------------

/// High-level async client for Model Context Protocol servers.
///
/// Wraps a [`Transport`] and provides typed helpers for the standard MCP
/// operations: `initialize`, `list_tools`, `call_tool`.
pub struct MCPClient {
    transport: Box<dyn Transport>,
    request_id: AtomicU64,
    /// Server info received during `initialize`.
    pub server_info: Option<Value>,
    /// Server capabilities received during `initialize`.
    pub server_capabilities: Option<Value>,
}

impl MCPClient {
    /// Create a new client backed by the given transport.
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            request_id: AtomicU64::new(0),
            server_info: None,
            server_capabilities: None,
        }
    }

    /// Generate the next request id (monotonically increasing).
    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Send a raw JSON-RPC request through the transport, validating the response.
    async fn rpc(&self, method: &str, params: Option<Value>) -> Result<Value, MCPError> {
        let id = self.next_id();
        let request = JsonRpcRequest::new(id, method, params);
        let response = self.transport.send(request).await?;

        // Check for JSON-RPC error.
        if let Some(err) = response.error {
            return Err(MCPError::ProtocolError(format!(
                "MCP error {}: {}",
                err.code, err.message
            )));
        }

        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Perform the MCP `initialize` handshake.
    ///
    /// Stores `server_info` and `server_capabilities` from the response.
    pub async fn initialize(&mut self) -> Result<(), MCPError> {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "openmirai-engine",
                "version": "0.1.0"
            }
        });

        let result = self.rpc("initialize", Some(params)).await?;

        self.server_capabilities = result.get("capabilities").cloned();
        self.server_info = result.get("serverInfo").cloned();

        info!(
            server = ?self.server_info,
            "MCP initialized"
        );

        // Per JSON-RPC spec, client MUST send `notifications/initialized` after init.
        // Notifications have no response — use send_notification() to avoid blocking.
        let notif_id = self.next_id();
        let notif = JsonRpcRequest::new(notif_id, "notifications/initialized", None);
        self.transport.send_notification(notif).await?;

        Ok(())
    }

    /// Request the list of tools exposed by the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<Value>, MCPError> {
        let result = self.rpc("tools/list", None).await?;

        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        info!(count = tools.len(), "MCP server exposes tool(s)");
        Ok(tools)
    }

    /// Execute a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: HashMap<String, Value>,
    ) -> Result<Value, MCPError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.rpc("tools/call", Some(params)).await?;
        Ok(result)
    }

    /// Shut down the transport.
    pub async fn close(mut self) -> Result<(), MCPError> {
        self.transport.close().await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // -- Shared mock state (Arc-based, survives move into Box<dyn Transport>) --

    #[derive(Default)]
    struct MockState {
        responses: StdMutex<Vec<JsonRpcResponse>>,
        sent: StdMutex<Vec<JsonRpcRequest>>,
    }

    impl MockState {
        fn new(responses: Vec<JsonRpcResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: StdMutex::new(responses),
                sent: StdMutex::new(Vec::new()),
            })
        }

        fn sent_requests(&self) -> Vec<JsonRpcRequest> {
            self.sent.lock().unwrap().clone()
        }
    }

    /// A mock transport that records sent requests and returns canned responses.
    struct MockTransport {
        state: Arc<MockState>,
    }

    impl MockTransport {
        fn new(state: Arc<MockState>) -> Self {
            Self { state }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, MCPError> {
            self.state.sent.lock().unwrap().push(request);
            let mut responses = self.state.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(MCPError::TransportError("no more canned responses".into()));
            }
            Ok(responses.remove(0))
        }

        async fn send_notification(&self, request: JsonRpcRequest) -> Result<(), MCPError> {
            // Record the notification but don't consume a response.
            self.state.sent.lock().unwrap().push(request);
            Ok(())
        }

        async fn close(&mut self) -> Result<(), MCPError> {
            Ok(())
        }
    }

    fn ok_response(id: u64, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    // -- Tests ------------------------------------------------------------

    #[tokio::test]
    async fn test_initialize() {
        let init_result = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "test-server", "version": "1.0" }
        });

        // Two responses: one for initialize, one for the notification.
        let state = MockState::new(vec![
            ok_response(1, init_result),
            ok_response(2, Value::Null),
        ]);
        let state_ref = state.clone();

        let mut client = MCPClient::new(Box::new(MockTransport::new(state)));
        client.initialize().await.unwrap();

        assert!(client.server_info.is_some());
        assert_eq!(client.server_info.as_ref().unwrap()["name"], "test-server");
        assert!(client.server_capabilities.is_some());

        // Verify two requests were sent.
        let sent = state_ref.sent_requests();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].method, "initialize");
        assert_eq!(sent[1].method, "notifications/initialized");
    }

    #[tokio::test]
    async fn test_list_tools() {
        let tools_result = serde_json::json!({
            "tools": [
                { "name": "echo", "description": "echoes input" },
                { "name": "add", "description": "adds numbers" }
            ]
        });

        let state = MockState::new(vec![ok_response(1, tools_result)]);
        let client = MCPClient::new(Box::new(MockTransport::new(state)));

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "echo");
    }

    #[tokio::test]
    async fn test_call_tool() {
        let call_result = serde_json::json!({
            "content": [{ "type": "text", "text": "hello" }]
        });

        let state = MockState::new(vec![ok_response(1, call_result.clone())]);
        let client = MCPClient::new(Box::new(MockTransport::new(state)));

        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::String("hi".into()));

        let result = client.call_tool("echo", args).await.unwrap();
        assert_eq!(result, call_result);
    }

    #[tokio::test]
    async fn test_protocol_error() {
        let err_response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found".into(),
                data: None,
            }),
        };

        let state = MockState::new(vec![err_response]);
        let client = MCPClient::new(Box::new(MockTransport::new(state)));

        let result = client.list_tools().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            MCPError::ProtocolError(msg) => {
                assert!(msg.contains("-32601"));
                assert!(msg.contains("Method not found"));
            }
            other => panic!("expected ProtocolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_request_ids_increment() {
        let state = MockState::new(vec![
            ok_response(1, Value::Null),
            ok_response(2, Value::Null),
            ok_response(3, Value::Null),
        ]);
        let state_ref = state.clone();

        let client = MCPClient::new(Box::new(MockTransport::new(state)));

        let _ = client.rpc("a", None).await;
        let _ = client.rpc("b", None).await;
        let _ = client.rpc("c", None).await;

        let sent = state_ref.sent_requests();
        assert_eq!(sent[0].id, 1);
        assert_eq!(sent[1].id, 2);
        assert_eq!(sent[2].id, 3);
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let req = JsonRpcRequest::new(42, "test/method", Some(serde_json::json!({"key": "val"})));
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 42);
        assert_eq!(json["method"], "test/method");
        assert_eq!(json["params"]["key"], "val");
    }

    #[test]
    fn test_json_rpc_request_no_params() {
        let req = JsonRpcRequest::new(1, "ping", None);
        let json = serde_json::to_string(&req).unwrap();
        // params should be omitted entirely.
        assert!(!json.contains("params"));
    }

    #[test]
    fn test_http_transport_creation() {
        let t = HttpTransport::new("http://localhost:8080");
        assert_eq!(t.url, "http://localhost:8080");
        assert!(t.headers.is_empty());
    }

    #[test]
    fn test_http_transport_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer tok".into());
        let t = HttpTransport::new("http://localhost:8080").with_headers(headers);
        assert_eq!(t.headers.len(), 1);
        assert_eq!(t.headers["Authorization"], "Bearer tok");
    }

    #[test]
    fn test_stdio_transport_creation() {
        let t = StdioTransport::new(vec!["echo".into(), "hello".into()]);
        assert_eq!(t.command, vec!["echo", "hello"]);
        assert_eq!(t.connect_timeout_secs, DEFAULT_CONNECT_TIMEOUT_SECS);
        assert_eq!(t.read_timeout_secs, DEFAULT_READ_TIMEOUT_SECS);
    }

    #[test]
    fn test_stdio_transport_builder() {
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());

        let t = StdioTransport::new(vec!["cmd".into()])
            .with_env(env)
            .with_connect_timeout(10)
            .with_read_timeout(120);

        assert_eq!(t.connect_timeout_secs, 10);
        assert_eq!(t.read_timeout_secs, 120);
        assert!(t.env.is_some());
    }
}
