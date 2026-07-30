//! HTTP API for the fleet SoT (`/api/v1/fleet/*`).
//!
//! The pinned contract:
//!
//! | Endpoint | Method | Body / Response |
//! |---|---|---|
//! | `/fleet/status` | POST | `{id, status, name?, kind?, host?, activity?, objective?, project_dir?, metadata?}` → `{id, status, last_seen, event}` |
//! | `/fleet/agents` | GET | `?status=&host=&limit=&stale_secs=` → `{agents: [member…], count}` |
//! | `/fleet/events` | GET (SSE) | `fleet_member_added` / `fleet_member_updated` / `fleet_member_removed`, `data:` = member record |
//!
//! `POST /fleet/status` is the "back-way status" ingress: any fleet member
//! (this Mac, the VPS, a subagent) reports its live status back to the center,
//! which persists it in the WAL SQLite SoT and fans it out over SSE.
//!
//! Auth is the same optional `X-API-Key` middleware as the rest of the API; the
//! SSE endpoint additionally accepts the key as `?api_key=` (EventSource cannot
//! set headers — see the auth middleware).

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::fleet::{FleetError, FleetEventKind, FleetMember, FleetQuery, FleetStatus, StatusUpdate};

use super::state::{MiraiState, ErrorResponse};

fn fleet_error_response(err: FleetError) -> (StatusCode, Json<ErrorResponse>) {
    let status = match &err {
        FleetError::Invalid(_) => StatusCode::BAD_REQUEST,
        FleetError::NotFound(_) => StatusCode::NOT_FOUND,
        FleetError::Db(_) | FleetError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

/// Serialize a member to its API JSON, optionally annotating heartbeat
/// staleness when the caller passed `?stale_secs=N` (member last seen more than
/// N seconds ago → `"stale": true`).
fn member_json(m: &FleetMember, stale_secs: Option<f64>, now: f64) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or_else(|_| json!({}));
    if let Some(threshold) = stale_secs {
        let stale = now - m.last_seen > threshold;
        v["stale"] = json!(stale);
    }
    v
}

// ---------------------------------------------------------------------------
// POST /api/v1/fleet/status  — back-way status ingress
// ---------------------------------------------------------------------------

/// POST /api/v1/fleet/status
pub(crate) async fn fleet_status(
    State(state): State<MiraiState>,
    Json(update): Json<StatusUpdate>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let (member, kind) = state
        .fleet
        .apply_status(update)
        .map_err(fleet_error_response)?;

    let event = match kind {
        FleetEventKind::Added => "added",
        FleetEventKind::Updated => "updated",
        FleetEventKind::Removed => "removed",
    };
    Ok(Json(json!({
        "id": member.id,
        "status": member.status,
        "last_seen": member.last_seen,
        "event": event,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/v1/fleet/agents — list the fleet
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct AgentsQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// When set, each member gets a `stale` flag: `last_seen` older than this
    /// many seconds ago.
    #[serde(default)]
    pub stale_secs: Option<f64>,
}

/// GET /api/v1/fleet/agents
pub(crate) async fn fleet_list_agents(
    State(state): State<MiraiState>,
    Query(query): Query<AgentsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let status = match &query.status {
        Some(s) => Some(FleetStatus::parse(s).ok_or_else(|| {
            fleet_error_response(FleetError::Invalid(format!(
                "invalid status '{s}' (want online|offline|working|waiting|idle|done|error|unknown)"
            )))
        })?),
        None => None,
    };
    let filter = FleetQuery {
        status,
        host: query.host.filter(|h| !h.is_empty()),
        limit: query.limit.map(|l| l.clamp(1, 5000)),
    };
    let members = state.fleet.list(&filter).map_err(fleet_error_response)?;
    let now = openmirai_engine::utils::now_epoch();
    let agents: Vec<Value> = members
        .iter()
        .map(|m| member_json(m, query.stale_secs, now))
        .collect();

    Ok(Json(json!({ "agents": agents, "count": agents.len() })))
}

// ---------------------------------------------------------------------------
// GET /api/v1/fleet/events — SSE stream of fleet changes
// ---------------------------------------------------------------------------

/// GET /api/v1/fleet/events
///
/// Emits `fleet_member_added`, `fleet_member_updated`, `fleet_member_removed`.
/// The `data:` payload is the [`FleetMember`] record DIRECTLY (not an internal
/// envelope), matching the fleet-view UI contract.
pub(crate) async fn fleet_events(State(state): State<MiraiState>) -> impl IntoResponse {
    use axum::body::Body;
    use tokio_stream::wrappers::ReceiverStream;

    let mut events = state.fleet.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256);

    tokio::spawn(async move {
        // Open the stream immediately so clients get headers + a first byte.
        if tx.send(Ok(": connected\n\n".to_string())).await.is_err() {
            return;
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    let payload =
                        serde_json::to_string(&event.member).unwrap_or_else(|_| "{}".into());
                    let frame =
                        format!("event: {}\ndata: {}\n\n", event.kind.event_name(), payload);
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
// Tests — handlers over an in-memory fleet store (router-level, tower oneshot)
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

    /// A core engine [`AppState`] with builtin tools, no api key.
    fn test_core() -> AppState {
        core_app_state(test_llm_factory(), None, 0)
    }

    /// Compose the full command-center router around `state` (no api key).
    fn router(state: MiraiState) -> Router {
        create_router(test_core(), state)
    }

    /// Same, but with an api key gating the shared security middleware.
    fn router_with_key(state: MiraiState, api_key: Option<String>) -> Router {
        create_router(core_app_state(test_llm_factory(), api_key, 0), state)
    }

    fn test_app() -> Router {
        router(MiraiState::new())
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn post_status(app: &Router, body: Value) -> (StatusCode, Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/fleet/status")
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
    async fn status_adds_then_updates_and_lists() {
        let app = test_app();

        // First report → added.
        let (st, body) = post_status(
            &app,
            json!({"id": "mac-centro", "status": "working", "name": "Centro", "host": "mac"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["event"], "added");
        assert_eq!(body["status"], "working");
        assert!(body["last_seen"].is_number());

        // Second report (bare heartbeat) → updated.
        let (_st, body) =
            post_status(&app, json!({"id": "mac-centro", "status": "idle"})).await;
        assert_eq!(body["event"], "updated");

        // GET /agents returns the merged record.
        let resp = app
            .oneshot(
                Request::get("/api/v1/fleet/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let listing = body_json(resp.into_body()).await;
        assert_eq!(listing["count"], 1);
        let agent = &listing["agents"][0];
        assert_eq!(agent["id"], "mac-centro");
        assert_eq!(agent["name"], "Centro", "name preserved across heartbeat");
        assert_eq!(agent["host"], "mac");
        assert_eq!(agent["status"], "idle");
        for key in [
            "id", "name", "kind", "host", "status", "activity", "objective", "project_dir",
            "metadata", "first_seen", "last_seen",
        ] {
            assert!(agent.get(key).is_some(), "missing contract field {key}");
        }
    }

    #[tokio::test]
    async fn status_requires_id() {
        let app = test_app();
        let (st, body) = post_status(&app, json!({"id": "  ", "status": "online"})).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("id"));
    }

    #[tokio::test]
    async fn status_rejects_unknown_status_value() {
        let app = test_app();
        // Unknown enum variant → axum body deserialization fails → 422.
        let resp = app
            .oneshot(
                Request::post("/api/v1/fleet/status")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"id": "x", "status": "bogus"})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn agents_filters_by_status_and_host() {
        let app = test_app();
        post_status(&app, json!({"id": "a", "status": "working", "host": "mac"})).await;
        post_status(&app, json!({"id": "b", "status": "idle", "host": "vps"})).await;

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/agents?status=idle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["count"], 1);
        assert_eq!(body["agents"][0]["id"], "b");

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/agents?host=mac")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["count"], 1);
        assert_eq!(body["agents"][0]["id"], "a");

        // Bad status filter → 400.
        let resp = app
            .oneshot(
                Request::get("/api/v1/fleet/agents?status=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn agents_stale_secs_flags_old_heartbeats() {
        let app = test_app();
        post_status(&app, json!({"id": "old", "status": "online"})).await;
        // Threshold 0 → anything reported before "now" is stale.
        let resp = app
            .oneshot(
                Request::get("/api/v1/fleet/agents?stale_secs=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(resp.into_body()).await;
        assert!(body["agents"][0]["stale"].is_boolean());
    }

    #[tokio::test]
    async fn events_streams_status_reports() {
        let state = MiraiState::new();
        let app = router(state.clone());

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/events")
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
        use futures_util::StreamExt;

        let first = body.next().await.unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&first), ": connected\n\n");

        // Report a status directly through the store → SSE frame.
        state
            .fleet
            .apply_status(crate::fleet::StatusUpdate {
                id: "worker".into(),
                status: crate::fleet::FleetStatus::Working,
                name: Some("Worker".into()),
                kind: None,
                host: None,
                activity: None,
                objective: None,
                project_dir: None,
                metadata: None,
            })
            .unwrap();

        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("timed out waiting for SSE event")
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(
            text.starts_with("event: fleet_member_added\n"),
            "got: {text}"
        );
        let data_line = text
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .expect("data line");
        let payload: Value = serde_json::from_str(data_line).unwrap();
        assert_eq!(payload["id"], "worker");
        assert_eq!(payload["status"], "working");
        assert_eq!(payload["name"], "Worker");
    }

    #[tokio::test]
    async fn fleet_routes_respect_api_key_and_sse_query_param() {
        let state = MiraiState::new();
        let app = router_with_key(state, Some("secret".into()));

        // GET /agents without key → 401.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // With header key → 200.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/agents")
                    .header("X-API-Key", "secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // SSE via query-param key (EventSource can't set headers) → 200.
        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/v1/fleet/events?api_key=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // SSE with wrong query key → 401.
        let resp = app
            .oneshot(
                Request::get("/api/v1/fleet/events?api_key=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
