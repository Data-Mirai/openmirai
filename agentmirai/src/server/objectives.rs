//! HTTP API for the objective SoT (`/api/v1/objectives`).
//!
//! The pinned contract (shared with the wizard / AgentMirai):
//!
//! | Endpoint | Method | Body / Response |
//! |---|---|---|
//! | `/objectives` | POST | `{text, project_dir?, status?, agents?}` → `{id, …}` |
//! | `/objectives` | GET | → `{objectives: [obj…], count}` |
//! | `/objectives/{id}` | GET | → `{id, text, status, project_dir, created, updated, agents: [member…]}` |
//! | `/objectives/{id}` | PATCH | `{status}` → the updated objective |
//! | `/objectives/{id}/agents` | POST | `{fleet_id}` \| `{fleet_ids: […]}` → link agent(s) |
//!
//! `agents` on `GET /objectives/{id}` are resolved through the `objective_agents`
//! bridge: each linked fleet id is looked up in the fleet SoT and returned as the
//! full member record (or a bare `{id}` when it is not in the fleet yet).
//!
//! Auth is the same optional `X-API-Key` middleware as the rest of the API.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::fleet::FleetMember;
use crate::objectives::{NewObjective, Objective, ObjectiveError, ObjectiveStatus};

use super::state::{ErrorResponse, MiraiState};

