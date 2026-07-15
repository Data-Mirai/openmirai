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
use crate::utils::short_id;

use super::helpers::{persist_memory_after_execution, run_agent_spec, run_agent_spec_streaming};
use super::state::{
    AgentCreateRequest, AppState, ErrorResponse, ExecuteRequest, GraphCreateRequest,
    SessionListQuery,
};
pub(crate) async fn health(State(state): State<AppState>) -> Json<Value> {
    let uptime_secs = state.start_time.elapsed().as_secs();
    let agents_count = state.agents.read().await.len();
    let sessions_count = state.sessions.read().await.len();
    let tools_count = state.tool_registry.list_tools().len();

    Json(json!({
        "status": "ok",
        "version": env!("MIRAI_VERSION"),
        "build": env!("MIRAI_BUILD"),
        "git_sha": env!("MIRAI_GIT_SHA"),
        "build_ts": env!("MIRAI_BUILD_TS"),
        "engine": "openmirai-engine-rs",
        "uptime_seconds": uptime_secs,
        "agents_loaded": agents_count,
        "sessions_total": sessions_count,
        "tools_registered": tools_count,
    }))
}

pub(crate) async fn version() -> Json<Value> {
    Json(json!({
        "version": env!("MIRAI_VERSION"),
        "build": env!("MIRAI_BUILD"),
        "git_sha": env!("MIRAI_GIT_SHA"),
        "build_ts": env!("MIRAI_BUILD_TS"),
        "engine": "openmirai-engine-rs"
    }))
}

// ---------------------------------------------------------------------------
// Graphs CRUD handlers
// ---------------------------------------------------------------------------

pub(crate) async fn create_graph(
    State(state): State<AppState>,
    Json(req): Json<GraphCreateRequest>,
) -> impl IntoResponse {
    let graph_id = short_id();

    let mut nodes = Vec::with_capacity(req.nodes.len());
    for (i, v) in req.nodes.into_iter().enumerate() {
        match serde_json::from_value::<NodeDef>(v) {
            Ok(n) => nodes.push(n),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid node at index {}: {}", i, e)})),
                )
                    .into_response();
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
                )
                    .into_response();
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
            )
                .into_response();
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

    let agent_id = short_id();

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
        schedule: None,
        triggers: Vec::new(),
        config: Default::default(),
        resources: Vec::new(),
        metadata: {
            let mut m = HashMap::new();
            m.insert("graph_id".to_string(), Value::String(req.graph_id.clone()));
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
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ErrorResponse {
                        error: format!(
                            "input validation failed: {}",
                            errors
                                .iter()
                                .map(|e| e.message.as_str())
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    }),
                ));
            }
        }
    } else {
        req.trigger_data.clone()
    };

    // PRD-008: Reject execute on live agents
    if spec.agent_type == crate::core::agent_spec::AgentType::Live {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "live agents are controlled via play/stop, not execute".to_string(),
            }),
        ));
    }

    // Run the agent graph with timeout protection.
    let timeout = std::time::Duration::from_secs(state.timeout_secs);
    let result =
        match tokio::time::timeout(timeout, run_agent_spec(&spec, &trigger_data, &state)).await {
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

    // PRD-008: persist memory after successful execution
    persist_memory_after_execution(&spec, &id, &result, &state).await;

    let session_id = short_id();
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
    )
        .into_response())
}

