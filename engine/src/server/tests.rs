use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use super::create_router;
use super::state::{AppState, LLMFactory};

/// Mock LLM factory for unit tests ONLY (#[cfg(test)]).
fn test_llm_factory() -> LLMFactory {
    use crate::adapters::MockLLMResource;
    Arc::new(|| Box::new(MockLLMResource::new()))
}

fn test_app() -> Router {
    let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
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
    // El handler emite MIRAI_VERSION (archivo VERSION), no CARGO_PKG_VERSION:
    // asertar la misma fuente evita falsos rojos si divergen.
    assert_eq!(json["version"], env!("MIRAI_VERSION"));
    assert_eq!(json["engine"], "openmirai-engine-rs");
}

#[tokio::test]
async fn graph_crud_lifecycle() {
    let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
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
            Request::post("/api/v1/graphs")
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
        .oneshot(Request::get("/api/v1/graphs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp.into_body()).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Get
    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/graphs/{graph_id}"))
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
            Request::delete(format!("/api/v1/graphs/{graph_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Get after delete -> 404
    let resp = app
        .oneshot(
            Request::get(format!("/api/v1/graphs/{graph_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_crud_lifecycle() {
    let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
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
            Request::post("/api/v1/graphs")
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
            Request::post("/api/v1/agents")
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
        .oneshot(Request::get("/api/v1/agents").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp.into_body()).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Get agent
    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/agents/{agent_id}"))
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
    let state = AppState::new(registry, test_llm_factory(), None);
    let app = create_router(state.clone());

    // Create agent via from-spec with a simple trigger→response graph
    let spec = json!({
        "name": "test-exec",
        "description": "test",
        "version": "v1",
        "graph": {
            "nodes": [
                {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hello"}}},
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
            Request::post("/api/v1/agents/from-spec")
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
            Request::post(format!("/api/v1/agents/{agent_id}/execute"))
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

/// Helper 0.7.0: crea un agente trigger→out por from-spec y lo ejecuta.
/// Devuelve (agent_id, session_id).
async fn create_and_execute(app: &Router, name: &str) -> (String, String) {
    let spec = json!({
        "name": name,
        "description": "test runs persistence",
        "version": "v1",
        "graph": {
            "nodes": [
                {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hola"}}},
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
            Request::post("/api/v1/agents/from-spec")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let agent_id = body_json(resp.into_body()).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/agents/{agent_id}/execute"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "trigger_data": {} })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session_id = body_json(resp.into_body()).await["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    (agent_id, session_id)
}

#[tokio::test]
async fn executed_run_persists_and_survives_memory_wipe() {
    // 0.7.0: los runs sobreviven reinicios — ejecutar, vaciar la cache de
    // memoria (símil de reinicio) y leerlo TODO desde SQLite.
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let mut state = AppState::new(registry, test_llm_factory(), None);
    state.session_repo = Some(std::sync::Arc::new(
        crate::db::SqliteSessionRepo::open_in_memory().unwrap(),
    ));
    let app = create_router(state.clone());

    let (agent_id, session_id) = create_and_execute(&app, "test-exec-persist").await;

    // Símil de reinicio: la memoria se pierde, la DB no.
    state.sessions.write().await.clear();

    // GET /sessions/{id} responde desde SQLite con el run completo.
    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "el run debe sobrevivir el wipe"
    );
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["status"], "Completed");
    assert_eq!(json["agent_id"], agent_id);
    assert_eq!(json["agent_name"], "test-exec-persist");
    assert!(json["started_at"].as_f64().unwrap() > 0.0);
    assert!(json["duration_ms"].as_f64().is_some());
    // Timeline por nodo con timestamps reales (base de la trazabilidad).
    let trace = json["trace"].as_array().unwrap();
    assert!(!trace.is_empty());
    assert!(trace[0]["started_at"].as_f64().unwrap() > 0.0);
    assert!(trace[0]["finished_at"].as_f64().unwrap() >= trace[0]["started_at"].as_f64().unwrap());

    // La traza OTel también sobrevive el reinicio.
    let resp = app
        .oneshot(
            Request::get(format!("/api/v1/sessions/{session_id}/otel-trace"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let otel = body_json(resp.into_body()).await;
    let spans = otel["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert!(spans.len() >= 2, "root + nodos");
    // Los spans llevan tiempos epoch reales (no fabricados desde t=0).
    let node_span = &spans[1];
    assert!(
        node_span["startTimeUnixNano"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 1_000_000_000_000_000_000, // > ~2001 en nanos: es epoch real
        "span debe usar timestamps reales"
    );
}

#[tokio::test]
async fn list_sessions_reads_from_db_with_agent_filter() {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let mut state = AppState::new(registry, test_llm_factory(), None);
    state.session_repo = Some(std::sync::Arc::new(
        crate::db::SqliteSessionRepo::open_in_memory().unwrap(),
    ));
    let app = create_router(state.clone());

    let (agent_a, _) = create_and_execute(&app, "agente-a").await;
    let (_, _) = create_and_execute(&app, "agente-b").await;

    // Sin filtro: los 2 runs, con metadata del registro persistido.
    let resp = app
        .clone()
        .oneshot(
            Request::get("/api/v1/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let all = body_json(resp.into_body()).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert!(all[0]["agent_name"].is_string());
    assert!(all[0]["started_at"].as_f64().unwrap() > 0.0);

    // Filtro ?agent_id= (antes se ignoraba; ahora es real vía SQL).
    let resp = app
        .oneshot(
            Request::get(format!("/api/v1/sessions?agent_id={agent_a}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let filtered = body_json(resp.into_body()).await;
    let arr = filtered.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["agent_id"], agent_a);
    assert_eq!(arr[0]["agent_name"], "agente-a");
}

#[tokio::test]
async fn list_tools_returns_all_builtins() {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let state = AppState::new(registry, test_llm_factory(), None);
    let app = create_router(state);

    let resp = app
        .oneshot(Request::get("/api/v1/tools").body(Body::empty()).unwrap())
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
            Request::post("/api/v1/agents")
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
            Request::get("/api/v1/sessions/ghost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_session_otel_trace_returns_otel_format() {
    use crate::core::runner::{ExecutionResult, ExecutionStatus, TraceEntry, TraceStatus};
    use crate::core::state::ExecutionState;

    let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
    let app = create_router(state.clone());

    let trace_entry = TraceEntry {
        node_id: "node_1".to_string(),
        tool_type: "logic/condition".to_string(),
        status: TraceStatus::Ok,
        duration_ms: 150,
        retries: 0,
        started_at: 0.0,
        finished_at: 0.0,
        error: None,
    };

    let exec_result = ExecutionResult {
        status: ExecutionStatus::Completed,
        state: ExecutionState::new(),
        trace: vec![trace_entry],
        transcript: vec![],
        error: None,
        interrupt_node_id: None,
        interrupt_info: None,
    };

    state
        .sessions
        .write()
        .await
        .insert("test-session-123".to_string(), exec_result);

    let resp = app
        .oneshot(
            Request::get("/api/v1/sessions/test-session-123/otel-trace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;

    // Check OpenTelemetry formatting attributes
    assert!(json.get("resourceSpans").is_some());
    let resource_spans = json["resourceSpans"].as_array().unwrap();
    assert_eq!(resource_spans.len(), 1);

    let scope_spans = resource_spans[0]["scopeSpans"].as_array().unwrap();
    assert_eq!(scope_spans.len(), 1);

    let spans = scope_spans[0]["spans"].as_array().unwrap();
    // 1 root span + 1 node span = 2 spans
    assert_eq!(spans.len(), 2);

    let root_span = &spans[0];
    assert_eq!(root_span["name"], "graph:test-session-123");

    let node_span = &spans[1];
    assert_eq!(node_span["name"], "node:node_1");
    assert_eq!(node_span["parentSpanId"], root_span["spanId"]);
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

// ---------------------------------------------------------------------------
// Security gate 0.7.0 — CORS allowlist + cross-site guard (drive-by RCE fix)
// ---------------------------------------------------------------------------

/// Cross-origin preflight from a hostile web origin must NOT be approved:
/// no `access-control-allow-origin` → the browser blocks the real request.
#[tokio::test]
async fn cors_preflight_rejects_external_origin() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/agents/x/execute")
                .header("origin", "https://evil.com")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "evil.com must not be CORS-approved"
    );
}

/// Loopback origins (the local UIs, any port) keep working cross-origin.
#[tokio::test]
async fn cors_preflight_allows_loopback_origin() {
    for origin in ["http://localhost:5173", "http://127.0.0.1:8080"] {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/orchestrator/sessions")
                    .header("origin", origin)
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "content-type")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(origin),
            "loopback origin {origin} must stay CORS-approved"
        );
    }
}

/// `Origin: null` (file:// pages, e.g. the Claude-Orchestrator web UI) is
/// trusted ONLY on the orchestrator API — not on agent execute.
#[tokio::test]
async fn cors_null_origin_scoped_to_orchestrator() {
    let preflight = |uri: &str| {
        Request::builder()
            .method("OPTIONS")
            .uri(uri)
            .header("origin", "null")
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap()
    };

    let resp = test_app()
        .oneshot(preflight("/api/v1/orchestrator/sessions"))
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_some(),
        "file:// orchestrator UI must keep working"
    );

    let resp = test_app()
        .oneshot(preflight("/api/v1/agents/x/execute"))
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "null origin must NOT reach agent execute"
    );
}

/// Defense in depth: a "simple" cross-origin POST (no preflight, e.g.
/// body-less stop) from a hostile origin is rejected server-side with 403.
#[tokio::test]
async fn cross_origin_guard_blocks_untrusted_mutations() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::post("/api/v1/orchestrator/sessions/some-id/stop")
                .header("origin", "https://evil.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// The guard lets through: no-Origin clients (curl/SDKs), loopback origins,
/// and same-host origins (UI served over LAN on the same port).
#[tokio::test]
async fn cross_origin_guard_allows_legit_clients() {
    // curl / SDK: no Origin header → untouched (404 = router miss, not 403).
    let resp = test_app()
        .oneshot(
            Request::post("/api/v1/orchestrator/sessions/some-id/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::FORBIDDEN);

    // Loopback origin → allowed.
    let resp = test_app()
        .oneshot(
            Request::post("/api/v1/orchestrator/sessions/some-id/stop")
                .header("origin", "http://localhost:9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::FORBIDDEN);

    // Same-host origin (LAN) → allowed.
    let resp = test_app()
        .oneshot(
            Request::post("/api/v1/orchestrator/sessions/some-id/stop")
                .header("origin", "http://192.168.1.50:3000")
                .header("host", "192.168.1.50:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::FORBIDDEN);
}

#[test]
fn api_key_matches_is_exact() {
    assert!(super::api_key_matches("secret-123", "secret-123"));
    assert!(!super::api_key_matches("secret-124", "secret-123"));
    assert!(!super::api_key_matches("secret-12", "secret-123"));
    assert!(!super::api_key_matches("", "secret-123"));
}

#[test]
fn host_is_loopback_classification() {
    assert!(super::host_is_loopback("127.0.0.1"));
    assert!(super::host_is_loopback("localhost"));
    assert!(super::host_is_loopback("::1"));
    assert!(!super::host_is_loopback("0.0.0.0"));
    assert!(!super::host_is_loopback("192.168.1.10"));
}

/// El `session_id` que viaja en los eventos SSE del stream DEBE ser el mismo
/// con el que el run queda persistido: un cliente que ve un evento y luego
/// pide ese run tiene que encontrarlo. Antes nacían dos ids distintos (uno en
/// el contexto de ejecución, otro en el handler al registrar), así que
/// correlacionar un run con sus eventos era imposible.
#[tokio::test]
async fn stream_event_session_id_matches_persisted_run() {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let state = AppState::new(registry, test_llm_factory(), None);
    let app = create_router(state);

    let spec = json!({
        "name": "stream-corr",
        "description": "correlación de session_id",
        "version": "v1",
        "graph": {
            "nodes": [
                {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hola"}}},
                {"id": "out", "tool_type": "output/response", "config": {"message": "done"}}
            ],
            "edges": [{"source": "trigger", "target": "out"}]
        }
    });
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/v1/agents/from-spec")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let agent_id = body_json(resp.into_body()).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/agents/{agent_id}/stream"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "trigger_data": {} })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Drenar el SSE y sacar el session_id que vio el cliente.
    let sse = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    let event_session_id = sse
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        // `to_sse` serializa el enum completo: {"event": "...", "data": {...}}
        .find_map(|v| {
            v.get("data")
                .and_then(|d| d.get("session_id"))
                .and_then(|s| s.as_str())
                .map(str::to_string)
        })
        .expect("el stream debe anunciar el run al que pertenecen sus eventos");

    // El run se persiste al terminar la ejecución, en otra tarea.
    let mut found = None;
    for _ in 0..40 {
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/sessions/{event_session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if resp.status() == StatusCode::OK {
            found = Some(body_json(resp.into_body()).await);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let run = found.unwrap_or_else(|| {
        panic!("el run con session_id={event_session_id} (el que vio el cliente) no existe")
    });
    assert_eq!(run["id"], event_session_id);
}

// ---------------------------------------------------------------------------
// DELETE /api/v1/agents/{id}
//
// Sin esta ruta un agente registrado no se podía retirar: quedaba en el
// registro para siempre y, si era live, se relanzaba en cada arranque.
// ---------------------------------------------------------------------------

/// Registra un agente mínimo y devuelve su id.
async fn registrar_agente(app: &Router, nombre: &str) -> String {
    let spec = json!({
        "name": nombre,
        "version": "v1",
        "graph": {
            "nodes": [
                {"id": "trigger", "tool_type": "trigger/manual"},
                {"id": "out", "tool_type": "output/response"}
            ],
            "edges": [{"source": "trigger", "target": "out"}]
        }
    });

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/v1/agents/from-spec")
                .header("content-type", "application/json")
                .body(Body::from(spec.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    body_json(resp.into_body()).await["id"]
        .as_str()
        .expect("id del agente")
        .to_string()
}

#[tokio::test]
async fn delete_agent_lo_saca_del_registro() {
    let app = test_app();
    let id = registrar_agente(&app, "descartable").await;

    let resp = app
        .clone()
        .oneshot(
            Request::delete(format!("/api/v1/agents/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["status"], "deleted");
    assert_eq!(json["was_playing"], false);

    // Ya no se puede consultar ni ejecutar.
    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/agents/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_agent_inexistente_da_404() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::delete("/api/v1/agents/no-existe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