fn objective_error_response(err: ObjectiveError) -> (StatusCode, Json<ErrorResponse>) {
    let status = match &err {
        ObjectiveError::Invalid(_) => StatusCode::BAD_REQUEST,
        ObjectiveError::NotFound(_) => StatusCode::NOT_FOUND,
        ObjectiveError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

/// The wire shape of an objective: `created_at`/`updated_at` become the contract
/// keys `created`/`updated`.
fn objective_json(o: &Objective) -> Value {
    json!({
        "id": o.id,
        "text": o.text,
        "status": o.status,
        "project_dir": o.project_dir,
        "created": o.created_at,
        "updated": o.updated_at,
    })
}

// ---------------------------------------------------------------------------
// POST /api/v1/objectives — create
// ---------------------------------------------------------------------------

/// Accepts one or many fleet ids so the wizard can link agents at creation and
/// via `POST /{id}/agents` with the same shape.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AgentsField {
    #[serde(default)]
    fleet_id: Option<String>,
    #[serde(default)]
    fleet_ids: Option<Vec<String>>,
}

impl AgentsField {
    /// Flatten `fleet_id` + `fleet_ids` into one de-blanked list.
    fn collect(self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(one) = self.fleet_id {
            out.push(one);
        }
        if let Some(many) = self.fleet_ids {
            out.extend(many);
        }
        out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateObjectiveBody {
    pub text: String,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub status: Option<ObjectiveStatus>,
    /// Optional fleet ids to link at creation (contract superset — omit for the
    /// bare `{text}` create).
    #[serde(default)]
    pub agents: Option<Vec<String>>,
}

/// POST /api/v1/objectives
pub(crate) async fn objectives_create(
    State(state): State<MiraiState>,
    Json(body): Json<CreateObjectiveBody>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let objective = state
        .objectives
        .create(NewObjective {
            text: body.text,
            project_dir: body.project_dir.unwrap_or_default(),
            status: body.status.unwrap_or_default(),
            agents: body.agents.unwrap_or_default(),
        })
        .map_err(objective_error_response)?;
    // Contract is `{id}`; we return the full record (a superset) so the wizard
    // gets timestamps + status without a second GET.
    Ok(Json(objective_json(&objective)))
}

// ---------------------------------------------------------------------------
// GET /api/v1/objectives — list
// ---------------------------------------------------------------------------

/// GET /api/v1/objectives
pub(crate) async fn objectives_list(
    State(state): State<MiraiState>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let objectives = state.objectives.list().map_err(objective_error_response)?;
    let items: Vec<Value> = objectives.iter().map(objective_json).collect();
    Ok(Json(json!({ "objectives": items, "count": items.len() })))
}

// ---------------------------------------------------------------------------
// GET /api/v1/objectives/{id} — one objective + its linked agents
// ---------------------------------------------------------------------------

/// GET /api/v1/objectives/{id}
pub(crate) async fn objectives_get(
    State(state): State<MiraiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let objective = state
        .objectives
        .get(&id)
        .map_err(objective_error_response)?
        .ok_or_else(|| objective_error_response(ObjectiveError::NotFound(format!("objective '{id}'"))))?;

    // Resolve the bridge: each linked fleet id → its full member record, or a
    // bare `{id}` when the agent is not (yet) in the fleet SoT.
    let fleet_ids = state
        .objectives
        .agent_ids(&id)
        .map_err(objective_error_response)?;
    let agents: Vec<Value> = fleet_ids
        .iter()
        .map(|fid| match state.fleet.get(fid) {
            Ok(Some(member)) => member_json(&member),
            _ => json!({ "id": fid }),
        })
        .collect();

    let mut out = objective_json(&objective);
    out["agents"] = json!(agents);
    Ok(Json(out))
}

fn member_json(m: &FleetMember) -> Value {
    serde_json::to_value(m).unwrap_or_else(|_| json!({}))
}

// ---------------------------------------------------------------------------
// PATCH /api/v1/objectives/{id} — update status
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct PatchObjectiveBody {
    pub status: ObjectiveStatus,
}

/// PATCH /api/v1/objectives/{id}
pub(crate) async fn objectives_patch(
    State(state): State<MiraiState>,
    Path(id): Path<String>,
    Json(body): Json<PatchObjectiveBody>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let objective = state
        .objectives
        .set_status(&id, body.status)
        .map_err(objective_error_response)?;
    Ok(Json(objective_json(&objective)))
}

// ---------------------------------------------------------------------------
// POST /api/v1/objectives/{id}/agents — link agent(s) to the objective
// ---------------------------------------------------------------------------

/// POST /api/v1/objectives/{id}/agents
///
/// Populates the objective↔agent bridge. Accepts `{fleet_id}` or
/// `{fleet_ids: [...]}` (both may be combined). Idempotent per pair.
pub(crate) async fn objectives_link_agents(
    State(state): State<MiraiState>,
    Path(id): Path<String>,
    Json(body): Json<AgentsField>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let fleet_ids = body.collect();
    if fleet_ids.is_empty() {
        return Err(objective_error_response(ObjectiveError::Invalid(
            "fleet_id or fleet_ids is required".into(),
        )));
    }
    let mut linked = 0usize;
    for fid in &fleet_ids {
        if state
            .objectives
            .link_agent(&id, fid)
            .map_err(objective_error_response)?
        {
            linked += 1;
        }
    }
    let agents = state
        .objectives
        .agent_ids(&id)
        .map_err(objective_error_response)?;
    Ok(Json(json!({
        "id": id,
        "linked": linked,
        "agents": agents,
    })))
}

// ---------------------------------------------------------------------------
// Tests — handlers over an in-memory store (router-level, tower oneshot)
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
    use crate::server::state::MiraiState;
    use openmirai_engine::server::{core_app_state, AppState, LLMFactory};

    fn test_llm_factory() -> LLMFactory {
        use openmirai_engine::MockLLMResource;
        Arc::new(|| Box::new(MockLLMResource::new()))
    }

    fn test_core() -> AppState {
        core_app_state(test_llm_factory(), None, 0)
    }

    fn router(state: MiraiState) -> Router {
        create_router(test_core(), state)
    }

    fn test_app() -> Router {
        router(MiraiState::new())
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        (status, body_json(resp.into_body()).await)
    }

    fn post(path: &str, body: Value) -> Request<Body> {
        Request::post(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn patch(path: &str, body: Value) -> Request<Body> {
        Request::patch(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn get(path: &str) -> Request<Body> {
        Request::get(path).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn create_then_get_and_list() {
        let app = test_app();

        // POST → {id, …}
        let (st, created) =
            send(&app, post("/api/v1/objectives", json!({"text": "ship the SoT"}))).await;
        assert_eq!(st, StatusCode::OK);
        let id = created["id"].as_str().expect("id in response").to_string();
        assert!(!id.is_empty());
        assert_eq!(created["text"], "ship the SoT");
        assert_eq!(created["status"], "open");
        assert!(created["created"].is_number());

        // GET /{id} → the saved objective with an (empty) agents list.
        let (st, one) = send(&app, get(&format!("/api/v1/objectives/{id}"))).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(one["id"], id);
        assert_eq!(one["text"], "ship the SoT");
        assert_eq!(one["status"], "open");
        assert!(one["created"].is_number());
        assert_eq!(one["agents"].as_array().unwrap().len(), 0);

        // GET list → contains it.
        let (st, list) = send(&app, get("/api/v1/objectives")).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list["count"], 1);
        assert_eq!(list["objectives"][0]["id"], id);
    }

    #[tokio::test]
    async fn create_requires_text() {
        let app = test_app();
        let (st, body) = send(&app, post("/api/v1/objectives", json!({"text": "   "}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("text"));
    }

    #[tokio::test]
    async fn create_with_project_dir_and_status() {
        let app = test_app();
        let (st, created) = send(
            &app,
            post(
                "/api/v1/objectives",
                json!({"text": "x", "project_dir": "/proj", "status": "working"}),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(created["project_dir"], "/proj");
        assert_eq!(created["status"], "working");
    }

    #[tokio::test]
    async fn get_missing_is_404() {
        let app = test_app();
        let (st, _) = send(&app, get("/api/v1/objectives/ghost")).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn patch_updates_status() {
        let app = test_app();
        let (_st, created) =
            send(&app, post("/api/v1/objectives", json!({"text": "goal"}))).await;
        let id = created["id"].as_str().unwrap().to_string();

        let (st, patched) = send(
            &app,
            patch(&format!("/api/v1/objectives/{id}"), json!({"status": "done"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(patched["status"], "done");

        // GET reflects it.
        let (_st, one) = send(&app, get(&format!("/api/v1/objectives/{id}"))).await;
        assert_eq!(one["status"], "done");
    }

    #[tokio::test]
    async fn patch_missing_is_404() {
        let app = test_app();
        let (st, _) = send(
            &app,
            patch("/api/v1/objectives/ghost", json!({"status": "done"})),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn link_agents_and_get_resolves_bridge() {
        // The bridge join: link a fleet id, seed that member in the fleet SoT,
        // and GET /{id} returns the full member record under `agents`.
        let state = MiraiState::new();
        state
            .fleet
            .apply_status(crate::fleet::StatusUpdate {
                id: "agent-a".into(),
                status: crate::fleet::FleetStatus::Working,
                name: Some("Worker A".into()),
                kind: None,
                host: Some("mac".into()),
                activity: None,
                objective: None,
                parent_id: None,
                project_dir: None,
                metadata: None,
            })
            .unwrap();
        let app = router(state);

        let (_st, created) =
            send(&app, post("/api/v1/objectives", json!({"text": "goal"}))).await;
        let id = created["id"].as_str().unwrap().to_string();

        // Link one known agent + one not-yet-in-fleet agent.
        let (st, linked) = send(
            &app,
            post(
                &format!("/api/v1/objectives/{id}/agents"),
                json!({"fleet_ids": ["agent-a", "agent-ghost"]}),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(linked["linked"], 2);

        let (st, one) = send(&app, get(&format!("/api/v1/objectives/{id}"))).await;
        assert_eq!(st, StatusCode::OK);
        let agents = one["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
        // Known agent → full record.
        assert_eq!(agents[0]["id"], "agent-a");
        assert_eq!(agents[0]["name"], "Worker A");
        assert_eq!(agents[0]["host"], "mac");
        // Unknown agent → bare {id} placeholder (link still visible).
        assert_eq!(agents[1]["id"], "agent-ghost");
    }

    #[tokio::test]
    async fn create_links_agents_inline() {
        let app = test_app();
        let (_st, created) = send(
            &app,
            post(
                "/api/v1/objectives",
                json!({"text": "goal", "agents": ["a", "b"]}),
            ),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_string();
        let (_st, one) = send(&app, get(&format!("/api/v1/objectives/{id}"))).await;
        let agents = one["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn link_agents_requires_a_fleet_id() {
        let app = test_app();
        let (_st, created) =
            send(&app, post("/api/v1/objectives", json!({"text": "goal"}))).await;
        let id = created["id"].as_str().unwrap().to_string();
        let (st, _) = send(
            &app,
            post(&format!("/api/v1/objectives/{id}/agents"), json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn link_agents_to_missing_objective_is_404() {
        let app = test_app();
        let (st, _) = send(
            &app,
            post("/api/v1/objectives/ghost/agents", json!({"fleet_id": "a"})),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
}
