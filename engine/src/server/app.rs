use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use crate::core::agent_spec::AgentSpec;
use crate::core::context::LLMResource;
use crate::core::graph::{EdgeDef, GraphDef, NodeDef};
use crate::core::runner::{ExecutionResult, ExecutionStatus, GraphRunner, TraceEntry};
use crate::tools::registry::RegistryExecutor;
use crate::core::state::SharedState;
use crate::resources::{SimpleExecutionContext, InMemoryDBResource, InMemoryStorageResource};
use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Shared application state passed to all handlers via axum's `State`.
///
/// Uses `Arc<RwLock<HashMap>>` for in-memory storage. A real DB layer will
/// replace this later.
/// Factory function that creates a real LLMResource for each execution.
/// This is set once at server startup and cloned per-request.
pub type LLMFactory = Arc<dyn Fn() -> Box<dyn LLMResource> + Send + Sync>;

#[derive(Clone)]
pub struct AppState {
    pub graphs: Arc<RwLock<HashMap<String, GraphDef>>>,
    pub agents: Arc<RwLock<HashMap<String, AgentSpec>>>,
    pub sessions: Arc<RwLock<HashMap<String, ExecutionResult>>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub runner: GraphRunner,
    pub llm_factory: LLMFactory,
    pub start_time: std::time::Instant,
}

impl AppState {
    /// Create app state with a real LLM factory. NO MOCKS.
    pub fn new(tool_registry: ToolRegistry, llm_factory: LLMFactory) -> Self {
        let registry = Arc::new(tool_registry);
        let executor = RegistryExecutor::new(registry.clone());
        let runner = GraphRunner::new(Box::new(executor));
        Self {
            graphs: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            tool_registry: registry,
            runner,
            llm_factory,
            start_time: std::time::Instant::now(),
        }
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

// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------

/// Build the axum router with all endpoints wired up.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health / version
        .route("/health", get(health))
        .route("/version", get(version))
        // Graphs CRUD
        .route("/api/graphs", post(create_graph).get(list_graphs))
        .route(
            "/api/graphs/{id}",
            get(get_graph).delete(delete_graph),
        )
        // Agents CRUD
        .route("/api/agents", post(create_agent).get(list_agents))
        .route("/api/agents/from-spec", post(create_agent_from_spec))
        .route("/api/agents/{id}", get(get_agent))
        .route("/api/agents/{id}/execute", post(execute_agent))
        .route("/api/agents/{id}/stream", post(stream_agent))
        .route("/api/agents/{id}/spec", get(get_agent_spec))
        // Tools
        .route("/api/tools", get(list_tools))
        // Templates
        .route("/api/templates", get(list_templates_handler))
        // Sessions
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
        // Universe
        .route("/api/universe/message", post(universe_message))
        // Metrics
        .route("/api/metrics", get(get_metrics))
        // RAG
        .route("/api/rag/search", post(rag_search))
        // Webhooks
        .route("/webhooks/{*path}", post(webhook_handler))
        // Middleware
        .layer(CorsLayer::permissive())
        // State
        .with_state(state)
}

// ---------------------------------------------------------------------------
// serve()
// ---------------------------------------------------------------------------

/// Start the HTTP server with a REAL LLM factory. No mocks.
///
/// The `llm_factory` creates a new `Box<dyn LLMResource>` for each agent execution.
/// The caller (CLI) is responsible for configuring the factory with the right
/// provider, model, and API key based on user flags / env vars.
pub async fn serve(
    host: &str,
    port: u16,
    llm_factory: LLMFactory,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let state = AppState::new(registry, llm_factory);
    let app = create_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("datamirai-engine listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Health / Version handlers
// ---------------------------------------------------------------------------

async fn health(State(state): State<AppState>) -> Json<Value> {
    let uptime_secs = state.start_time.elapsed().as_secs();
    let agents_count = state.agents.read().await.len();
    let sessions_count = state.sessions.read().await.len();
    let tools_count = state.tool_registry.list_tools().len();

    Json(json!({
        "status": "ok",
        "version": "0.2.0",
        "engine": "datamirai-engine-rs",
        "uptime_seconds": uptime_secs,
        "agents_loaded": agents_count,
        "sessions_total": sessions_count,
        "tools_registered": tools_count,
    }))
}

async fn version() -> Json<Value> {
    Json(json!({
        "version": "0.2.0",
        "engine": "datamirai-engine-rs"
    }))
}

// ---------------------------------------------------------------------------
// Graphs CRUD handlers
// ---------------------------------------------------------------------------

async fn create_graph(
    State(state): State<AppState>,
    Json(req): Json<GraphCreateRequest>,
) -> impl IntoResponse {
    let graph_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let nodes: Vec<NodeDef> = req
        .nodes
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    let edges: Vec<EdgeDef> = req
        .edges
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    let graph = GraphDef {
        id: graph_id.clone(),
        name: req.name,
        version: "1.0.0".to_string(),
        nodes,
        edges,
        metadata: req.metadata,
    };

    let body = serde_json::to_value(&graph).unwrap_or(json!({}));
    state.graphs.write().await.insert(graph_id, graph);

    (StatusCode::CREATED, Json(body))
}

async fn list_graphs(State(state): State<AppState>) -> Json<Value> {
    let graphs = state.graphs.read().await;
    let list: Vec<Value> = graphs
        .values()
        .filter_map(|g| serde_json::to_value(g).ok())
        .collect();
    Json(json!(list))
}

async fn get_graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let graphs = state.graphs.read().await;
    match graphs.get(&id) {
        Some(g) => Ok(Json(serde_json::to_value(g).unwrap_or(json!({})))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Graph not found".to_string(),
            }),
        )),
    }
}

