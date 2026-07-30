//! HTTP API to RUN workflows by name (`/api/v1/workflows/{name}/run` + `/runs`).
//!
//! Gabriel's principle: reading/writing the fleet SoT is done as **workflow
//! executions** (traceable SOPs), not inline glue code. This endpoint loads a
//! workflow YAML by name, compiles it to a graph and runs it on the existing
//! [`GraphRunner`] — ASYNCHRONOUSLY: the POST returns a `run_id` immediately and
//! the caller polls `GET /api/v1/runs/{id}` for status + the final state.
//!
//! | Endpoint | Method | Body / Response |
//! |---|---|---|
//! | `/workflows/{name}/run` | POST | `{trigger_data?, base_url?}` → 202 `{run_id, status:"running", workflow}` |
//! | `/runs/{id}` | GET | `{id, workflow, status, started_at, finished_at?, error?, state?}` |
//!
//! `base_url` is injected into the workflow's trigger payload so `net/http_request`
//! nodes can call back into THIS engine (e.g. spawn a session, then write the
//! fleet SoT). It defaults to `http://127.0.0.1:{server_port}`.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::agent_spec::AgentSpec;
use crate::core::runner::ExecutionStatus;
use crate::utils::{now_epoch, short_id};

use super::helpers::run_agent_spec;
use super::state::{AppState, ErrorResponse};

/// Lifecycle of a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

/// A single async workflow run, stored in `AppState.runs`.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub workflow: String,
    pub status: RunStatus,
    pub started_at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Final graph state snapshot (node_id → {field → value}) on completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RunRequest {
    #[serde(default)]
    pub trigger_data: HashMap<String, Value>,
    /// Base URL the workflow's HTTP nodes call back on. Defaults to this
    /// server (`http://127.0.0.1:{server_port}`).
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Reject names that could escape the workflows dirs (path traversal).
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// Resolve `{name}` to a workflow file across the configured dirs.
fn resolve_workflow(state: &AppState, name: &str) -> Option<std::path::PathBuf> {
    let stem = name.strip_suffix(".yaml").unwrap_or(name);
    let stem = stem.strip_suffix(".yml").unwrap_or(stem);
    for dir in &state.workflows_dirs {
        for ext in ["yaml", "yml"] {
            let candidate = dir.join(format!("{stem}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// POST /api/v1/workflows/{name}/run
pub(crate) async fn workflow_run(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<RunRequest>>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    let err = |code: StatusCode, m: String| (code, Json(ErrorResponse { error: m }));

    if !is_safe_name(&name) {
        return Err(err(StatusCode::BAD_REQUEST, format!("invalid workflow name '{name}'")));
    }
    let path = resolve_workflow(&state, &name).ok_or_else(|| {
        err(
            StatusCode::NOT_FOUND,
            format!(
                "workflow '{name}' not found in {:?}",
                state.workflows_dirs
            ),
        )
    })?;
    let spec = AgentSpec::from_file(&path.to_string_lossy())
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("failed to load workflow '{name}': {e}")))?;

    let req = body.map(|Json(b)| b).unwrap_or_default();
    let mut trigger_data = req.trigger_data;
    // Inject base_url so net/http_request nodes can reach this engine.
    trigger_data.entry("base_url".to_string()).or_insert_with(|| {
        let base = req
            .base_url
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", state.server_port));
        Value::String(base)
    });

    let run_id = short_id();
    let record = RunRecord {
        id: run_id.clone(),
        workflow: name.clone(),
        status: RunStatus::Running,
        started_at: now_epoch(),
        finished_at: None,
        error: None,
        state: None,
    };
    state.runs.write().await.insert(run_id.clone(), record);

    // Run asynchronously; the POST returns immediately with the run_id.
    let task_state = state.clone();
    let task_id = run_id.clone();
    tokio::spawn(async move {
        let result = run_agent_spec(&spec, &trigger_data, &task_state).await;
        let status = match result.status {
            ExecutionStatus::Completed => RunStatus::Completed,
            _ => RunStatus::Failed,
        };
        let snapshot = serde_json::to_value(result.state.snapshot()).ok();
        if let Some(rec) = task_state.runs.write().await.get_mut(&task_id) {
            rec.status = status;
            rec.finished_at = Some(now_epoch());
            rec.error = result.error.clone();
            rec.state = snapshot;
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "run_id": run_id, "status": "running", "workflow": name })),
    ))
}

/// GET /api/v1/runs/{id}
pub(crate) async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunRecord>, (StatusCode, Json<ErrorResponse>)> {
    state
        .runs
        .read()
        .await
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("run '{id}' not found"),
                }),
            )
        })
}

