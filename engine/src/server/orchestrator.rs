//! PRD-013 — HTTP API for orchestrated Claude sessions (`/api/v1/orchestrator/*`).
//!
//! Implements the pinned contract:
//!
//! | Endpoint | Method | Body / Response |
//! |---|---|---|
//! | `/sessions` | POST | `{name?, project_dir, objective, model?, effort?, ultracode?}` → `{id, tmux_session}` |
//! | `/sessions` | GET | array of session records |
//! | `/sessions/{id}` | GET | record + `{output_tail: [lines]}` |
//! | `/sessions/{id}/send` | POST | `{text}` → 202 |
//! | `/sessions/{id}/output?lines=N` | GET | `{lines: […]}` |
//! | `/sessions/{id}/stop` | POST | 200 |
//! | `/events` | GET (SSE) | `session_created`, `session_status_changed`, `session_output`, `session_stopped` |
//!
//! Auth is the same optional X-API-Key middleware as the rest of the API;
//! CORS is the router-wide permissive layer (the web UI opens via `file://`).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::events::EventType;
use crate::sessions::{SessionError, SpawnParams};

use super::state::{AppState, ErrorResponse};

/// Default lines for `/output` and the detail `output_tail`.
const DEFAULT_OUTPUT_LINES: usize = 100;
const DETAIL_TAIL_LINES: usize = 40;

fn session_error_response(err: SessionError) -> (StatusCode, Json<ErrorResponse>) {
    let status = match &err {
        SessionError::NotFound(_) => StatusCode::NOT_FOUND,
        SessionError::Invalid(_) => StatusCode::BAD_REQUEST,
        SessionError::Stopped(_) => StatusCode::CONFLICT,
        SessionError::Backend(_) | SessionError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/orchestrator/sessions
pub(crate) async fn orchestrator_create_session(
    State(state): State<AppState>,
    Json(params): Json<SpawnParams>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let record = state
        .orchestrator
        .spawn(params)
        .await
        .map_err(session_error_response)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": record.id,
            "tmux_session": record.tmux_session,
        })),
    ))
}

/// GET /api/v1/orchestrator/sessions
pub(crate) async fn orchestrator_list_sessions(State(state): State<AppState>) -> Json<Value> {
    let list: Vec<Value> = state
        .orchestrator
        .list()
        .await
        .iter()
        .map(|r| r.to_api_json())
        .collect();
    Json(json!(list))
}

/// GET /api/v1/orchestrator/sessions/{id}
pub(crate) async fn orchestrator_get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let record = state
        .orchestrator
        .get(&id)
        .await
        .ok_or_else(|| session_error_response(SessionError::NotFound(id.clone())))?;

    let output_tail = state
        .orchestrator
        .output(&id, DETAIL_TAIL_LINES)
        .await
        .unwrap_or_default();

    let mut body = record.to_api_json();
    body["output_tail"] = json!(output_tail);
    Ok(Json(body))
}

#[derive(Debug, Deserialize)]
pub(crate) struct SendRequest {
    pub text: String,
}

/// POST /api/v1/orchestrator/sessions/{id}/send
pub(crate) async fn orchestrator_send(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    if req.text.trim().is_empty() {
        return Err(session_error_response(SessionError::Invalid(
            "text is required".into(),
        )));
    }
    state
        .orchestrator
        .send(&id, &req.text)
        .await
        .map_err(session_error_response)?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id, "sent": true }))))
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputQuery {
    pub lines: Option<usize>,
}

/// GET /api/v1/orchestrator/sessions/{id}/output?lines=N
pub(crate) async fn orchestrator_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<OutputQuery>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let lines = query.lines.unwrap_or(DEFAULT_OUTPUT_LINES).clamp(1, 5000);
    let output = state
        .orchestrator
        .output(&id, lines)
        .await
        .map_err(session_error_response)?;

    Ok(Json(json!({ "lines": output })))
}

/// POST /api/v1/orchestrator/sessions/{id}/stop
pub(crate) async fn orchestrator_stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let record = state
        .orchestrator
        .stop(&id)
        .await
        .map_err(session_error_response)?;

    Ok(Json(json!({ "id": record.id, "status": record.status })))
}