async fn delete_graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let mut graphs = state.graphs.write().await;
    match graphs.remove(&id) {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Graph not found".to_string(),
            }),
        )),
    }
}

// ---------------------------------------------------------------------------
// Agents CRUD handlers
// ---------------------------------------------------------------------------

async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<AgentCreateRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    // Verify graph exists.
    let graphs = state.graphs.read().await;
    if !graphs.contains_key(&req.graph_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Graph '{}' not found", req.graph_id),
            }),
        ));
    }
    drop(graphs);

    let agent_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let spec = AgentSpec {
        name: req.name.clone(),
        description: req.description.unwrap_or_default(),
        version: "v1".to_string(),
        agent_type: Default::default(),
        system_prompt: None,
        soul: None,
        graph: Default::default(),
        triggers: Vec::new(),
        config: Default::default(),
        resources: Vec::new(),
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "graph_id".to_string(),
                Value::String(req.graph_id.clone()),
            );
            m.insert("agent_id".to_string(), Value::String(agent_id.clone()));
            m
        },
    };

    let body = json!({
        "id": agent_id,
        "name": req.name,
        "graph_id": req.graph_id,
        "status": "created",
    });

    state.agents.write().await.insert(agent_id, spec);

    Ok((StatusCode::CREATED, Json(body)))
}

async fn list_agents(State(state): State<AppState>) -> Json<Value> {
    let agents = state.agents.read().await;
    let list: Vec<Value> = agents
        .iter()
        .map(|(id, spec)| {
            let graph_id = spec
                .metadata
                .get("graph_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            json!({
                "id": id,
                "name": spec.name,
                "description": spec.description,
                "graph_id": graph_id,
            })
        })
        .collect();
    Json(json!(list))
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let agents = state.agents.read().await;
    match agents.get(&id) {
        Some(spec) => {
            let graph_id = spec
                .metadata
                .get("graph_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(Json(json!({
                "id": id,
                "name": spec.name,
                "description": spec.description,
                "graph_id": graph_id,
            })))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Agent not found".to_string(),
            }),
        )),
    }
}

