//! Application state and shared request/response types.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::core::agent_spec::AgentSpec;
use crate::core::context::LLMResource;
use crate::core::graph::GraphDef;
use crate::core::runner::{ExecutionResult, GraphRunner};
use crate::tools::registry::{RegistryExecutor, ToolRegistry};
use crate::tools::builtin::register_all_builtin_tools;
/// Shared application state passed to all handlers via axum's `State`.
///
/// Uses `Arc<RwLock<HashMap>>` for in-memory storage. A real DB layer will
/// replace this later.
/// Factory function that creates a real LLMResource for each execution.
/// This is set once at server startup and cloned per-request.
pub type LLMFactory = Arc<dyn Fn() -> Box<dyn LLMResource> + Send + Sync>;

/// Maximum sessions kept in memory before eviction.
const DEFAULT_MAX_SESSIONS: usize = 10_000;

/// Default execution timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Clone)]
pub struct AppState {
    pub graphs: Arc<RwLock<HashMap<String, GraphDef>>>,
    pub agents: Arc<RwLock<HashMap<String, AgentSpec>>>,
    pub sessions: Arc<RwLock<HashMap<String, ExecutionResult>>>,
    /// Insertion-order tracking for FIFO eviction.
    session_order: Arc<RwLock<Vec<String>>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub runner: GraphRunner,
    pub llm_factory: LLMFactory,
    pub start_time: std::time::Instant,
    /// Optional API key for authentication.
    pub api_key: Option<String>,
    /// Max sessions in memory (FIFO eviction when exceeded).
    pub max_sessions: usize,
    /// Execution timeout in seconds for agent run endpoints.
    pub timeout_secs: u64,
}

impl AppState {
    /// Create app state with a real LLM factory. NO MOCKS.
    pub fn new(tool_registry: ToolRegistry, llm_factory: LLMFactory, api_key: Option<String>) -> Self {
        let registry = Arc::new(tool_registry);
        let executor = RegistryExecutor::new(registry.clone());
        let runner = GraphRunner::new(Box::new(executor));
        Self {
            graphs: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            session_order: Arc::new(RwLock::new(Vec::new())),
            tool_registry: registry,
            runner,
            llm_factory,
            start_time: std::time::Instant::now(),
            api_key,
            max_sessions: DEFAULT_MAX_SESSIONS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    /// Insert a session with FIFO eviction when max_sessions is exceeded.
    pub async fn insert_session(&self, id: String, result: ExecutionResult) {
        let mut sessions = self.sessions.write().await;
        let mut order = self.session_order.write().await;

        // Evict oldest if at capacity.
        while order.len() >= self.max_sessions {
            if let Some(oldest_id) = order.first().cloned() {
                sessions.remove(&oldest_id);
                order.remove(0);
            }
        }

        sessions.insert(id.clone(), result);
        order.push(id);
    }
}

// ---------------------------------------------------------------------------
// Request / Response models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GraphCreateRequest {
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub edges: Vec<Value>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct AgentCreateRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub graph_id: String,
    #[serde(default)]
    pub triggers: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteRequest {
    #[serde(default)]
    pub entry_node_id: Option<String>,
    #[serde(default)]
    pub trigger_data: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct SessionListQuery {
    pub agent_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

