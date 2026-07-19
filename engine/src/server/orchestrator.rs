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
use crate::sessions::{SessionError, SessionStatus, SpawnParams};

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

#[derive(Debug, Deserialize)]
pub(crate) struct RestartRequest {
    /// `"bypass"` → restart with `--dangerously-skip-permissions`; `"default"`
    /// → restart without it (clears bypass). Omitted → keep the current mode.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Optional new model; omitted keeps the current one.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional new effort; omitted keeps the current one.
    #[serde(default)]
    pub effort: Option<String>,
}

/// POST /api/v1/orchestrator/sessions/{id}/restart
///
/// Kill this session's tmux and re-spawn `claude` with new flags
/// (permission_mode / model / effort), KEEPING the same id so the visualizer's
/// node and edges stay attached. This is the "flip a session to bypass-all and
/// restart" action. Only valid for tmux-backed sessions (external nodes → 400).
/// Emits `session_status_changed` (stopped → starting). Returns 200 with the
/// updated record (same shape as `GET /sessions/{id}`, minus `output_tail`).
pub(crate) async fn orchestrator_restart(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RestartRequest>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let record = state
        .orchestrator
        .restart(&id, req.permission_mode, req.model, req.effort)
        .await
        .map_err(session_error_response)?;

    Ok(Json(record.to_api_json()))
}

// ---------------------------------------------------------------------------
// External nodes (bridge) — register nodes that live on ANOTHER substrate
// (e.g. the Claude Code FleetView subagents of a chat session) so they appear
// in the visualizer graph WITHOUT the engine spawning them over tmux.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterExternalRequest {
    pub name: Option<String>,
    pub objective: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Optional caller-provided id (idempotent re-register). Otherwise minted.
    #[serde(default)]
    pub id: Option<String>,
    /// Initial status; defaults to `working`. One of the UI status values.
    #[serde(default)]
    pub status: Option<String>,
}

/// Parse a status string into the enum via the same lowercase serde mapping the
/// UI uses (working|waiting|permission|stopped|error|starting). Rejects unknown
/// values with a clear 400.
fn parse_status(raw: &str) -> Result<SessionStatus, SessionError> {
    serde_json::from_value::<SessionStatus>(Value::String(raw.trim().to_lowercase()))
        .map_err(|_| {
            SessionError::Invalid(format!(
                "invalid status '{raw}' (want working|waiting|permission|stopped|error|starting)"
            ))
        })
}

/// POST /api/v1/orchestrator/sessions/register — register an external node.
/// Returns `{id}` (201). Emits `session_created` with the full node record.
pub(crate) async fn orchestrator_register_external(
    State(state): State<AppState>,
    Json(req): Json<RegisterExternalRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let status = match &req.status {
        Some(s) => Some(parse_status(s).map_err(session_error_response)?),
        None => None,
    };
    let record = state
        .orchestrator
        .register_external(
            req.id,
            req.name,
            req.objective,
            req.parent_id,
            req.model,
            req.effort,
            req.project_dir,
            status,
        )
        .await
        .map_err(session_error_response)?;

    Ok((StatusCode::CREATED, Json(json!({ "id": record.id }))))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExternalStatusRequest {
    pub status: String,
    #[serde(default)]
    pub activity: Option<String>,
}

/// POST /api/v1/orchestrator/sessions/{id}/status — update an external node's
/// status (+ optional activity label). Emits `session_status_changed` (and
/// `session_activity` when `activity` is present). → 200 `{id, status}`.
pub(crate) async fn orchestrator_set_external_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExternalStatusRequest>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let status = parse_status(&req.status).map_err(session_error_response)?;
    let record = state
        .orchestrator
        .set_external_status(&id, status, req.activity.as_deref())
        .await
        .map_err(session_error_response)?;

    Ok(Json(json!({ "id": record.id, "status": record.status })))
}

/// POST /api/v1/orchestrator/sessions/{id}/unregister
/// (also DELETE /api/v1/orchestrator/sessions/{id}) — mark an external node
/// stopped. Emits `session_stopped` → `{id}`. → 200 `{id, status}`.
pub(crate) async fn orchestrator_unregister_external(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let record = state
        .orchestrator
        .unregister_external(&id)
        .await
        .map_err(session_error_response)?;

    Ok(Json(json!({ "id": record.id, "status": record.status })))
}

