//! HTTP handlers — all endpoint implementations.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::core::agent_spec::AgentSpec;
use crate::core::graph::{EdgeDef, GraphDef, NodeDef};
use crate::core::runner::{ExecutionStatus, TraceEntry};

use super::state::{AppState, ErrorResponse, ExecuteRequest, GraphCreateRequest, AgentCreateRequest, SessionListQuery};
use super::helpers::{run_agent_spec, run_agent_spec_streaming};
pub(crate) async fn health(State(state): State<AppState>) -> Json<Value> {
    let uptime_secs = state.start_time.elapsed().as_secs();
    let agents_count = state.agents.read().await.len();
    let sessions_count = state.sessions.read().await.len();
    let tools_count = state.tool_registry.list_tools().len();

    Json(json!({
        "status": "ok",
        "version": "0.4.1",
        "engine": "datamirai-engine-rs",
        "uptime_seconds": uptime_secs,
        "agents_loaded": agents_count,
        "sessions_total": sessions_count,
        "tools_registered": tools_count,
    }))
}

pub(crate) async fn version() -> Json<Value> {
    Json(json!({
        "version": "0.4.1",
        "engine": "datamirai-engine-rs"
    }))
}

// ---------------------------------------------------------------------------
// Graphs CRUD handlers
// ---------------------------------------------------------------------------

pub(crate) async fn create_graph(
    State(state): State<AppState>,
    Json(req): Json<GraphCreateRequest>,
) -> impl IntoResponse {
    let graph_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let mut nodes = Vec::with_capacity(req.nodes.len());
    for (i, v) in req.nodes.into_iter().enumerate() {
        match serde_json::from_value::<NodeDef>(v) {
            Ok(n) => nodes.push(n),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid node at index {}: {}", i, e)})),
                ).into_response();
            }
        }
    }

    let mut edges = Vec::with_capacity(req.edges.len());
    for (i, v) in req.edges.into_iter().enumerate() {
        match serde_json::from_value::<EdgeDef>(v) {
            Ok(e) => edges.push(e),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid edge at index {}: {}", i, e)})),
                ).into_response();
            }
        }
    }

    let graph = GraphDef {
        id: graph_id.clone(),
        name: req.name,
        version: "1.0.0".to_string(),
        nodes,
        edges,
        metadata: req.metadata,
    };

    let body = match serde_json::to_value(&graph) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("failed to serialize graph: {}", e)})),
            ).into_response();
        }
    };
    state.graphs.write().await.insert(graph_id, graph);

    (StatusCode::CREATED, Json(body)).into_response()
}

pub(crate) async fn list_graphs(State(state): State<AppState>) -> Json<Value> {
    let graphs = state.graphs.read().await;
    let list: Vec<Value> = graphs
        .values()
        .filter_map(|g| serde_json::to_value(g).ok())
        .collect();
    Json(json!(list))
}

