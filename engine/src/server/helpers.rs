//! Execution helpers shared by agent handlers.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use tracing::warn;

use crate::adapters::{DefaultExecutionContext, InMemoryDBResource, InMemoryStorageResource};
use crate::core::agent_spec::{AgentSpec, MemoryPersistMode};
use crate::core::runner::{ExecutionResult, ExecutionStatus};
use crate::core::state::SharedState;
use crate::search::cosine_similarity;

use super::state::{AppState, ErrorResponse};

/// Build a SharedState pre-populated with agent memory (PRD-008).
///
/// Reads from the appropriate store based on persist mode and injects
/// as a virtual "memory" node in the state.
pub(crate) async fn build_state_with_memory(
    spec: &AgentSpec,
    agent_id: &str,
    app_state: &AppState,
    is_first_cycle_of_session: bool,
) -> SharedState {
    let shared_state = SharedState::new();

    let mem_spec = match &spec.graph.memory {
        Some(m) => m,
        None => return shared_state,
    };

    let memory_values = match mem_spec.persist {
        MemoryPersistMode::None => {
            // Always use initial values
            mem_spec.keys.clone()
        }
        MemoryPersistMode::Cycle => {
            if is_first_cycle_of_session {
                mem_spec.keys.clone()
            } else {
                let stored = app_state.memory_store.get_cycle_memory(agent_id).await;
                if stored.is_empty() {
                    mem_spec.keys.clone()
                } else {
                    merge_with_initials(&mem_spec.keys, &stored)
                }
            }
        }
        MemoryPersistMode::Execution => {
            let stored = app_state.memory_store.get_execution_memory(agent_id).await;
            if stored.is_empty() {
                mem_spec.keys.clone()
            } else {
                merge_with_initials(&mem_spec.keys, &stored)
            }
        }
    };

    // Inject as virtual node "memory" so ${memory.key} resolves
    let _ = shared_state.set("memory", memory_values, true);
    shared_state
}

/// Merge stored values with initial values (stored takes precedence for existing keys).
fn merge_with_initials(
    initials: &HashMap<String, Value>,
    stored: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    let mut merged = initials.clone();
    for (key, value) in stored {
        if initials.contains_key(key) {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged
}

pub(crate) async fn run_agent_spec(
    spec: &AgentSpec,
    trigger_data: &HashMap<String, Value>,
    state: &AppState,
    session_id: &str,
    run_agent_id: &str,
) -> ExecutionResult {
    run_agent_spec_with_memory(
        spec,
        trigger_data,
        state,
        &spec.name,
        true,
        session_id,
        run_agent_id,
    )
    .await
}

/// Run an agent spec with memory support (PRD-008).
///
/// `agent_id`: used to key the memory store.
/// `is_first_cycle_of_session`: controls cycle-mode memory behavior.
/// `session_id`: id del run, generado por el handler ANTES de ejecutar. El
/// contexto se construye con él para que los eventos, los logs y el registro
/// persistido hablen del mismo run (antes cada capa inventaba el suyo).
/// `run_agent_id`: id del agente con el que se **persiste** el run (el del
/// registro/la ruta). Va aparte de `agent_id` a propósito: ese es la llave del
/// store de memoria y en la ruta `/execute` vale `spec.name`, no el id — si se
/// unifican, se cambia el comportamiento de memoria de PRD-008.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_agent_spec_with_memory(
    spec: &AgentSpec,
    trigger_data: &HashMap<String, Value>,
    state: &AppState,
    agent_id: &str,
    is_first_cycle_of_session: bool,
    session_id: &str,
    run_agent_id: &str,
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
        if let Some(entry) = graph
            .nodes
            .iter_mut()
            .find(|n| n.tool_type.starts_with("trigger/"))
        {
            if let Ok(val) = serde_json::to_value(trigger_data) {
                entry.config.insert("payload".to_string(), val);
            }
        }
    }

    // Inject MCP server configs into mcp/call nodes.
    if !spec.config.mcp_servers.is_empty() {
        let mcp_val = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config
                    .insert("__mcp_servers".to_string(), mcp_val.clone());
            }
        }
    }

    // PRD-008: inject memory spec into state/memory nodes so they know
    // which keys are declared and what persist mode to use.
    if let Some(ref mem_spec) = spec.graph.memory {
        let mem_meta = serde_json::to_value(mem_spec).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "state/memory" {
                node.config
                    .insert("__memory_spec".to_string(), mem_meta.clone());
                node.config.insert(
                    "__agent_id".to_string(),
                    Value::String(agent_id.to_string()),
                );
            }
        }
    }

    // Resolve system prompt from Soul or spec.
    let system_prompt = if let Some(ref soul_path) = spec.soul {
        match crate::soul::load_from_file(std::path::Path::new(soul_path)) {
            Ok(soul) => Some(soul.to_system_prompt()),
            Err(e) => {
                warn!(soul_path = %soul_path, error = %e, "failed to load SOUL.md — falling back to system_prompt");
                spec.system_prompt.clone()
            }
        }
    } else {
        spec.system_prompt.clone()
    };

    // Create context with REAL LLM from the factory. Zero mocks.
    let llm = (state.llm_factory)();
    let mut ctx_builder = DefaultExecutionContext::builder(llm)
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()))
        .with_session_id(session_id);
    if let Some(ref prompt) = system_prompt {
        ctx_builder = ctx_builder.with_system_prompt(prompt);
    }
    let context = ctx_builder.build();

    // PRD-008: Build state with memory injected
    let initial_state =
        build_state_with_memory(spec, agent_id, state, is_first_cycle_of_session).await;

    // PRD-021-A: runner con checkpoints. Cada nodo terminado deja el estado en
    // SQLite, así que el run sobrevive a un reinicio del proceso.
    let runner = state.runner_with_checkpoints(run_agent_id, &spec.name, crate::utils::now_epoch());

    match runner.run_with_state(&graph, &context, initial_state).await {
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
///
/// Devuelve el `ExecutionResult` final (si el grafo llegó a correr) para que
/// el caller pueda registrar/persistir el run — los streams también son runs.
pub(crate) async fn run_agent_spec_streaming(
    spec: &AgentSpec,
    trigger_data: &HashMap<String, Value>,
    state: &AppState,
    session_id: &str,
    event_tx: tokio::sync::mpsc::Sender<crate::streaming::StreamEvent>,
    run_agent_id: &str,
) -> Option<ExecutionResult> {
    let mut graph = spec.to_graph(Some(&spec.name));
    graph.auto_generate_edge_ids();

    if let Err(e) = graph.validate() {
        let _ = event_tx
            .send(crate::streaming::StreamEvent::GraphError {
                error: format!("Graph validation failed: {e}"),
            })
            .await;
        return None;
    }

    // Same injections as run_agent_spec.
    if !trigger_data.is_empty() {
        if let Some(entry) = graph
            .nodes
            .iter_mut()
            .find(|n| n.tool_type.starts_with("trigger/"))
        {
            if let Ok(val) = serde_json::to_value(trigger_data) {
                entry.config.insert("payload".to_string(), val);
            }
        }
    }
    if !spec.config.mcp_servers.is_empty() {
        let mcp_val = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config
                    .insert("__mcp_servers".to_string(), mcp_val.clone());
            }
        }
    }

    let system_prompt = if let Some(ref soul_path) = spec.soul {
        match crate::soul::load_from_file(std::path::Path::new(soul_path)) {
            Ok(soul) => Some(soul.to_system_prompt()),
            Err(e) => {
                warn!(soul_path = %soul_path, error = %e, "failed to load SOUL.md — falling back to system_prompt");
                spec.system_prompt.clone()
            }
        }
    } else {
        spec.system_prompt.clone()
    };

    let llm = (state.llm_factory)();
    let mut ctx_builder = DefaultExecutionContext::builder(llm)
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()))
        .with_session_id(session_id);
    if let Some(ref prompt) = system_prompt {
        ctx_builder = ctx_builder.with_system_prompt(prompt);
    }
    let context = ctx_builder.build();

    // Runner con el canal de streaming Y con checkpoints (PRD-021-A): un run
    // observado en vivo es igual de reanudable que uno normal.
    let streaming_runner = state
        .runner_with_checkpoints(run_agent_id, &spec.name, crate::utils::now_epoch())
        .with_stream_tx(event_tx.clone());

    match streaming_runner.run(&graph, &context).await {
        Ok(result) => Some(result), // GraphCompleted already sent by runner
        Err(e) => {
            let _ = event_tx
                .send(crate::streaming::StreamEvent::GraphError {
                    error: e.to_string(),
                })
                .await;
            None
        }
    }
    // Channel drops when event_tx is dropped → receiver gets None → stream ends
}