/// POST /api/v1/orchestrator/sessions/{id}/activity — resource activity
/// reported by the Claude Code PostToolUse hook (M6). → 202.
pub(crate) async fn orchestrator_record_activity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ActivityRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let entry = state
        .orchestrator
        .record_activity(&id, &req.tool, &req.action, req.path.as_deref())
        .await
        .map_err(session_error_response)?;

    Ok((StatusCode::ACCEPTED, Json(entry)))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ActivityRequest {
    pub tool: String,
    pub action: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ActivityQuery {
    pub limit: Option<usize>,
}

/// GET /api/v1/orchestrator/sessions/{id}/activity?limit=50 — recent activity
/// (in-memory ring buffer) as `{events: [...]}` in CHRONOLOGICAL order
/// (canonical envelope fixed by the M6 web UI).
pub(crate) async fn orchestrator_get_activity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let entries = state
        .orchestrator
        .get_activity(&id, limit)
        .await
        .map_err(session_error_response)?;

    Ok(Json(json!({ "events": entries })))
}

/// GET /api/v1/orchestrator/events — SSE stream of orchestrator events.
///
/// Forwards only the PRD-013 event types from the manager's emitter:
/// `session_created`, `session_status_changed`, `session_output`,
/// `session_stopped`, `session_activity`.
///
/// The `data:` payload is the event's data object DIRECTLY (not the internal
/// ExecutionEvent envelope) — exact shapes the web UI parses:
/// - `session_created` → the full session record
/// - `session_status_changed` → `{id, status, last_activity}`
/// - `session_output` → `{id, lines}` (only the NEW lines)
/// - `session_stopped` → `{id}`
/// - `session_activity` → `{id, tool, action, path, ts}` (M6)
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
                            | EventType::SessionActivity
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

