//! Application state and shared request/response types.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::core::agent_spec::AgentSpec;
use crate::core::context::LLMResource;
use crate::core::graph::GraphDef;
use crate::core::runner::{ExecutionResult, GraphRunner};
use crate::fleet::FleetStore;
use crate::runtime::agent_memory_store::AgentMemoryStore;
use crate::runtime::scheduler::Scheduler;
use crate::sessions::{SessionManager, TmuxBackend};
use crate::tools::registry::{RegistryExecutor, ToolRegistry};
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
    /// Per-agent persistent memory store (PRD-008).
    pub memory_store: AgentMemoryStore,
    /// Background scheduler for live agent cycles (PRD-008).
    pub scheduler: Arc<Scheduler>,
    /// Orchestrated Claude sessions over tmux (PRD-013).
    ///
    /// Created with the real [`TmuxBackend`] by default; `serve()` loads and
    /// reconciles the persistent registry and starts polling. Tests replace
    /// this field with a manager over a fake backend before building the
    /// router.
    pub orchestrator: Arc<SessionManager>,
    /// Directory served as the static web UI under `/ui` (PRD-013 M6).
    /// `None` → `/ui` answers 404 with a clear message.
    pub ui_dir: Option<std::path::PathBuf>,
    /// Roots scanned for first-level project directories (PRD-013 M7,
    /// `--projects-dirs a:b:c` or MIRAI_PROJECTS_DIRS). Empty → /projects
    /// lists session dirs only.
    pub projects_dirs: Vec<std::path::PathBuf>,
    /// Native host folder picker (PRD-013 M9). One dialog at a time.
    pub folder_picker: Arc<crate::sessions::picker::FolderPicker>,
    /// Fleet SoT — SQLite (WAL) source of truth for the agent fleet.
    ///
    /// `AppState::new` defaults to an in-memory store; `serve()` swaps in the
    /// file-backed one at `~/.openmirai/fleet.db` so the fleet survives
    /// restarts. Tests reach it directly for setup/asserts.
    pub fleet: Arc<FleetStore>,
    /// Async workflow runs registry (run_id → record). Populated by
    /// `POST /api/v1/workflows/{name}/run`, read by `GET /api/v1/runs/{id}`.
    pub runs: Arc<RwLock<HashMap<String, crate::server::workflows::RunRecord>>>,
    /// Directories scanned for `{name}.yaml` workflows by the run endpoint
    /// (`--workflows-dir` / MIRAI_WORKFLOWS_DIR; defaults to `./agents`).
    pub workflows_dirs: Vec<std::path::PathBuf>,
    /// Port this server is bound to — injected as `base_url` into workflow
    /// runs so `net/http_request` nodes can call this same engine. Set by
    /// `serve()`; 0 in tests unless overridden.
    pub server_port: u16,
}

impl AppState {
    /// Create app state with a real LLM factory. NO MOCKS.
    pub fn new(
        tool_registry: ToolRegistry,
        llm_factory: LLMFactory,
        api_key: Option<String>,
    ) -> Self {
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
            memory_store: AgentMemoryStore::new(),
            scheduler: Arc::new(Scheduler::new()),
            orchestrator: Arc::new(SessionManager::new(
                Arc::new(TmuxBackend::new()),
                SessionManager::default_registry_path(),
            )),
            ui_dir: None,
            projects_dirs: Vec::new(),
            folder_picker: Arc::new(crate::sessions::picker::FolderPicker::new()),
            fleet: Arc::new(
                FleetStore::in_memory().expect("in-memory fleet store must open"),
            ),
            runs: Arc::new(RwLock::new(HashMap::new())),
            workflows_dirs: Vec::new(),
            server_port: 0,
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