async fn execute_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let agents = state.agents.read().await;
    let spec = match agents.get(&id) {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            ))
        }
    };
    drop(agents);

    // Run the agent graph with real execution.
    let result = run_agent_spec(&spec, &req.trigger_data, &state).await;

    let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let body = json!({
        "session_id": &session_id,
        "agent_id": id,
        "agent_name": spec.name,
        "status": result.status,
        "trace": result.trace,
        "transcript": result.transcript,
        "state": result.state.snapshot(),
        "error": result.error,
    });

    state.sessions.write().await.insert(session_id, result);

    Ok(Json(body))
}

/// Execute an agent with SSE streaming response.
/// Execute an agent with REAL-TIME SSE streaming.
///
/// Events are emitted DURING execution via an mpsc channel.
/// The response streams events as they arrive — not post-execution replay.
async fn stream_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteRequest>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    use axum::body::Body;
    use axum::response::IntoResponse;
    use tokio_stream::wrappers::ReceiverStream;

    let agents = state.agents.read().await;
    let spec = match agents.get(&id) {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            ))
        }
    };
    drop(agents);

    // Create channel for real-time events.
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<crate::streaming::StreamEvent>(256);

    // Create a byte-stream channel for the HTTP response.
    let (byte_tx, byte_rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256);

    // Spawn a task that drains events and converts to SSE text.
    let drain_handle = tokio::spawn(async move {
        let mut rx = event_rx;
        while let Some(event) = rx.recv().await {
            let sse_text = event.to_sse();
            if byte_tx.send(Ok(sse_text)).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    // Spawn the actual agent execution with the stream channel.
    let state_clone = state.clone();
    let spec_clone = spec.clone();
    let trigger_data = req.trigger_data.clone();
    tokio::spawn(async move {
        run_agent_spec_streaming(&spec_clone, &trigger_data, &state_clone, event_tx).await;
    });

    // Stream the byte channel as the HTTP response body.
    let stream = ReceiverStream::new(byte_rx);
    let body = Body::from_stream(stream);

    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("connection", "keep-alive"),
        ],
        body,
    ).into_response())
}

/// Create an agent directly from a full AgentSpec (no separate graph needed).
async fn create_agent_from_spec(
    State(state): State<AppState>,
    Json(spec): Json<AgentSpec>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let agent_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let name = spec.name.clone();

    state.agents.write().await.insert(agent_id.clone(), spec);

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": agent_id,
            "name": name,
            "status": "created",
        })),
    ))
}

/// List all registered tools with their specs.
async fn list_tools(State(state): State<AppState>) -> Json<Value> {
    let tools: Vec<Value> = state
        .tool_registry
        .list_tools()
        .iter()
        .map(|spec| {
            json!({
                "tool_type": spec.tool_type,
                "name": spec.name,
                "description": spec.description,
                "category": spec.category,
                "inputs": spec.inputs.iter().map(|f| json!({
                    "name": f.name,
                    "type": format!("{:?}", f.field_type),
                    "required": f.required,
                    "description": f.description,
                })).collect::<Vec<_>>(),
                "outputs": spec.outputs.iter().map(|f| json!({
                    "name": f.name,
                    "type": format!("{:?}", f.field_type),
                    "description": f.description,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(json!(tools))
}

/// List available agent templates.
async fn list_templates_handler() -> Json<Value> {
    let templates: Vec<Value> = crate::templates::builtin_templates()
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "category": t.category,
                "description": t.description,
                "required_providers": t.required_providers,
                "tags": t.tags,
            })
        })
        .collect();
    Json(json!(templates))
}

/// Internal: run an AgentSpec through the GraphRunner.
async fn run_agent_spec(
    spec: &AgentSpec,
    trigger_data: &HashMap<String, Value>,
    state: &AppState,
) -> ExecutionResult {
    let mut graph = spec.to_graph(Some(&spec.name));
    graph.auto_generate_edge_ids();

    if let Err(e) = graph.validate() {
        return ExecutionResult {
            status: ExecutionStatus::Failed,
            state: SharedState::new(),
            trace: vec![],
            transcript: vec![],
            error: Some(format!("Graph validation failed: {e}")),
            interrupt_node_id: None,
            interrupt_info: None,
        };
    }

    // Inject trigger data into entry node.
    if !trigger_data.is_empty() {
        if let Some(entry) = graph.nodes.iter_mut().find(|n| n.tool_type.starts_with("trigger/")) {
            if let Ok(val) = serde_json::to_value(trigger_data) {
                entry.config.insert("mock_payload".to_string(), val);
            }
        }
    }

    // Inject MCP server configs into mcp/call nodes.
    if !spec.config.mcp_servers.is_empty() {
        let mcp_val = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config.insert("__mcp_servers".to_string(), mcp_val.clone());
            }
        }
    }

    // Resolve system prompt from Soul or spec.
    let system_prompt = if let Some(ref soul_path) = spec.soul {
        crate::soul::load_from_file(std::path::Path::new(soul_path))
            .ok()
            .map(|s| s.to_system_prompt())
            .or(spec.system_prompt.clone())
    } else {
        spec.system_prompt.clone()
    };

    // Create context with REAL LLM from the factory. Zero mocks.
    let llm = (state.llm_factory)();
    let mut ctx_builder = SimpleExecutionContext::builder(llm)
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()));
    if let Some(ref prompt) = system_prompt {
        ctx_builder = ctx_builder.with_system_prompt(prompt);
    }
    let context = ctx_builder.build();

    match state.runner.run(&graph, &context).await {
        Ok(result) => result,
        Err(e) => ExecutionResult {
            status: ExecutionStatus::Failed,
            state: SharedState::new(),
            trace: vec![],
            transcript: vec![],
            error: Some(e.to_string()),
            interrupt_node_id: None,
            interrupt_info: None,
        },
    }
}

