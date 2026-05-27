//! Execution helpers shared by agent handlers.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use tracing::warn;

use crate::core::agent_spec::AgentSpec;
use crate::core::context::LLMResource;
use crate::core::graph::{EdgeDef, GraphDef, NodeDef};
use crate::core::runner::{ExecutionResult, ExecutionStatus, GraphRunner, TraceEntry};
use crate::core::state::SharedState;
use crate::adapters::{InMemoryDBResource, InMemoryStorageResource, SimpleExecutionContext};
use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::{RegistryExecutor, ToolRegistry};

use super::state::{AppState, ErrorResponse};

pub(crate) async fn run_agent_spec(
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
                entry.config.insert("payload".to_string(), val);
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
pub(crate) async fn run_agent_spec_streaming(
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
                entry.config.insert("payload".to_string(), val);
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
pub(crate) async fn rag_search(
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
pub(crate) fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
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