/// Create an agent directly from a full AgentSpec (no separate graph needed).
pub(crate) async fn create_agent_from_spec(
    State(state): State<AppState>,
    Json(spec): Json<AgentSpec>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let agent_id = short_id();
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
    let judge_model = req
        .get("judge_model")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let eval_types: Vec<crate::eval::EvalType> = req
        .get("eval_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.as_str().and_then(|s| match s {
                        "relevance" => Some(crate::eval::EvalType::Relevance),
                        "faithfulness" => Some(crate::eval::EvalType::Faithfulness),
                        "completeness" => Some(crate::eval::EvalType::Completeness),
                        "format_compliance" => Some(crate::eval::EvalType::FormatCompliance),
                        "latency" => Some(crate::eval::EvalType::Latency),
                        _ => None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if eval_types.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "eval_types array required (relevance, faithfulness, completeness, format_compliance, latency)".into(),
        })));
    }

    let llm = (state.llm_factory)();
    let results = crate::eval::execute_eval(
        &eval_types,
        crate::eval::EvalInput {
            input: input_text,
            output: output_text,
            context: context_text,
        },
        duration_ms,
        None,
        &*llm,
        judge_model,
    )
    .await;

    let scores: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "eval_type": r.eval_type,
                "score": r.score,
                "details": r.details,
                "judge_model": r.judge_model,
            })
        })
        .collect();

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
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "topic is required".into(),
            }),
        ));
    }
    let participants = match participants {
        Some(p) if p.len() >= 2 => p,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "participants array required (min 2 agents with name + personality)"
                        .into(),
                }),
            ))
        }
    };

    let llm = (state.llm_factory)();
    let mut transcript: Vec<Value> = Vec::new();

    for round in 0..max_rounds {
        let speaker_idx = round % participants.len();
        let speaker = &participants[speaker_idx];
        let name = speaker
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("agent");
        let personality = speaker
            .get("personality")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Build context: topic + transcript so far.
        let history: String = transcript
            .iter()
            .map(|t| {
                format!(
                    "[{}]: {}",
                    t["agent"].as_str().unwrap_or("?"),
                    t["content"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "You are {}. {}.\n\nTopic: {}\n\nPrevious discussion:\n{}\n\nGive your perspective in 2-3 sentences.",
            name, personality, topic, if history.is_empty() { "(none yet)".to_string() } else { history }
        );

        match llm.call("", &prompt, &[], 0.7, Some(256)).await {
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
    let completed = sessions
        .values()
        .filter(|r| r.status == ExecutionStatus::Completed)
        .count();
    let failed = sessions
        .values()
        .filter(|r| r.status == ExecutionStatus::Failed)
        .count();

    let all_traces: Vec<&TraceEntry> = sessions.values().flat_map(|r| r.trace.iter()).collect();
    let total_duration: u64 = all_traces.iter().map(|t| t.duration_ms).sum();
    let avg_duration = if all_traces.is_empty() {
        0.0
    } else {
        total_duration as f64 / all_traces.len() as f64
    };

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
            Json(ErrorResponse {
                error: "message is required".into(),
            }),
        ));
    }

    // Build Universe from agent_configs in request.
    let agent_configs = req.get("agents").and_then(|v| v.as_array());
    if agent_configs.is_none() || agent_configs.unwrap().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "agents array is required".into(),
            }),
        ));
    }

    let strategy_str = req
        .get("strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("keyword_match");
    let strategy = match strategy_str {
        "explicit" => crate::universe::RouterStrategy::Explicit,
        "round_robin" => crate::universe::RouterStrategy::RoundRobin,
        "llm_classify" => crate::universe::RouterStrategy::LlmClassify,
        _ => crate::universe::RouterStrategy::KeywordMatch,
    };

    let universe_config = crate::universe::UniverseConfig {
        name: req
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("universe")
            .to_string(),
        description: String::new(),
        router_strategy: strategy,
        router_prompt: None,
        default_response: "No agent available".into(),
    };

    let mut universe = crate::universe::Universe::new(universe_config);

    // Parse agents: each needs a soul (name + capabilities) and an agent_id.
    let agents_arr = agent_configs.unwrap();
    for agent_val in agents_arr {
        let name = agent_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed");
        let capabilities: Vec<String> = agent_val
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let agent_id = agent_val
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or(name);

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

/// Hash determinista de 64 bits, nunca cero.
///
/// Los IDs OTel deben ser hex y NUNCA all-zero (la spec los declara inválidos
/// y los collectors descartan la traza). Hasheamos en vez de truncar/rellenar
/// el nombre crudo: truncar node_ids parecidos colisionaba (rompía el árbol
/// de spans) y rellenar con ceros podía producir un ID all-zero.
/// Determinista a propósito: GETs repetidos del mismo trace devuelven los
/// mismos IDs.
fn otel_hash64(input: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    // `.max(1)` garantiza que nunca sea 0 (all-zero = inválido por spec).
    hasher.finish().max(1)
}

/// trace_id OTel: 16 bytes (32 hex) derivados del session_id.
fn get_trace_id(session_id: &str) -> String {
    let hi = otel_hash64(session_id);
    let lo = otel_hash64(&format!("{session_id}#lo"));
    format!("{hi:016x}{lo:016x}")
}

/// span_id OTel: 8 bytes (16 hex) derivados de name + índice en el trace.
/// El índice evita colisiones entre entradas con el mismo node_id.
fn get_span_id(name: &str, index: usize) -> String {
    format!("{:016x}", otel_hash64(&format!("{name}#{index}")))
}

pub(crate) async fn get_session_otel_trace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let sessions = state.sessions.read().await;
    match sessions.get(&id) {
        Some(result) => {
            let trace_id = get_trace_id(&id);
            // usize::MAX como índice reservado para el root: nunca colisiona
            // con los índices reales del trace (0..len).
            let root_span_id = get_span_id("root", usize::MAX);

            let mut total_duration_ms = 0;
            let mut spans = Vec::new();

            for (index, entry) in result.trace.iter().enumerate() {
                total_duration_ms += entry.duration_ms;

                let span_id = get_span_id(&entry.node_id, index);

                // Limitación conocida: TraceEntry solo guarda duration_ms (no
                // hay timestamps reales de inicio/fin), así que los spans se
                // fabrican en serie desde t=0. En grafos con nodos paralelos
                // esto infla el wall-time aparente y aplana el DAG a una
                // secuencia. Para timings fieles habría que capturar
                // start/end reales en el runner y usarlos aquí.
                let start_time_nano = (total_duration_ms - entry.duration_ms) * 1_000_000;
                let end_time_nano = total_duration_ms * 1_000_000;

                let status_code = match entry.status {
                    crate::core::runner::TraceStatus::Ok => "STATUS_CODE_OK",
                    crate::core::runner::TraceStatus::Error => "STATUS_CODE_ERROR",
                    crate::core::runner::TraceStatus::Skipped => "STATUS_CODE_UNSET",
                };

                let mut attributes = vec![
                    json!({ "key": "openmirai.node.id", "value": { "stringValue": entry.node_id } }),
                    json!({ "key": "openmirai.tool.type", "value": { "stringValue": entry.tool_type } }),
                    json!({ "key": "openmirai.retries", "value": { "intValue": entry.retries } }),
                ];

                if let Some(ref err) = entry.error {
                    attributes.push(json!({ "key": "openmirai.error", "value": { "stringValue": err.clone() } }));
                }

                spans.push(json!({
                    "traceId": trace_id,
                    "spanId": span_id,
                    "parentSpanId": root_span_id,
                    "name": format!("node:{}", entry.node_id),
                    "kind": "SPAN_KIND_INTERNAL",
                    "startTimeUnixNano": start_time_nano.to_string(),
                    "endTimeUnixNano": end_time_nano.to_string(),
                    "attributes": attributes,
                    "status": {
                        "code": status_code
                    }
                }));
            }

            // Add the graph root span
            let root_span = json!({
                "traceId": trace_id,
                "spanId": root_span_id,
                "name": format!("graph:{}", id),
                "kind": "SPAN_KIND_SERVER",
                "startTimeUnixNano": "0",
                "endTimeUnixNano": (total_duration_ms * 1_000_000).to_string(),
                "attributes": [
                    { "key": "openmirai.session.id", "value": { "stringValue": id.clone() } },
                    { "key": "openmirai.status", "value": { "stringValue": format!("{:?}", result.status) } }
                ],
                "status": {
                    "code": match result.status {
                        crate::core::runner::ExecutionStatus::Completed => "STATUS_CODE_OK",
                        _ => "STATUS_CODE_ERROR"
                    }
                }
            });

            let mut all_spans = vec![root_span];
            all_spans.extend(spans);

            let otel_json = json!({
                "resourceSpans": [
                    {
                        "resource": {
                            "attributes": [
                                {
                                    "key": "service.name",
                                    "value": { "stringValue": "openmirai-engine" }
                                }
                            ]
                        },
                        "scopeSpans": [
                            {
                                "scope": {
                                    "name": "openmirai.runner",
                                    // Versión real del build (VERSION en la raíz),
                                    // no un literal que se desactualiza.
                                    "version": env!("MIRAI_VERSION")
                                },
                                "spans": all_spans
                            }
                        ]
                    }
                ]
            });

            Ok(Json(otel_json))
        }
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

// ---------------------------------------------------------------------------
// PRD-008: Live agent play/stop/cycles/memory handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/agents/{id}/play — Start cycling a live agent.
pub(crate) async fn play_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let agents = state.agents.read().await;
    let spec = match agents.get(&id) {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".into(),
                }),
            ));
        }
    };
    drop(agents);

    // Must be a live agent
    if spec.agent_type != crate::core::agent_spec::AgentType::Live {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "only live agents support play/stop".into(),
            }),
        ));
    }

    // Must have a schedule
    let schedule = match &spec.schedule {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorResponse {
                    error: "live agent has no schedule configured".into(),
                }),
            ));
        }
    };

    // Check not already playing
    if state.scheduler.is_scheduled(&id).await {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "agent is already playing".into(),
            }),
        ));
    }

    let interval = schedule.interval_seconds.unwrap_or(60);
    let max_cycles = schedule.max_cycles;
    let on_error = match schedule.on_cycle_error {
        crate::core::agent_spec::CycleErrorMode::Continue => "continue",
        crate::core::agent_spec::CycleErrorMode::Stop => "stop",
    };

    // Clear cycle memory on new play session (persist: cycle resets)
    state.memory_store.clear_cycle_memory(&id).await;

    // Build the cycle callback that executes the agent's graph
    let agent_id = id.clone();
    let app_state = state.clone();
    let agent_spec = spec.clone();
    let callback: crate::runtime::scheduler::CycleCallback =
        std::sync::Arc::new(move |aid, cycle_num, is_first| {
            let s = app_state.clone();
            let sp = agent_spec.clone();
            Box::pin(async move {
                let trigger_data = {
                    let mut td = HashMap::new();
                    td.insert("cycle_number".to_string(), serde_json::json!(cycle_num));
                    td.insert("triggered_by".to_string(), serde_json::json!("scheduler"));
                    td
                };
                let result = super::helpers::run_agent_spec_with_memory(
                    &sp,
                    &trigger_data,
                    &s,
                    &aid,
                    is_first,
                )
                .await;

                // Persist memory after execution
                super::helpers::persist_memory_after_execution(&sp, &aid, &result, &s).await;

                match result.status {
                    ExecutionStatus::Completed => Ok(()),
                    _ => Err(result.error.unwrap_or_else(|| "cycle failed".into())),
                }
            })
        });

    state
        .scheduler
        .schedule_agent(&agent_id, interval, max_cycles, on_error, callback)
        .await;

    let memory_keys: Vec<String> = spec
        .graph
        .memory
        .as_ref()
        .map(|m| m.keys.keys().cloned().collect())
        .unwrap_or_default();

    Ok(Json(json!({
        "agent_id": id,
        "status": "playing",
        "schedule": { "interval_seconds": interval },
        "memory_keys": memory_keys,
    })))
}

