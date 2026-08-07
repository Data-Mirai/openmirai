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
use crate::db::{AgentStore, Repository, SessionRecord, SessionStatus, SqliteSessionRepo};
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
    /// Persistencia de runs en SQLite (0.7.0). `None` → solo memoria
    /// (tests / factories sin `serve()`); `serve()` la cablea siempre.
    pub session_repo: Option<Arc<SqliteSessionRepo>>,
    /// Registro durable de agentes, sobre la MISMA base que `session_repo`.
    /// `None` → el registro vive solo en memoria y se pierde al reiniciar,
    /// dejando además a los agentes live apagados sin quién los relance.
    pub agent_store: Option<AgentStore>,
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
            session_repo: None,
            agent_store: None,
        }
    }

    /// Espeja un agente en el registro durable, si está cableado.
    ///
    /// Un fallo de escritura se loguea y se traga a propósito: perder
    /// durabilidad no puede convertir un request que funcionó en un 500. El
    /// agente sigue vivo en el mapa en memoria y la respuesta sale igual.
    pub async fn persist_agent(&self, id: &str, spec: &AgentSpec) {
        if let Some(store) = &self.agent_store {
            if let Err(e) = store.upsert(id, spec).await {
                tracing::error!(agent_id = %id, error = %e, "cannot persist agent — it will be lost on restart");
            }
        }
    }

    /// Anota si un agente quedó ciclando, para relanzarlo al arrancar.
    pub async fn persist_playing(&self, id: &str, playing: bool) {
        if let Some(store) = &self.agent_store {
            if let Err(e) = store.set_playing(id, playing).await {
                tracing::error!(agent_id = %id, error = %e, "cannot persist playing state");
            }
        }
    }

    /// Registra una ejecución terminada: cache en memoria (para lecturas
    /// calientes) + persistencia en SQLite si está cableada. Un fallo de
    /// persistencia NO tumba el request — se loguea y la respuesta sigue.
    pub async fn record_session(
        &self,
        id: String,
        agent_id: &str,
        agent_name: &str,
        started_at: f64,
        result: ExecutionResult,
    ) {
        if let Some(repo) = &self.session_repo {
            let finished_at = crate::utils::now_epoch();
            let rec = SessionRecord {
                id: id.clone(),
                agent_id: agent_id.to_string(),
                agent_name: agent_name.to_string(),
                graph_id: String::new(),
                result: result.clone(),
                created_at: started_at,
                finished_at: Some(finished_at),
                duration_ms: Some((finished_at - started_at) * 1000.0),
                status: SessionStatus::from_execution(&result.status),
            };
            if let Err(e) = repo.save(&rec).await {
                tracing::warn!(session_id = %id, error = %e, "no se pudo persistir el run");
            }
        }
        self.insert_session(id, result).await;
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