/// Internal: run an AgentSpec with real-time streaming via mpsc channel.
async fn run_agent_spec_streaming(
    spec: &AgentSpec,
    trigger_data: &HashMap<String, Value>,
    state: &AppState,
    event_tx: tokio::sync::mpsc::Sender<crate::streaming::StreamEvent>,
) {
    let mut graph = spec.to_graph(Some(&spec.name));
    graph.auto_generate_edge_ids();

    if let Err(e) = graph.validate() {
        let _ = event_tx.send(crate::streaming::StreamEvent::GraphError {
            error: format!("Graph validation failed: {e}"),
        }).await;
        return;
    }

    // Same injections as run_agent_spec.
    if !trigger_data.is_empty() {
        if let Some(entry) = graph.nodes.iter_mut().find(|n| n.tool_type.starts_with("trigger/")) {
            if let Ok(val) = serde_json::to_value(trigger_data) {
                entry.config.insert("mock_payload".to_string(), val);
            }
        }
    }
    if !spec.config.mcp_servers.is_empty() {
        let mcp_val = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config.insert("__mcp_servers".to_string(), mcp_val.clone());
            }
        }
    }

    let system_prompt = if let Some(ref soul_path) = spec.soul {
        crate::soul::load_from_file(std::path::Path::new(soul_path))
            .ok()
            .map(|s| s.to_system_prompt())
            .or(spec.system_prompt.clone())
    } else {
        spec.system_prompt.clone()
    };

    let llm = (state.llm_factory)();
    let mut ctx_builder = SimpleExecutionContext::builder(llm)
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()));
    if let Some(ref prompt) = system_prompt {
        ctx_builder = ctx_builder.with_system_prompt(prompt);
    }
    let context = ctx_builder.build();

    // Create a runner WITH the stream channel for real-time events.
    let streaming_runner = state.runner.clone().with_stream_tx(event_tx.clone());

    match streaming_runner.run(&graph, &context).await {
        Ok(_) => {} // GraphCompleted already sent by runner
        Err(e) => {
            let _ = event_tx.send(crate::streaming::StreamEvent::GraphError {
                error: e.to_string(),
            }).await;
        }
    }
    // Channel drops when event_tx is dropped → receiver gets None → stream ends
}