pub(crate) async fn get_graph(
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

pub(crate) async fn delete_graph(
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

pub(crate) async fn create_agent(
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
        inputs: None,
        outputs: None,
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

pub(crate) async fn list_agents(State(state): State<AppState>) -> Json<Value> {
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

pub(crate) async fn get_agent(
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

pub(crate) async fn execute_agent(
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

    // Validate trigger_data against spec.inputs if defined (PRD-004 Capa 1)
    let trigger_data = if let Some(ref inputs_schema) = spec.inputs {
        match crate::core::agent_spec::validate_agent_inputs(&req.trigger_data, inputs_schema) {
            Ok(enriched) => enriched,
            Err(errors) => {
                let _details: Vec<Value> = errors.iter().map(|e| json!({
                    "field": e.field,
                    "error": e.error_type,
                    "message": e.message,
                })).collect();
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ErrorResponse {
                        error: format!("input validation failed: {}", errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; ")),
                    }),
                ));
            }
        }
    } else {
        req.trigger_data.clone()
    };

    // Run the agent graph with timeout protection.
    let timeout = std::time::Duration::from_secs(state.timeout_secs);
    let result = match tokio::time::timeout(timeout, run_agent_spec(&spec, &trigger_data, &state)).await {
        Ok(r) => r,
        Err(_) => {
            return Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(ErrorResponse {
                    error: format!("execution timed out after {}s", state.timeout_secs),
                }),
            ));
        }
    };

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

    state.insert_session(session_id, result).await;

    Ok(Json(body))
}

/// Execute an agent with SSE streaming response.
/// Execute an agent with REAL-TIME SSE streaming.
///
/// Events are emitted DURING execution via an mpsc channel.
/// The response streams events as they arrive — not post-execution replay.
pub(crate) async fn stream_agent(
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
    let _drain_handle = tokio::spawn(async move {
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
pub(crate) async fn create_agent_from_spec(
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
pub(crate) async fn list_tools(State(state): State<AppState>) -> Json<Value> {
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
pub(crate) async fn list_templates_handler() -> Json<Value> {
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

/// Execute eval on a session with REAL LLM judge.
pub(crate) async fn eval_session(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let input_text = req.get("input").and_then(|v| v.as_str()).unwrap_or("");
    let output_text = req.get("output").and_then(|v| v.as_str()).unwrap_or("");
    let context_text = req.get("context").and_then(|v| v.as_str());
    let duration_ms = req.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let judge_model = req.get("judge_model").and_then(|v| v.as_str()).unwrap_or("");

    let eval_types: Vec<crate::eval::EvalType> = req.get("eval_types")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().and_then(|s| match s {
            "relevance" => Some(crate::eval::EvalType::Relevance),
            "faithfulness" => Some(crate::eval::EvalType::Faithfulness),
            "completeness" => Some(crate::eval::EvalType::Completeness),
            "format_compliance" => Some(crate::eval::EvalType::FormatCompliance),
            "latency" => Some(crate::eval::EvalType::Latency),
            _ => None,
        })).collect())
        .unwrap_or_default();

    if eval_types.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "eval_types array required (relevance, faithfulness, completeness, format_compliance, latency)".into(),
        })));
    }

    let llm = (state.llm_factory)();
    let results = crate::eval::execute_eval(
        &eval_types, input_text, output_text, context_text,
        duration_ms, None, &*llm, judge_model,
    ).await;

    let scores: Vec<Value> = results.iter().map(|r| json!({
        "eval_type": r.eval_type,
        "score": r.score,
        "details": r.details,
        "judge_model": r.judge_model,
    })).collect();

    Ok(Json(json!({
        "results": scores,
        "eval_count": scores.len(),
    })))
}

/// Execute a GroupChat debate with REAL LLM.
pub(crate) async fn groupchat(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let topic = req.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let max_rounds = req.get("max_rounds").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let participants = req.get("participants").and_then(|v| v.as_array());

    if topic.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "topic is required".into() })));
    }
    let participants = match participants {
        Some(p) if p.len() >= 2 => p,
        _ => return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "participants array required (min 2 agents with name + personality)".into(),
        }))),
    };

    let llm = (state.llm_factory)();
    let mut transcript: Vec<Value> = Vec::new();

    for round in 0..max_rounds {
        let speaker_idx = round % participants.len();
        let speaker = &participants[speaker_idx];
        let name = speaker.get("name").and_then(|v| v.as_str()).unwrap_or("agent");
        let personality = speaker.get("personality").and_then(|v| v.as_str()).unwrap_or("");

        // Build context: topic + transcript so far.
        let history: String = transcript.iter().map(|t| {
            format!("[{}]: {}", t["agent"].as_str().unwrap_or("?"), t["content"].as_str().unwrap_or(""))
        }).collect::<Vec<_>>().join("\n");

        let prompt = format!(
            "You are {}. {}.\n\nTopic: {}\n\nPrevious discussion:\n{}\n\nGive your perspective in 2-3 sentences.",
            name, personality, topic, if history.is_empty() { "(none yet)".to_string() } else { history }
        );

        match llm.call("", &prompt, &[], 0.7, 256).await {
            Ok(response) => {
                transcript.push(json!({
                    "round": round + 1,
                    "agent": name,
                    "content": response.response,
                }));
            }
            Err(e) => {
                transcript.push(json!({
                    "round": round + 1,
                    "agent": name,
                    "content": format!("(error: {e})"),
                }));
            }
        }
    }

    Ok(Json(json!({
        "topic": topic,
        "rounds": max_rounds,
        "participants": participants.len(),
        "transcript": transcript,
    })))
}

/// Get aggregated metrics from all sessions.
pub(crate) async fn get_metrics(State(state): State<AppState>) -> Json<Value> {
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
pub(crate) async fn universe_message(
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

pub(crate) async fn get_agent_spec(
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

/// PRD-004: Return agent's input/output contract as JSON.
pub(crate) async fn get_agent_schema(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let agents = state.agents.read().await;
    match agents.get(&id) {
        Some(spec) => {
            let schema = json!({
                "name": spec.name,
                "version": spec.version,
                "description": spec.description,
                "inputs": spec.inputs,
                "outputs": spec.outputs,
            });
            Ok(Json(schema))
        }
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

pub(crate) async fn list_sessions(
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

pub(crate) async fn get_session(
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

pub(crate) async fn webhook_handler(
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

