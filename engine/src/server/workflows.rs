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

// The crear-agente E2E (run the workflow → spawned session + fleet SoT row)
// lives in the `mirai` crate, which owns the orchestrator + fleet layer that
// the workflow drives. See `mirai/src/server/tests.rs`.