/// RAG search — chunk documents, embed with real LLM, search by cosine similarity.
async fn rag_search(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let query = req.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let documents = req.get("documents").and_then(|v| v.as_array());
    let top_k = req.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let chunk_strategy = req.get("chunk_strategy").and_then(|v| v.as_str()).unwrap_or("paragraph");

    if query.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "query is required".into() })));
    }
    if documents.is_none() || documents.unwrap().is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "documents array is required".into() })));
    }

    // Chunk all documents.
    let rag_config = crate::rag::RAGPipelineConfig {
        name: "search".into(),
        source_type: crate::rag::SourceType::Text,
        chunking_strategy: match chunk_strategy {
            "fixed_size" => crate::rag::ChunkingStrategy::FixedSize,
            "sentence" => crate::rag::ChunkingStrategy::Sentence,
            _ => crate::rag::ChunkingStrategy::Paragraph,
        },
        chunk_size: req.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(512) as usize,
        chunk_overlap: 50,
        embedding_model: "nomic-embed-text".into(),
    };

    let mut all_chunks = Vec::new();
    for doc_val in documents.unwrap() {
        let text = doc_val.as_str().unwrap_or("");
        if !text.is_empty() {
            let chunks = crate::rag::chunk_text(text, &rag_config);
            all_chunks.extend(chunks);
        }
    }

    if all_chunks.is_empty() {
        return Ok(Json(json!({"results": [], "chunks_total": 0})));
    }

    // Generate embeddings for all chunks + query using REAL Ollama.
    let llm = (state.llm_factory)();

    let query_embedding = llm.embed(query, "nomic-embed-text").await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: format!("Failed to embed query: {e}"),
        }))
    })?;

    let mut chunk_embeddings = Vec::new();
    for chunk in &all_chunks {
        match llm.embed(chunk, "nomic-embed-text").await {
            Ok(emb) => chunk_embeddings.push(emb),
            Err(e) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                    error: format!("Failed to embed chunk: {e}"),
                })));
            }
        }
    }

    // Cosine similarity search.
    let mut scored: Vec<(usize, f64)> = chunk_embeddings
        .iter()
        .enumerate()
        .map(|(i, emb)| (i, cosine_similarity(&query_embedding, emb)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let results: Vec<Value> = scored
        .iter()
        .take(top_k)
        .map(|(i, score)| {
            json!({
                "chunk": all_chunks[*i],
                "score": score,
                "index": i,
            })
        })
        .collect();

    Ok(Json(json!({
        "results": results,
        "chunks_total": all_chunks.len(),
        "query": query,
        "embedding_model": "nomic-embed-text",
        "dimensions": query_embedding.len(),
    })))
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Get aggregated metrics from all sessions.
async fn get_metrics(State(state): State<AppState>) -> Json<Value> {
    let sessions = state.sessions.read().await;

    let total = sessions.len();
    let completed = sessions.values().filter(|r| r.status == ExecutionStatus::Completed).count();
    let failed = sessions.values().filter(|r| r.status == ExecutionStatus::Failed).count();

    let all_traces: Vec<&TraceEntry> = sessions.values().flat_map(|r| r.trace.iter()).collect();
    let total_duration: u64 = all_traces.iter().map(|t| t.duration_ms).sum();
    let avg_duration = if all_traces.is_empty() { 0.0 } else { total_duration as f64 / all_traces.len() as f64 };

    Json(json!({
        "sessions": {
            "total": total,
            "completed": completed,
            "failed": failed,
        },
        "nodes": {
            "total_executed": all_traces.len(),
            "total_duration_ms": total_duration,
            "avg_duration_ms": avg_duration,
        },
        "tools_registered": state.tool_registry.list_tools().len(),
        "agents_loaded": state.agents.read().await.len(),
    }))
}

/// Send a message to a Universe — routes to best agent and executes.
async fn universe_message(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let message = req.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "message is required".into() }),
        ));
    }

    // Build Universe from agent_configs in request.
    let agent_configs = req.get("agents").and_then(|v| v.as_array());
    if agent_configs.is_none() || agent_configs.unwrap().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "agents array is required".into() }),
        ));
    }

    let strategy_str = req.get("strategy").and_then(|v| v.as_str()).unwrap_or("keyword_match");
    let strategy = match strategy_str {
        "explicit" => crate::universe::RouterStrategy::Explicit,
        "round_robin" => crate::universe::RouterStrategy::RoundRobin,
        "llm_classify" => crate::universe::RouterStrategy::LlmClassify,
        _ => crate::universe::RouterStrategy::KeywordMatch,
    };

    let universe_config = crate::universe::UniverseConfig {
        name: req.get("name").and_then(|v| v.as_str()).unwrap_or("universe").to_string(),
        description: String::new(),
        router_strategy: strategy,
        router_prompt: None,
        default_response: "No agent available".into(),
    };

    let mut universe = crate::universe::Universe::new(universe_config);

    // Parse agents: each needs a soul (name + capabilities) and an agent_id.
    let agents_arr = agent_configs.unwrap();
    for agent_val in agents_arr {
        let name = agent_val.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed");
        let capabilities: Vec<String> = agent_val.get("capabilities")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let agent_id = agent_val.get("agent_id").and_then(|v| v.as_str()).unwrap_or(name);

        let soul = crate::soul::Soul {
            name: name.to_string(),
            identity: String::new(),
            personality: String::new(),
            capabilities,
            constraints: vec![],
            workflows: vec![],
            knowledge_refs: vec![],
            context: String::new(),
        };
        universe.add_agent(soul, agent_id);
    }

    // Route the message.
    let decision = universe.route(message);

    // Execute the selected agent if it exists in our loaded agents.
    let agents = state.agents.read().await;
    let agent_result = if let Some(spec) = agents.get(&decision.agent_id) {
        let spec = spec.clone();
        drop(agents);

        let mut trigger_data = HashMap::new();
        trigger_data.insert("message".to_string(), Value::String(message.to_string()));

        let result = run_agent_spec(&spec, &trigger_data, &state).await;
        Some(json!({
            "status": result.status,
            "state": result.state.snapshot(),
            "error": result.error,
        }))
    } else {
        drop(agents);
        None
    };

    Ok(Json(json!({
        "routing": {
            "agent_name": decision.agent_name,
            "agent_id": decision.agent_id,
            "confidence": decision.confidence,
            "strategy": decision.strategy_used,
            "reason": decision.reason,
        },
        "executed": agent_result.is_some(),
        "result": agent_result,
    })))
}

