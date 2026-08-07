//! Application state and shared request/response types.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::core::agent_spec::AgentSpec;
use crate::core::context::LLMResource;
use crate::core::events::EventEmitter;
use crate::core::graph::GraphDef;
use crate::core::runner::{ExecutionResult, GraphRunner};
use crate::db::{
    Repository, SessionRecord, SessionStatus, SqliteCheckpointStore, SqliteSessionRepo,
};
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

/// Capacidad del bus de ejecución (PRD-021-E).
///
/// Es un `broadcast`: si un suscriptor se atrasa más de esto, pierde los
/// eventos viejos (`Lagged`) — no bloquea al runner, que es lo que importa.
/// 1024 aguanta un grafo largo con un cliente distraído sin ser un buffer
/// desmedido por proceso.
const EVENT_BUS_CAPACITY: usize = 1024;

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
    /// **Bus de ejecución** (PRD-021-E): los `ExecutionEvent` que emite el
    /// runner mientras corre un grafo — `session_started`, `block_*`,
    /// `checkpoint_created`, `interrupt_created`, `session_interrupted`…
    ///
    /// Es un `broadcast` de proceso: N suscriptores, atraviesa TODOS los runs.
    /// Se cablea al `runner` en [`AppState::new`], así que cualquier camino que
    /// clone ese runner (`/execute`, `/stream`) emite aquí sin tener que
    /// acordarse. Se consume por `GET /api/v1/events` (ver `server/events.rs`).
    ///
    /// **No confundir con `StreamEvent`** (`streaming.rs`), que es el canal
    /// `mpsc` por-request de `POST /agents/{id}/stream`: otro flujo, otro
    /// contrato. La tabla que compara los dos está en `server/events.rs`.
    pub events: EventEmitter,

    /// Runs EN VUELO en este proceso → su bandera de cancelación (PRD-021-B).
    ///
    /// El runner que ejecuta vive dentro de la tarea del run; el endpoint
    /// `cancel` corre en otra. Este mapa es el puente: se registra al arrancar
    /// el run y se suelta al registrarlo, así que su tamaño es "runs
    /// concurrentes", no "runs históricos".
    pub run_cancels: Arc<RwLock<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
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
        let events = EventEmitter::new(EVENT_BUS_CAPACITY);
        // PRD-021-E: EL cableado. `with_event_emitter` no se llamaba en ningún
        // camino de producción, así que `GraphRunner::emit_event` era un no-op
        // y TODO lo que el runner emitía (incluido el `checkpoint_created` de
        // 021-A) se perdía. Se ata aquí, en el runner base, y no por-run: así
        // cualquier handler que clone `state.runner` —`/execute`, `/stream`, y
        // el que venga— hereda el bus sin tener que acordarse.
        let runner = GraphRunner::new(Box::new(executor)).with_event_emitter(events.clone());
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
            events,
            run_cancels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Runner cableado con **persistencia de checkpoints** para ESTE run
    /// (PRD-021-A) y con su **bandera de cancelación** registrada (PRD-021-B).
    ///
    /// El callback se ata por run —igual que `with_stream_tx`— porque necesita
    /// la identidad del run: el `Checkpoint` que emite el runner solo trae el
    /// `session_id`. Sin DB cableada devuelve el runner sin checkpoints: el
    /// motor corre igual, solo que sin poder reanudar (la cancelación sí
    /// funciona, no depende de la DB).
    pub async fn runner_with_checkpoints(
        &self,
        session_id: &str,
        agent_id: &str,
        agent_name: &str,
        started_at: f64,
    ) -> GraphRunner {
        let runner =
            match &self.session_repo {
                Some(repo) => self.runner.clone().with_checkpoint_callback(Box::new(
                    SqliteCheckpointStore::new(repo.clone(), agent_id, agent_name, started_at),
                )),
                None => self.runner.clone(),
            };
        // Bandera propia por run: `GraphRunner` es Clone y comparte la del
        // constructor por Arc — sin esto, cancelar un run cancelaría TODOS.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.run_cancels
            .write()
            .await
            .insert(session_id.to_string(), flag.clone());
        runner.with_cancel_flag(flag)
    }

    /// Da de alta un agente: registro en memoria **y** en disco (PRD-021-F).
    ///
    /// El `HashMap` en memoria es la lectura caliente; la fila en SQLite es lo
    /// que hace que un run **se pueda reanudar tras reiniciar el proceso**. Sin
    /// esta escritura, al arrancar de nuevo el checkpoint decía dónde se quedó
    /// el run pero nadie sabía qué grafo correr, y `resume` respondía 409.
    ///
    /// Persistir es **best-effort**: si la DB falla se loguea y el alta sigue
    /// —igual que en `record_session`—, porque no poder guardar no puede
    /// impedir trabajar. Sin DB cableada (tests, factories sin `serve()`) solo
    /// queda el registro en memoria, que es el comportamiento de siempre.
    pub async fn register_agent(&self, agent_id: String, spec: AgentSpec) {
        if let Some(repo) = &self.session_repo {
            if let Err(e) = repo.save_agent_spec(&agent_id, &spec).await {
                tracing::warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "no se pudo persistir la definición del agente: sus runs no se podrán reanudar tras reiniciar"
                );
            }
        }
        self.agents.write().await.insert(agent_id, spec);
    }

    /// La definición de un agente: primero la memoria, y si no está —proceso
    /// recién reiniciado— la DB (PRD-021-F).
    ///
    /// **No** re-siembra el `HashMap` con lo que lee: la lectura es de solo
    /// lectura a propósito, para que reanudar un run viejo no "resucite" en
    /// `GET /agents` un agente que este proceso nunca dio de alta.
    pub async fn agent_spec(&self, agent_id: &str) -> Option<AgentSpec> {
        if let Some(spec) = self.agents.read().await.get(agent_id).cloned() {
            return Some(spec);
        }
        let repo = self.session_repo.as_ref()?;
        match repo.get_agent_spec(agent_id).await {
            Ok(spec) => spec,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "no se pudo leer la definición del agente");
                None
            }
        }
    }

    /// Pide cancelar un run EN VUELO. `false` si no hay ninguno con ese id
    /// corriendo en este proceso.
    pub async fn request_cancel(&self, session_id: &str) -> bool {
        match self.run_cancels.read().await.get(session_id) {
            Some(flag) => {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }
            None => false,
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
        // El run dejó de estar en vuelo: se suelta su bandera de cancelación.
        self.run_cancels.write().await.remove(&id);

        if let Some(repo) = &self.session_repo {
            let finished_at = crate::utils::now_epoch();
            let status = SessionStatus::from_execution(&result.status);
            let rec = SessionRecord {
                id: id.clone(),
                agent_id: agent_id.to_string(),
                agent_name: agent_name.to_string(),
                graph_id: String::new(),
                result: result.clone(),
                created_at: started_at,
                finished_at: Some(finished_at),
                duration_ms: Some((finished_at - started_at) * 1000.0),
                status: status.clone(),
                // Dónde quedó: solo tiene sentido si NO terminó el grafo.
                current_node_id: stopped_at_node(&result),
            };
            if let Err(e) = repo.save(&rec).await {
                tracing::warn!(session_id = %id, error = %e, "no se pudo persistir el run");
            }
            // Un run completado no tiene nada que reanudar: se suelta su
            // checkpoint (si no, la tabla crece con cada ejecución exitosa).
            if status == SessionStatus::Completed {
                if let Err(e) = repo.delete_checkpoint(&id).await {
                    tracing::warn!(session_id = %id, error = %e, "no se pudo soltar el checkpoint");
                }
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

/// Nodo en el que se quedó el run, o `None` si llegó al final.
///
/// Las pausas y cancelaciones lo traen explícito (`interrupt_node_id`); un
/// **fallo** no, y hasta 0.7.0 esos runs quedaban registrados sin nodo — o sea,
/// "falló" sin decir dónde, que es justo el dato que hace falta para reanudar
/// (PRD-021-B W5/W6). Se recupera del último nodo con error en la traza.
fn stopped_at_node(result: &ExecutionResult) -> Option<String> {
    if let Some(node_id) = &result.interrupt_node_id {
        return Some(node_id.clone());
    }
    result
        .trace
        .iter()
        .rev()
        .find(|t| t.status == crate::core::runner::TraceStatus::Error)
        .map(|t| t.node_id.clone())
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
    /// Filtro por estado del run (PRD-021-B): `running`, `paused`,
    /// `completed`, `failed`, `cancelled`, `timeout`, `interrupted`.
    /// Es lo que responde "¿qué quedó pausado?" sin traerse todo el histórico.
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