/// POST /api/v1/agents/{id}/stop — Stop cycling a live agent.
pub(crate) async fn stop_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    if !state.scheduler.is_scheduled(&id).await {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "agent is not playing".into(),
            }),
        ));
    }

    let cycles_completed = state.scheduler.get_cycle_count(&id).await;
    state.scheduler.unschedule_agent(&id).await;

    Ok(Json(json!({
        "agent_id": id,
        "status": "enabled",
        "cycles_completed": cycles_completed,
    })))
}

/// GET /api/v1/agents/{id}/cycles — Get cycle history.
pub(crate) async fn get_agent_cycles(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let cycles = state.scheduler.get_cycles(&id, limit).await;
    let total = state.scheduler.get_cycle_count(&id).await;

    Ok(Json(json!({
        "agent_id": id,
        "total_cycles": total,
        "cycles": cycles,
    })))
}

/// GET /api/v1/agents/{id}/memory — Get current agent memory.
pub(crate) async fn get_agent_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let memory = state.memory_store.get_all_memory(&id).await;

    Ok(Json(json!({
        "agent_id": id,
        "memory": memory,
    })))
}

/// DELETE /api/v1/agents/{id}/memory — Clear agent memory to initial values.
pub(crate) async fn clear_agent_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    // Cannot clear while playing
    if state.scheduler.is_scheduled(&id).await {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "cannot clear memory while agent is playing".into(),
            }),
        ));
    }

    // Get initial values from spec
    let agents = state.agents.read().await;
    let initial_values = agents
        .get(&id)
        .and_then(|spec| spec.graph.memory.as_ref())
        .map(|m| m.keys.clone());
    drop(agents);

    state
        .memory_store
        .clear_all_memory(&id, initial_values.as_ref())
        .await;

    Ok(Json(json!({
        "agent_id": id,
        "memory": initial_values.unwrap_or_default(),
        "reset_to": "initial_values",
    })))
}