async fn get_agent_spec(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let agents = state.agents.read().await;
    match agents.get(&id) {
        Some(spec) => Ok(Json(serde_json::to_value(spec).unwrap_or(json!({})))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Agent not found".to_string(),
            }),
        )),
    }
}

// ---------------------------------------------------------------------------
// Sessions handlers
// ---------------------------------------------------------------------------

async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<SessionListQuery>,
) -> Json<Value> {
    let sessions = state.sessions.read().await;
    let limit = params.limit.unwrap_or(50);

    let list: Vec<Value> = sessions
        .iter()
        .take(limit)
        .map(|(id, result)| {
            json!({
                "id": id,
                "status": result.status,
                "trace_len": result.trace.len(),
                "error": result.error,
            })
        })
        .collect();

    Json(json!(list))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let sessions = state.sessions.read().await;
    match sessions.get(&id) {
        Some(result) => Ok(Json(json!({
            "id": id,
            "status": result.status,
            "trace": result.trace,
            "error": result.error,
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Session not found".to_string(),
            }),
        )),
    }
}

// ---------------------------------------------------------------------------
// Webhooks handler
// ---------------------------------------------------------------------------

async fn webhook_handler(
    State(_state): State<AppState>,
    Path(path): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    // Placeholder: in the full implementation this would route to the agent
    // whose trigger matches `/webhooks/{path}`.
    tracing::info!(path = %path, "webhook received");

    Ok(Json(json!({
        "received": true,
        "path": path,
        "body": body,
    })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Mock LLM factory for unit tests ONLY (#[cfg(test)]).
    fn test_llm_factory() -> LLMFactory {
        use crate::resources::MockLLMResource;
        Arc::new(|| Box::new(MockLLMResource::new()))
    }

    fn test_app() -> Router {
        let state = AppState::new(ToolRegistry::new(), test_llm_factory());
        create_router(state)
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn version_returns_engine_info() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/version").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["engine"], "datamirai-engine-rs");
    }

    #[tokio::test]
    async fn graph_crud_lifecycle() {
        let state = AppState::new(ToolRegistry::new(), test_llm_factory());
        let app = create_router(state.clone());

        // Create
        let create_body = json!({
            "name": "test-graph",
            "nodes": [{"id": "n1", "tool_type": "ai/llm_call"}],
            "edges": []
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/graphs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp.into_body()).await;
        let graph_id = created["id"].as_str().unwrap().to_string();

        // List
        let resp = app
            .clone()
            .oneshot(Request::get("/api/graphs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp.into_body()).await;
        assert_eq!(list.as_array().unwrap().len(), 1);

        // Get
        let resp = app
            .clone()
            .oneshot(
                Request::get(&format!("/api/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Delete
        let resp = app
            .clone()
            .oneshot(
                Request::delete(&format!("/api/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Get after delete -> 404
        let resp = app
            .oneshot(
                Request::get(&format!("/api/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_crud_lifecycle() {
        let state = AppState::new(ToolRegistry::new(), test_llm_factory());
        let app = create_router(state.clone());

        // First create a graph
        let graph_body = json!({
            "name": "agent-graph",
            "nodes": [{"id": "n1", "tool_type": "ai/llm_call"}],
            "edges": []
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/graphs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&graph_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let created_graph = body_json(resp.into_body()).await;
        let graph_id = created_graph["id"].as_str().unwrap().to_string();

        // Create agent
        let agent_body = json!({
            "name": "test-agent",
            "graph_id": graph_id,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&agent_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created_agent = body_json(resp.into_body()).await;
        let agent_id = created_agent["id"].as_str().unwrap().to_string();

        // List agents
        let resp = app
            .clone()
            .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp.into_body()).await;
        assert_eq!(list.as_array().unwrap().len(), 1);

        // Get agent
        let resp = app
            .clone()
            .oneshot(
                Request::get(&format!("/api/agents/{agent_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn execute_agent_from_spec() {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let state = AppState::new(registry, test_llm_factory());
        let app = create_router(state.clone());

        // Create agent via from-spec with a simple trigger→response graph
        let spec = json!({
            "name": "test-exec",
            "description": "test",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"mock_payload": {"msg": "hello"}}},
                    {"id": "out", "tool_type": "output/response", "config": {"message": "done"}}
                ],
                "edges": [
                    {"source": "trigger", "target": "out"}
                ]
            }
        });

        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/agents/from-spec")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp.into_body()).await;
        let agent_id = created["id"].as_str().unwrap().to_string();

        // Execute
        let exec_body = json!({ "trigger_data": {} });
        let resp = app
            .clone()
            .oneshot(
                Request::post(&format!("/api/agents/{agent_id}/execute"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&exec_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let result = body_json(resp.into_body()).await;
        assert_eq!(result["status"], "Completed");
        assert_eq!(result["agent_id"], agent_id);
    }

    #[tokio::test]
    async fn list_tools_returns_all_builtins() {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let state = AppState::new(registry, test_llm_factory());
        let app = create_router(state);

        let resp = app
            .oneshot(Request::get("/api/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let tools = body_json(resp.into_body()).await;
        let arr = tools.as_array().unwrap();
        assert!(arr.len() >= 40, "Expected 40+ tools, got {}", arr.len());
    }

    #[tokio::test]
    async fn create_agent_with_invalid_graph_returns_404() {
        let app = test_app();
        let body = json!({ "name": "orphan", "graph_id": "nonexistent" });
        let resp = app
            .oneshot(
                Request::post("/api/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_nonexistent_session_returns_404() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::get("/api/sessions/ghost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn webhook_returns_received() {
        let app = test_app();
        let body = json!({ "event": "push" });
        let resp = app
            .oneshot(
                Request::post("/webhooks/my-hook")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["received"], true);
        assert_eq!(json["path"], "my-hook");
    }
}