/// GET /api/v1/orchestrator/events — SSE stream of orchestrator events.
///
/// Forwards only the four PRD-013 event types from the manager's emitter:
/// `session_created`, `session_status_changed`, `session_output`,
/// `session_stopped`.
///
/// The `data:` payload is the event's data object DIRECTLY (not the internal
/// ExecutionEvent envelope) — exact shapes the web UI parses:
/// - `session_created` → the full session record
/// - `session_status_changed` → `{id, status, last_activity}`
/// - `session_output` → `{id, lines}` (only the NEW lines)
/// - `session_stopped` → `{id}`
///
/// Auth: besides the X-API-Key header, this endpoint accepts the key as a
/// `?api_key=` query param (EventSource cannot set headers).
pub(crate) async fn orchestrator_events(State(state): State<AppState>) -> impl IntoResponse {
    use axum::body::Body;
    use tokio_stream::wrappers::ReceiverStream;

    let mut events = state.orchestrator.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256);

    tokio::spawn(async move {
        // Open the stream immediately so clients get headers + a first byte.
        if tx.send(Ok(": connected\n\n".to_string())).await.is_err() {
            return;
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    let relevant = matches!(
                        event.event_type,
                        EventType::SessionCreated
                            | EventType::SessionStatusChanged
                            | EventType::SessionOutput
                            | EventType::SessionStopped
                    );
                    if !relevant {
                        continue;
                    }
                    let payload =
                        serde_json::to_string(&event.data).unwrap_or_else(|_| "{}".into());
                    let frame = format!("event: {}\ndata: {}\n\n", event.event_type, payload);
                    if tx.send(Ok(frame)).await.is_err() {
                        break; // client disconnected
                    }
                }
                // Slow consumer: skip the lagged messages, keep streaming.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("connection", "keep-alive"),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
}

// ---------------------------------------------------------------------------
// Tests — handlers over a fake backend (same pattern as server/tests.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::server::create_router;
    use crate::server::state::{AppState, LLMFactory};
    use crate::sessions::{FakeBackend, SessionBackend, SessionManager};
    use crate::tools::registry::ToolRegistry;
    use crate::utils::short_id;

    fn test_llm_factory() -> LLMFactory {
        use crate::adapters::MockLLMResource;
        Arc::new(|| Box::new(MockLLMResource::new()))
    }

    /// AppState whose orchestrator runs on a FakeBackend + temp registry.
    fn test_state() -> (AppState, Arc<FakeBackend>, std::path::PathBuf) {
        let backend = Arc::new(FakeBackend::new());
        let registry_path =
            std::env::temp_dir().join(format!("mirai-orch-http-test-{}.json", short_id()));
        let mut state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
        state.orchestrator = Arc::new(SessionManager::new(
            backend.clone(),
            registry_path.clone(),
        ));
        (state, backend, registry_path)
    }

    fn test_app() -> (Router, Arc<FakeBackend>, std::path::PathBuf) {
        let (state, backend, path) = test_state();
        (create_router(state), backend, path)
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn spawn_session(app: &Router) -> Value {
        let body = json!({
            "name": "worker",
            "project_dir": "/tmp/proj",
            "objective": "build the thing",
            "model": "claude-opus-4-8",
            "effort": "high",
            "ultracode": true,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        body_json(resp.into_body()).await
    }

    #[tokio::test]
    async fn create_session_returns_id_and_tmux_session() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;

        let id = created["id"].as_str().unwrap();
        assert_eq!(
            created["tmux_session"].as_str().unwrap(),
            format!("mirai-{id}")
        );
        // The backend actually spawned claude with model + effort.
        let spawned = backend.spawned.lock().unwrap();
        assert_eq!(spawned[0].2, "claude --model claude-opus-4-8 --effort high");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn create_session_without_project_dir_is_400() {
        let (app, _backend, path) = test_app();
        let body = json!({ "project_dir": "", "objective": "x" });
        let resp = app
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_sessions_returns_contract_shape() {
        let (app, _backend, path) = test_app();
        spawn_session(&app).await;

        let resp = app
            .oneshot(
                Request::get("/api/v1/orchestrator/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp.into_body()).await;
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let rec = &arr[0];
        for key in [
            "id",
            "name",
            "project_dir",
            "objective",
            "status",
            "model",
            "effort",
            "ultracode",
            "tmux_session",
            "created_at",
            "last_activity",
        ] {
            assert!(rec.get(key).is_some(), "missing contract field {key}");
        }
        assert_eq!(rec["status"], "starting");
        assert_eq!(rec["ultracode"], true);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn get_session_includes_output_tail() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();
        backend.set_pane(&format!("mirai-{id}"), &["hola", "│ > "]);

        let resp = app
            .oneshot(
                Request::get(format!("/api/v1/orchestrator/sessions/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let detail = body_json(resp.into_body()).await;
        assert_eq!(detail["id"], id);
        let tail: Vec<String> = detail["output_tail"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(tail.contains(&"hola".to_string()));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn get_unknown_session_is_404() {
        let (app, _backend, path) = test_app();
        let resp = app
            .oneshot(
                Request::get("/api/v1/orchestrator/sessions/ghost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn send_returns_202_and_applies_ultracode_on_first_prompt() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/send"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "text": "arranca" })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent[0].1, "ultracode arranca");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn send_to_unknown_session_is_404() {
        let (app, _backend, path) = test_app();
        let resp = app
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions/ghost/send")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json!({"text": "x"})).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn output_respects_lines_param() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();
        backend.set_pane(&format!("mirai-{id}"), &["a", "b", "c", "d"]);

        let resp = app
            .oneshot(
                Request::get(format!("/api/v1/orchestrator/sessions/{id}/output?lines=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let out = body_json(resp.into_body()).await;
        assert_eq!(out["lines"], json!(["c", "d"]));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn stop_kills_session_and_marks_stopped() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/stop"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["status"], "stopped");
        assert!(!backend.session_exists(&format!("mirai-{id}")));

        // Sending after stop → 409.
        let resp = app
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/send"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json!({"text": "x"})).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn events_endpoint_streams_session_created() {
        let (state, _backend, path) = test_state();
        let app = create_router(state.clone());

        // Open the SSE stream, then spawn a session and read the event.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/orchestrator/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );

        let mut body = resp.into_body().into_data_stream();

        // First frame: the connected comment.
        use futures_util::StreamExt;
        let first = body.next().await.unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&first), ": connected\n\n");

        // Trigger an event through the manager.
        state
            .orchestrator
            .spawn(crate::sessions::SpawnParams {
                name: None,
                project_dir: "/tmp/p".into(),
                objective: "o".into(),
                model: None,
                effort: None,
                ultracode: false,
            })
            .await
            .unwrap();

        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("timed out waiting for SSE event")
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(text.starts_with("event: session_created\n"), "got: {text}");

        // The data payload is the session record DIRECTLY (contract shape),
        // not the internal ExecutionEvent envelope.
        let data_line = text
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .expect("data line");
        let payload: Value = serde_json::from_str(data_line).unwrap();
        assert!(payload["id"].is_string());
        assert_eq!(payload["status"], "starting");
        assert!(payload["tmux_session"]
            .as_str()
            .unwrap()
            .starts_with("mirai-"));
        assert!(payload.get("event_type").is_none(), "envelope leaked");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn events_accepts_api_key_via_query_param() {
        let backend = Arc::new(FakeBackend::new());
        let registry_path =
            std::env::temp_dir().join(format!("mirai-orch-sse-auth-{}.json", short_id()));
        let mut state = AppState::new(
            ToolRegistry::new(),
            test_llm_factory(),
            Some("secret".into()),
        );
        state.orchestrator = Arc::new(SessionManager::new(backend, registry_path.clone()));
        let app = create_router(state);

        // No key at all → 401.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/orchestrator/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Wrong query key → 401.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/orchestrator/events?api_key=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Correct key via query param (EventSource cannot set headers) → 200.
        let resp = app
            .oneshot(
                Request::get("/api/v1/orchestrator/events?api_key=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = std::fs::remove_file(registry_path);
    }

    #[tokio::test]
    async fn orchestrator_routes_respect_api_key_auth() {
        let backend = Arc::new(FakeBackend::new());
        let registry_path =
            std::env::temp_dir().join(format!("mirai-orch-auth-test-{}.json", short_id()));
        let mut state = AppState::new(
            ToolRegistry::new(),
            test_llm_factory(),
            Some("secret".into()),
        );
        state.orchestrator = Arc::new(SessionManager::new(backend, registry_path.clone()));
        let app = create_router(state);

        // Without key → 401.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/orchestrator/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // With key → 200.
        let resp = app
            .oneshot(
                Request::get("/api/v1/orchestrator/sessions")
                    .header("X-API-Key", "secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = std::fs::remove_file(registry_path);
    }
}
