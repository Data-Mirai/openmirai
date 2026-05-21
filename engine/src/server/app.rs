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
use crate::core::graph::{EdgeDef, GraphDef, NodeDef};
use crate::core::runner::{ExecutionResult, ExecutionStatus, TraceEntry};
use crate::core::state::SharedState;
use crate::tools::registry::ToolRegistry;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Shared application state passed to all handlers via axum's `State`.
///
/// Uses `Arc<RwLock<HashMap>>` for in-memory storage. A real DB layer will
/// replace this later.
#[derive(Clone)]
pub struct AppState {
    pub graphs: Arc<RwLock<HashMap<String, GraphDef>>>,
    pub agents: Arc<RwLock<HashMap<String, AgentSpec>>>,
    pub sessions: Arc<RwLock<HashMap<String, ExecutionResult>>>,
    pub tool_registry: Arc<ToolRegistry>,
}

impl AppState {
    pub fn new(tool_registry: ToolRegistry) -> Self {
        Self {
            graphs: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            tool_registry: Arc::new(tool_registry),
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
        .route("/api/agents/{id}", get(get_agent))
        .route("/api/agents/{id}/execute", post(execute_agent))
        .route("/api/agents/{id}/spec", get(get_agent_spec))
        // Sessions
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
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

/// Start the HTTP server on the given host and port.
pub async fn serve(host: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::new(ToolRegistry::new());
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

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn version() -> Json<Value> {
    Json(json!({
        "version": "0.1.0",
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

    // Placeholder execution — real graph execution will be wired later.
    let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let result = ExecutionResult {
        status: ExecutionStatus::Completed,
        state: SharedState::new(),
        trace: vec![TraceEntry {
            node_id: req.entry_node_id.unwrap_or_else(|| "placeholder".to_string()),
            tool_type: "placeholder".to_string(),
            status: "ok".to_string(),
            duration_ms: 0,
            retries: 0,
            error: None,
        }],
        transcript: vec![],
        error: None,
        interrupt_node_id: None,
        interrupt_info: None,
    };

    let body = json!({
        "session_id": session_id,
        "agent_id": id,
        "agent_name": spec.name,
        "status": "completed",
        "trigger_data": req.trigger_data,
        "trace": result.trace,
    });

    state
        .sessions
        .write()
        .await
        .insert(session_id, result);

    Ok(Json(body))
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

    fn test_app() -> Router {
        let state = AppState::new(ToolRegistry::new());
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
        let state = AppState::new(ToolRegistry::new());
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
        let state = AppState::new(ToolRegistry::new());
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
    async fn execute_agent_placeholder() {
        let state = AppState::new(ToolRegistry::new());
        let app = create_router(state.clone());

        // Create graph + agent
        let graph_body = json!({ "name": "exec-graph", "nodes": [], "edges": [] });
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
        let graph_id = body_json(resp.into_body()).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let agent_body = json!({ "name": "exec-agent", "graph_id": graph_id });
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
        let agent_id = body_json(resp.into_body()).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        // Execute
        let exec_body = json!({ "trigger_data": { "msg": "hello" } });
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
        assert_eq!(result["status"], "completed");
        assert_eq!(result["agent_id"], agent_id);
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