/// Persist memory after a successful execution (PRD-008).
///
/// Reads `__memory_write` from any `state/memory` node's output in the state
/// and writes it to the appropriate store based on persist mode.
pub(crate) async fn persist_memory_after_execution(
    spec: &AgentSpec,
    agent_id: &str,
    result: &ExecutionResult,
    app_state: &AppState,
) {
    let mem_spec = match &spec.graph.memory {
        Some(m) => m,
        None => return,
    };

    // Only persist on success
    if result.status != ExecutionStatus::Completed {
        return;
    }

    // Find state/memory node outputs — look for __memory_write
    let state_snapshot = result.state.snapshot();
    let mut write_data: HashMap<String, Value> = HashMap::new();

    for node_output in state_snapshot.values() {
        if let Some(mw) = node_output.get("__memory_write") {
            if let Ok(data) = serde_json::from_value::<HashMap<String, Value>>(mw.clone()) {
                write_data.extend(data);
            }
        }
    }

    if write_data.is_empty() {
        return;
    }

    match mem_spec.persist {
        MemoryPersistMode::None => {
            // No-op
        }
        MemoryPersistMode::Cycle => {
            app_state
                .memory_store
                .set_cycle_memory(agent_id, write_data, &mem_spec.keys)
                .await;
        }
        MemoryPersistMode::Execution => {
            app_state
                .memory_store
                .set_execution_memory(agent_id, write_data, &mem_spec.keys)
                .await;
        }
    }
}

/// RAG search — chunk documents, embed with real LLM, search by cosine similarity.
pub(crate) async fn rag_search(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let query = req.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let documents = req.get("documents").and_then(|v| v.as_array());
    let top_k = req.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let chunk_strategy = req
        .get("chunk_strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("paragraph");

    if query.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "query is required".into(),
            }),
        ));
    }
    if documents.is_none() || documents.unwrap().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "documents array is required".into(),
            }),
        ));
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
        chunk_size: req
            .get("chunk_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(512) as usize,
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to embed query: {e}"),
            }),
        )
    })?;

    let mut chunk_embeddings = Vec::new();
    for chunk in &all_chunks {
        match llm.embed(chunk, "nomic-embed-text").await {
            Ok(emb) => chunk_embeddings.push(emb),
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to embed chunk: {e}"),
                    }),
                ));
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
