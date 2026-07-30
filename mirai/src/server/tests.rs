//! Composed-router integration tests for the AgentMirai layer.
//!
//! The headline case is the crear-agente E2E: run the shipped workflow through
//! the run endpoint against a live in-process server. The workflow spawns a
//! session (over a FakeBackend, no real tmux) and then WRITES the fleet SoT row
//! via `net/http_request` → `POST /fleet/status`. Both the orchestrator route
//! and the fleet route live in this crate, so the E2E belongs here.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use openmirai_engine::server::{core_app_state, AppState, LLMFactory};

use super::create_router;
use crate::sessions::{FakeBackend, SessionManager};
use crate::server::state::MiraiState;
use openmirai_engine::utils::short_id;

fn test_llm_factory() -> LLMFactory {
    use openmirai_engine::MockLLMResource;
    Arc::new(|| Box::new(MockLLMResource::new()))
}

/// Path to the shipped workflows dir (`<repo>/agents`).
fn agents_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("agents")
}

/// Core state pointing its workflow-run dir at the shipped `agents/` dir.
fn e2e_core() -> AppState {
    let mut core = core_app_state(test_llm_factory(), None, 0);
    core.workflows_dirs = vec![agents_dir()];
    core
}

/// A MiraiState whose orchestrator runs on a FakeBackend + temp registry.
fn e2e_mirai() -> (MiraiState, Arc<FakeBackend>, std::path::PathBuf) {
    let backend = Arc::new(FakeBackend::new());
    let registry_path = std::env::temp_dir().join(format!("mirai-wf-e2e-{}.json", short_id()));
    let mut mirai = MiraiState::new();
    mirai.orchestrator = Arc::new(SessionManager::new(backend.clone(), registry_path.clone()));
    (mirai, backend, registry_path)
}

#[tokio::test]
async fn unknown_workflow_is_404() {
    let (mirai, _b, path) = e2e_mirai();
    let app = create_router(e2e_core(), mirai);
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
    let (mirai, _b, path) = e2e_mirai();
    let app = create_router(e2e_core(), mirai);
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
    let (mirai, _backend, reg_path) = e2e_mirai();
    // Handles for assertions (Arcs shared with the router + run task).
    let orchestrator = mirai.orchestrator.clone();
    let fleet = mirai.fleet.clone();

    // Bind a real ephemeral server so the workflow's HTTP nodes can call back.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let app = create_router(e2e_core(), mirai);
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

/// THE pub-sub E2E: run the shipped `reportar-hijo` and `leer-objetivo`
/// workflows through the run endpoint against a live in-process server, and
/// assert the reactive `objective_complete` notification fires when the last
/// child reports done — observed through a filtering [`FleetSubscriber`].
#[tokio::test]
async fn reportar_hijo_completes_objective_and_notifies_subscriber() {
    use crate::fleet::{FleetStatus, FleetSubscriber, StatusUpdate};

    let (mirai, _backend, reg_path) = e2e_mirai();
    let fleet = mirai.fleet.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let app = create_router(e2e_core(), mirai);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    // Run a workflow to completion and assert it finished cleanly.
    let run = |wf: &'static str, data: Value| {
        let client = client.clone();
        let base_url = base_url.clone();
        async move {
            let resp = client
                .post(format!("{base_url}/api/v1/workflows/{wf}/run"))
                .json(&json!({ "base_url": base_url, "trigger_data": data }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 202, "{wf} run not accepted");
            let run_id = resp.json::<Value>().await.unwrap()["run_id"]
                .as_str()
                .unwrap()
                .to_string();
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
            assert_eq!(final_run["status"], "completed", "{wf}: {final_run}");
            final_run
        }
    };

    // Parent + two working children exist first.
    fleet
        .apply_status(StatusUpdate {
            id: "boss".into(),
            status: FleetStatus::Working,
            name: None,
            kind: None,
            host: None,
            activity: None,
            objective: Some("ship the release".into()),
            parent_id: None,
            project_dir: None,
            metadata: None,
        })
        .unwrap();
    run("reportar-hijo", json!({"id":"h1","parent_id":"boss","status":"working"})).await;
    run("reportar-hijo", json!({"id":"h2","parent_id":"boss","status":"working"})).await;

    // Subscribe for boss's completion, THEN drive both children to done. The
    // broadcast buffers the event, so no race with the async runs.
    let mut sub = FleetSubscriber::objective_complete(fleet.clone(), Some("boss".into()));
    run("reportar-hijo", json!({"id":"h1","parent_id":"boss","status":"done"})).await;
    run("reportar-hijo", json!({"id":"h2","parent_id":"boss","status":"done"})).await;

    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), sub.recv())
        .await
        .expect("objective_complete not delivered")
        .expect("stream closed");
    assert_eq!(ev.member.id, "boss");
    assert_eq!(ev.member.objective, "ship the release");
    assert!(ev.progress.as_ref().unwrap().complete);

    // The `leer-objetivo` workflow runs cleanly on the pull side (validates the
    // shipped YAML end-to-end)...
    run("leer-objetivo", json!({"parent_id":"boss"})).await;
    // ...and the aggregation endpoint it wraps reports the completion over HTTP.
    let obj: Value = client
        .get(format!("{base_url}/api/v1/fleet/objective/boss"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(obj["total"], 2);
    assert_eq!(obj["done"], 2);
    assert_eq!(obj["complete"], true);
    assert_eq!(obj["children"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_file(reg_path);
}