// ---------------------------------------------------------------------------
// Tests — including the E2E: run crear-agente → session + fleet row
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::server::create_router;
    use crate::server::state::{AppState, LLMFactory};
    use crate::sessions::{FakeBackend, SessionManager};
    use crate::tools::builtin::register_all_builtin_tools;
    use crate::tools::registry::ToolRegistry;
    use crate::utils::short_id;

    fn test_llm_factory() -> LLMFactory {
        use crate::adapters::MockLLMResource;
        Arc::new(|| Box::new(MockLLMResource::new()))
    }

    /// AppState with all builtin tools registered (net/http_request included)
    /// and the orchestrator over a FakeBackend so no real tmux is spawned.
    fn e2e_state() -> (AppState, Arc<FakeBackend>, std::path::PathBuf) {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let backend = Arc::new(FakeBackend::new());
        let registry_path =
            std::env::temp_dir().join(format!("mirai-wf-e2e-{}.json", short_id()));
        let mut state = AppState::new(registry, test_llm_factory(), None);
        state.orchestrator = Arc::new(SessionManager::new(backend.clone(), registry_path.clone()));
        (state, backend, registry_path)
    }

    /// Path to the shipped workflows dir (`<repo>/agents`).
    fn agents_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("agents")
    }

    #[tokio::test]
    async fn unknown_workflow_is_404() {
        let (state, _b, path) = e2e_state();
        let mut state = state;
        state.workflows_dirs = vec![agents_dir()];
        let app = create_router(state);
        let resp = app
            .oneshot(
                Request::post("/api/v1/workflows/does-not-exist/run")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn traversal_name_is_400() {
        let (state, _b, path) = e2e_state();
        let app = create_router(state);
        let resp = app
            .oneshot(
                Request::post("/api/v1/workflows/..%2f..%2fetc%2fpasswd/run")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum decodes %2f; our guard rejects the traversal name.
        assert!(matches!(
            resp.status(),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
        ));
        let _ = std::fs::remove_file(path);
    }

    /// THE E2E: run the shipped `crear-agente` workflow through the run endpoint
    /// against a live in-process server. The workflow spawns a session (over the
    /// FakeBackend) and then WRITES the fleet SoT row via net/http_request →
    /// POST /fleet/status. We assert BOTH: the session exists AND the fleet has
    /// its row with status "working".
    #[tokio::test]
    async fn crear_agente_workflow_spawns_session_and_writes_fleet() {
        let (mut state, _backend, reg_path) = e2e_state();
        state.workflows_dirs = vec![agents_dir()];
        // Handles for assertions (Arcs shared with the router + run task).
        let orchestrator = state.orchestrator.clone();
        let fleet = state.fleet.clone();

        // Bind a real ephemeral server so the workflow's HTTP nodes can call back.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let app = create_router(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Kick the workflow: POST /workflows/crear-agente/run.
        let client = reqwest::Client::new();
        let run_resp = client
            .post(format!("{base_url}/api/v1/workflows/crear-agente/run"))
            .json(&json!({
                "base_url": base_url,
                "trigger_data": {
                    "name": "worker-e2e",
                    "project_dir": "/tmp",
                    "objective": "build the thing",
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(run_resp.status(), 202);
        let run_body: Value = run_resp.json().await.unwrap();
        let run_id = run_body["run_id"].as_str().unwrap().to_string();

        // Poll the run to completion.
        let mut final_run = Value::Null;
        for _ in 0..50 {
            let r: Value = client
                .get(format!("{base_url}/api/v1/runs/{run_id}"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if r["status"] != "running" {
                final_run = r;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(
            final_run["status"], "completed",
            "workflow run did not complete cleanly: {final_run}"
        );

        // 1) A session was actually spawned (FakeBackend).
        let sessions = orchestrator.list().await;
        assert_eq!(sessions.len(), 1, "expected exactly one spawned session");
        let session_id = sessions[0].id.clone();
        assert_eq!(sessions[0].objective, "build the thing");

        // 2) The fleet SoT has the row, keyed by the spawned session id,
        //    with status "working" — written BY THE WORKFLOW via /fleet/status.
        let members = fleet
            .list(&crate::fleet::FleetQuery::default())
            .expect("fleet list");
        assert_eq!(members.len(), 1, "expected exactly one fleet member");
        assert_eq!(members[0].id, session_id, "fleet row keyed by spawned id");
        assert_eq!(members[0].status, crate::fleet::FleetStatus::Working);
        assert_eq!(members[0].name, "worker-e2e");
        assert_eq!(members[0].objective, "build the thing");

        let _ = std::fs::remove_file(reg_path);
    }
}