/// GET /api/v1/orchestrator/projects — known projects for the create-session
/// picker (M7). Shape: `{projects: [{path, name, source, exists}]}` — union
/// of session project_dirs (source "session") and first-level subdirs of the
/// configured roots (source "scan"); dedup by path, session wins; session
/// entries first, then scan, alphabetical within each group.
pub(crate) async fn orchestrator_list_projects(State(state): State<AppState>) -> Json<Value> {
    let projects = state.orchestrator.list_projects(&state.projects_dirs).await;
    Json(json!({ "projects": projects }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct PickFolderRequest {
    #[serde(default)]
    pub start: Option<String>,
}

/// POST /api/v1/orchestrator/pick-folder — open the HOST's native folder
/// dialog (M9). 200 `{path}` on choose, 200 `{cancelled: true}` on cancel or
/// ~120s timeout, 409 while another dialog is open, 501 without a native
/// picker on this host. Runs async — the server keeps serving while the
/// dialog is open.
pub(crate) async fn orchestrator_pick_folder(
    State(state): State<AppState>,
    Json(req): Json<PickFolderRequest>,
) -> axum::response::Response {
    use crate::sessions::picker::PickOutcome;

    let outcome = state
        .folder_picker
        .pick(std::env::consts::OS, req.start.as_deref())
        .await;

    match outcome {
        PickOutcome::Picked(path) => (StatusCode::OK, Json(json!({ "path": path }))).into_response(),
        PickOutcome::Cancelled => {
            (StatusCode::OK, Json(json!({ "cancelled": true }))).into_response()
        }
        PickOutcome::Busy => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "a folder dialog is already open on the host" })),
        )
            .into_response(),
        PickOutcome::Unsupported(msg) => {
            (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": msg }))).into_response()
        }
        PickOutcome::Failed(msg) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Static web UI under /ui (PRD-013 M6)
// ---------------------------------------------------------------------------

/// GET /ui → index.html of the configured UI directory.
pub(crate) async fn serve_ui_index(State(state): State<AppState>) -> axum::response::Response {
    serve_ui_file(&state, "index.html")
}

/// GET /ui/{*path} → static file from the configured UI directory.
pub(crate) async fn serve_ui(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> axum::response::Response {
    serve_ui_file(&state, &path)
}

fn serve_ui_file(state: &AppState, rel_path: &str) -> axum::response::Response {
    let Some(ui_dir) = &state.ui_dir else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "UI not configured: start the server with --ui-dir <path> (or set MIRAI_UI_DIR) pointing at the web UI directory"
            })),
        )
            .into_response();
    };

    // No traversal: reject `..`, absolute paths, root-relative paths and
    // Windows path prefixes. `Prefix`/`RootDir` matter on Windows, where a
    // drive-relative path like `C:foo` (or a rooted `\foo`) is NOT
    // `is_absolute()` yet makes `ui_dir.join(rel)` REPLACE the base dir,
    // allowing reads outside `ui_dir` on this unauthenticated endpoint.
    let rel = std::path::Path::new(rel_path);
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid path" })),
        )
            .into_response();
    }

    let mut file = ui_dir.join(rel);
    if file.is_dir() {
        file = file.join("index.html");
    }

    match std::fs::read(&file) {
        Ok(bytes) => {
            let content_type = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("json") | Some("map") => "application/json",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                Some("txt") | Some("md") => "text/plain; charset=utf-8",
                _ => "application/octet-stream",
            };
            (StatusCode::OK, [("content-type", content_type)], bytes).into_response()
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("not found: /ui/{rel_path}") })),
        )
            .into_response(),
    }
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
            "project_dir": "/tmp",
            "objective": "build the thing",
            "model": "claude-opus-4-8",
            "effort": "high",
            "ultracode": true,
            // Hook wiring has dedicated tests; keep the command deterministic here.
            "no_hooks": true,
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
            "parent_id",
        ] {
            assert!(rec.get(key).is_some(), "missing contract field {key}");
        }
        assert_eq!(rec["status"], "starting");
        assert_eq!(rec["ultracode"], true);
        assert_eq!(rec["parent_id"], Value::Null);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_accepts_parent_id() {
        let (app, _backend, path) = test_app();
        let parent = spawn_session(&app).await;
        let parent_id = parent["id"].as_str().unwrap();

        let body = json!({
            "project_dir": "/tmp",
            "objective": "child work",
            "parent_id": parent_id,
            "no_hooks": true,
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
        let child = body_json(resp.into_body()).await;
        let child_id = child["id"].as_str().unwrap();

        // Visible in the listing (M6: the canvas draws the parent→child edge).
        let resp = app
            .oneshot(
                Request::get(format!("/api/v1/orchestrator/sessions/{child_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let detail = body_json(resp.into_body()).await;
        assert_eq!(detail["parent_id"], parent_id);
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
    async fn restart_flips_to_bypass_keeps_id_and_respawns_with_flag() {
        let (app, backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap().to_string();

        // Original spawn had no bypass flag.
        assert!(!backend.spawned.lock().unwrap()[0]
            .2
            .contains("--dangerously-skip-permissions"));

        // POST /restart with permission_mode "bypass".
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/restart"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "permission_mode": "bypass" })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        // Same id, back to starting, bypass surfaced in the record.
        assert_eq!(body["id"], id);
        assert_eq!(body["status"], "starting");
        assert_eq!(body["permission_mode"], "bypass");

        // Re-spawned under the SAME tmux name, now carrying the flag.
        let spawned = backend.spawned.lock().unwrap();
        assert_eq!(spawned.len(), 2);
        assert_eq!(spawned[1].0, format!("mirai-{id}"));
        assert!(spawned[1].2.contains("--dangerously-skip-permissions"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn restart_unknown_session_is_404() {
        let (app, _backend, path) = test_app();
        let resp = app
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions/ghost/restart")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json!({})).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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
                project_dir: "/tmp".into(),
                objective: "o".into(),
                model: None,
                effort: None,
                ultracode: false,
                parent_id: None,
                no_hooks: true,
                create_dir: false,
                mcp: false,
                permission_mode: None,
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

    // -- M6: resource activity endpoints -------------------------------------

    #[tokio::test]
    async fn post_activity_returns_202_and_get_lists_it() {
        let (app, _backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/activity"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(
                            &json!({"tool": "Edit", "action": "write", "path": "/tmp/a.rs"}),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let entry = body_json(resp.into_body()).await;
        assert_eq!(entry["action"], "write");
        assert!(entry["ts"].is_string());

        // Second event without path (Bash).
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/activity"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"tool": "Bash", "action": "exec"})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // GET with limit.
        let resp = app
            .oneshot(
                Request::get(format!(
                    "/api/v1/orchestrator/sessions/{id}/activity?limit=1"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        // Canonical envelope {events} in chronological order; limit keeps
        // the most recent.
        let activity = body["events"].as_array().unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0]["tool"], "Bash");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn post_activity_validates_input() {
        let (app, _backend, path) = test_app();
        let created = spawn_session(&app).await;
        let id = created["id"].as_str().unwrap();

        // Bad action → 400.
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/orchestrator/sessions/{id}/activity"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"tool": "Edit", "action": "delete"})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Unknown session → 404.
        let resp = app
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions/ghost/activity")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"tool": "Edit", "action": "write"})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn activity_flows_through_sse() {
        let (state, _backend, path) = test_state();
        let app = create_router(state.clone());

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/orchestrator/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = resp.into_body().into_data_stream();

        use futures_util::StreamExt;
        let first = body.next().await.unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&first), ": connected\n\n");

        // Spawn + report activity through the manager.
        let rec = state
            .orchestrator
            .spawn(crate::sessions::SpawnParams {
                name: None,
                project_dir: "/tmp".into(),
                objective: "o".into(),
                model: None,
                effort: None,
                ultracode: false,
                parent_id: None,
                no_hooks: true,
                create_dir: false,
                mcp: false,
                permission_mode: None,
            })
            .await
            .unwrap();
        state
            .orchestrator
            .record_activity(&rec.id, "Write", "write", Some("/tmp/x.md"))
            .await
            .unwrap();

        // First frame: session_created; second: session_activity.
        let mut saw_activity = false;
        for _ in 0..2 {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
                .await
                .expect("timed out")
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&frame).to_string();
            if text.starts_with("event: session_activity\n") {
                let data_line = text.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
                let payload: Value = serde_json::from_str(data_line).unwrap();
                assert_eq!(payload["id"], rec.id.as_str());
                assert_eq!(payload["tool"], "Write");
                assert_eq!(payload["action"], "write");
                assert_eq!(payload["path"], "/tmp/x.md");
                assert!(payload["ts"].is_string());
                saw_activity = true;
            }
        }
        assert!(saw_activity, "session_activity frame not seen");
        let _ = std::fs::remove_file(path);
    }

    // -- M7: /projects + create_dir --------------------------------------------

    #[tokio::test]
    async fn projects_returns_contract_shape_with_union() {
        let (state, _backend, path) = test_state();
        // Scan root with one project.
        let root = std::env::temp_dir().join(format!("mirai-m7-http-root-{}", short_id()));
        std::fs::create_dir_all(root.join("scanned-app")).unwrap();
        let mut state = state;
        state.projects_dirs = vec![root.clone()];
        let app = create_router(state.clone());

        // One session (source: session).
        spawn_session(&app).await;

        let resp = app
            .oneshot(
                Request::get("/api/v1/orchestrator/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        let projects = body["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);

        // Exact contract shape per entry.
        for p in projects {
            let obj = p.as_object().unwrap();
            assert_eq!(obj.len(), 4, "exactly path/name/source/exists: {obj:?}");
            for key in ["path", "name", "source", "exists"] {
                assert!(obj.contains_key(key), "missing {key}");
            }
        }
        // Session entry first, then scan.
        assert_eq!(projects[0]["source"], "session");
        assert_eq!(projects[0]["path"], "/tmp");
        assert_eq!(projects[1]["source"], "scan");
        assert_eq!(projects[1]["name"], "scanned-app");
        assert_eq!(projects[1]["exists"], true);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_missing_dir_is_400_and_create_dir_fixes_it() {
        let (app, _backend, path) = test_app();
        let missing = std::env::temp_dir().join(format!("mirai-m7-http-dir-{}", short_id()));

        // Without create_dir → 400 with a clear message.
        let body = json!({
            "project_dir": missing.to_str().unwrap(),
            "objective": "x",
            "no_hooks": true,
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
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let err = body_json(resp.into_body()).await;
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("does not exist"), "clear: {msg}");
        assert!(msg.contains("create_dir"), "hints the fix: {msg}");

        // With create_dir → 201 and the dir exists.
        let body = json!({
            "project_dir": missing.to_str().unwrap(),
            "objective": "x",
            "no_hooks": true,
            "create_dir": true,
        });
        let resp = app
            .oneshot(
                Request::post("/api/v1/orchestrator/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert!(missing.is_dir());

        let _ = std::fs::remove_dir_all(missing);
        let _ = std::fs::remove_file(path);
    }

    // -- M9: native folder picker ------------------------------------------------

    /// Minimal scripted runner for HTTP-level tests.
    struct HttpFakeRunner {
        outcome: crate::sessions::picker::RunOutcome,
        hold_ms: u64,
    }

    #[async_trait::async_trait]
    impl crate::sessions::picker::DialogRunner for HttpFakeRunner {
        async fn run(
            &self,
            _program: &str,
            _args: &[String],
            _timeout: std::time::Duration,
        ) -> crate::sessions::picker::RunOutcome {
            if self.hold_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.hold_ms)).await;
            }
            self.outcome.clone()
        }
    }

    fn app_with_picker(outcome: crate::sessions::picker::RunOutcome, hold_ms: u64) -> (Router, std::path::PathBuf) {
        let (mut state, _backend, path) = test_state();
        state.folder_picker = Arc::new(crate::sessions::picker::FolderPicker::with_runner(
            Box::new(HttpFakeRunner { outcome, hold_ms }),
            std::time::Duration::from_secs(2),
        ));
        (create_router(state), path)
    }

    async fn post_pick(app: &Router, body: Value) -> (StatusCode, Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/orchestrator/pick-folder")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        (status, body_json(resp.into_body()).await)
    }

    #[tokio::test]
    async fn pick_folder_returns_chosen_path() {
        use crate::sessions::picker::RunOutcome;
        let (app, path) = app_with_picker(
            RunOutcome::Completed {
                code: Some(0),
                stdout: "/Users/gabo/proyecto/
".into(),
                stderr: String::new(),
            },
            0,
        );
        let (status, body) = post_pick(&app, json!({"start": "/tmp"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"path": "/Users/gabo/proyecto"}));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn pick_folder_cancel_and_timeout_are_cancelled_200() {
        use crate::sessions::picker::RunOutcome;
        // User cancel (osascript -128).
        let (app, path) = app_with_picker(
            RunOutcome::Completed {
                code: Some(1),
                stdout: String::new(),
                stderr: "execution error: User canceled. (-128)".into(),
            },
            0,
        );
        let (status, body) = post_pick(&app, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"cancelled": true}));
        let _ = std::fs::remove_file(path);

        // Timeout → also cancelled.
        let (app, path) = app_with_picker(RunOutcome::TimedOut, 0);
        let (status, body) = post_pick(&app, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"cancelled": true}));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn pick_folder_unsupported_is_501() {
        use crate::sessions::picker::RunOutcome;
        let (app, path) = app_with_picker(RunOutcome::SpawnNotFound, 0);
        let (status, body) = post_pick(&app, json!({})).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body["error"].as_str().unwrap().len() > 5);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn pick_folder_second_concurrent_request_is_409() {
        use crate::sessions::picker::RunOutcome;
        let (app, path) = app_with_picker(
            RunOutcome::Completed {
                code: Some(0),
                stdout: "/tmp/a
".into(),
                stderr: String::new(),
            },
            300,
        );

        let app1 = app.clone();
        let first =
            tokio::spawn(async move { post_pick(&app1, json!({})).await });
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;

        let (status, body) = post_pick(&app, json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().unwrap().contains("already open"));

        let (status, body) = first.await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["path"], "/tmp/a");
        let _ = std::fs::remove_file(path);
    }

    // -- M6: /ui static serving -----------------------------------------------

    #[tokio::test]
    async fn ui_unconfigured_returns_clear_404() {
        let (app, _backend, path) = test_app();
        let resp = app
            .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_json(resp.into_body()).await;
        let msg = body["error"].as_str().unwrap();
        assert!(msg.contains("--ui-dir"), "clear message: {msg}");
        assert!(msg.contains("MIRAI_UI_DIR"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn ui_serves_index_and_assets_without_auth() {
        // UI dir with an index + an asset.
        let ui_dir = std::env::temp_dir().join(format!("mirai-ui-test-{}", short_id()));
        std::fs::create_dir_all(&ui_dir).unwrap();
        std::fs::write(ui_dir.join("index.html"), "<!doctype html><title>UI</title>").unwrap();
        std::fs::write(ui_dir.join("app.js"), "console.log(1)").unwrap();

        // Server WITH api key: /ui must still be public.
        let backend = Arc::new(FakeBackend::new());
        let registry_path =
            std::env::temp_dir().join(format!("mirai-ui-reg-{}.json", short_id()));
        let mut state = AppState::new(
            ToolRegistry::new(),
            test_llm_factory(),
            Some("secret".into()),
        );
        state.orchestrator = Arc::new(SessionManager::new(backend, registry_path.clone()));
        state.ui_dir = Some(ui_dir.clone());
        let app = create_router(state);

        let resp = app
            .clone()
            .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("<title>UI</title>"));

        let resp = app
            .clone()
            .oneshot(Request::get("/ui/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/javascript; charset=utf-8"
        );

        // Missing file → 404; traversal → 400.
        let resp = app
            .clone()
            .oneshot(Request::get("/ui/nope.css").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let resp = app
            .clone()
            .oneshot(
                Request::get("/ui/..%2F..%2Fetc%2Fpasswd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Root-relative path (leading `/`) → 400. On Windows `\foo` is not
        // `is_absolute()` but would still replace the base in `join`.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/ui/%2Fetc%2Fpasswd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Windows drive-relative path (`Component::Prefix`) → 400. `C:foo`
        // makes `join` replace the base dir entirely. Only parses as a
        // prefix on Windows, so assert there only.
        #[cfg(windows)]
        {
            let resp = app
                .clone()
                .oneshot(Request::get("/ui/C:foo").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }

        let _ = std::fs::remove_dir_all(ui_dir);
        let _ = std::fs::remove_file(registry_path);
    }
}
