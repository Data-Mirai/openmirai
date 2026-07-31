//! Composed command-center HTTP server.
//!
//! The engine builds its core router ([`openmirai_engine::server::core_router`])
//! and exposes the shared security stack ([`openmirai_engine::server::apply_security`]).
//! This module adds the AgentMirai routes (`/api/v1/orchestrator/*`,
//! `/api/v1/fleet/*`, `/ui`) over [`MiraiState`] and merges both into ONE
//! router under a single auth / CORS / cross-site policy.

pub mod fleet;
pub mod objectives;
pub mod orchestrator;
pub mod state;

#[cfg(test)]
mod tests;

pub use state::{ErrorResponse, MiraiState};

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use openmirai_engine::server::{
    apply_security, bind_and_serve, core_app_state, core_router, expand_home, warn_if_no_api_key,
    AppState, LLMFactory,
};

use self::fleet::{fleet_events, fleet_list_agents, fleet_objective, fleet_status};
use self::objectives::{
    objectives_create, objectives_get, objectives_link_agents, objectives_list, objectives_patch,
};
use self::orchestrator::{
    orchestrator_create_session, orchestrator_events, orchestrator_get_activity,
    orchestrator_get_session, orchestrator_list_projects, orchestrator_list_sessions,
    orchestrator_output, orchestrator_pick_folder, orchestrator_record_activity,
    orchestrator_register_external, orchestrator_restart, orchestrator_send,
    orchestrator_set_external_status, orchestrator_stop, orchestrator_unregister_external,
    serve_ui, serve_ui_index,
};

// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------

/// Build the AgentMirai routes (orchestrator + fleet + static UI), state applied.
///
/// Returned as a finalized `Router` (state erased) so it merges cleanly onto
/// the engine's core router — see [`create_router`].
pub fn mirai_router(state: MiraiState) -> Router {
    Router::new()
        // Orchestrated Claude sessions over tmux (PRD-013)
        .route(
            "/api/v1/orchestrator/sessions",
            post(orchestrator_create_session).get(orchestrator_list_sessions),
        )
        // External nodes (bridge): register/drive nodes living on ANOTHER
        // substrate (FleetView subagents) so they show in the graph without a
        // tmux spawn. `register` is a static segment — declared before the
        // `{id}` route so it never gets captured as an id.
        .route(
            "/api/v1/orchestrator/sessions/register",
            post(orchestrator_register_external),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/status",
            post(orchestrator_set_external_status),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/unregister",
            post(orchestrator_unregister_external),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}",
            get(orchestrator_get_session).delete(orchestrator_unregister_external),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/send",
            post(orchestrator_send),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/output",
            get(orchestrator_output),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/stop",
            post(orchestrator_stop),
        )
        // Restart a tmux session in place with new flags (permission_mode /
        // model / effort), keeping the same id. The "bypass-all + restart"
        // action from the visualizer.
        .route(
            "/api/v1/orchestrator/sessions/{id}/restart",
            post(orchestrator_restart),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}/activity",
            post(orchestrator_record_activity).get(orchestrator_get_activity),
        )
        .route("/api/v1/orchestrator/events", get(orchestrator_events))
        // Known projects for the create-session picker (PRD-013 M7)
        .route(
            "/api/v1/orchestrator/projects",
            get(orchestrator_list_projects),
        )
        // Native host folder picker (PRD-013 M9)
        .route(
            "/api/v1/orchestrator/pick-folder",
            post(orchestrator_pick_folder),
        )
        // Fleet SoT (SQLite/WAL): back-way status ingress, list, live SSE.
        .route("/api/v1/fleet/status", post(fleet_status))
        .route("/api/v1/fleet/agents", get(fleet_list_agents))
        .route("/api/v1/fleet/objective/{parent_id}", get(fleet_objective))
        .route("/api/v1/fleet/events", get(fleet_events))
        // Objective SoT (SQLite/WAL): create/list/get/patch + agent linking.
        .route(
            "/api/v1/objectives",
            post(objectives_create).get(objectives_list),
        )
        .route(
            "/api/v1/objectives/{id}",
            get(objectives_get).patch(objectives_patch),
        )
        .route(
            "/api/v1/objectives/{id}/agents",
            post(objectives_link_agents),
        )
        // Static web UI served by the engine (PRD-013 M6) — no auth (localhost)
        .route("/ui", get(serve_ui_index))
        .route("/ui/{*path}", get(serve_ui))
        // State
        .with_state(state)
}

/// Compose the full command-center router: engine core + AgentMirai, under ONE
/// shared auth / CORS / cross-site policy (keyed by `core.api_key`).
pub fn create_router(core: AppState, mirai: MiraiState) -> Router {
    let api_key = core.api_key.clone();
    let merged = core_router(core).merge(mirai_router(mirai));
    apply_security(merged, api_key)
}

// ---------------------------------------------------------------------------
// serve()
// ---------------------------------------------------------------------------

/// Start the full AgentMirai HTTP server: core engine + orchestrator + fleet + UI.
///
/// `ui_dir` (PRD-013 M6): directory served under `/ui` (`--ui-dir` /
/// `MIRAI_UI_DIR`). `projects_dirs` (M7): colon-separated roots scanned for the
/// create-session picker. Graceful shutdown on SIGTERM / SIGINT.
pub async fn serve(
    host: &str,
    port: u16,
    llm_factory: LLMFactory,
    api_key: Option<String>,
    ui_dir: Option<String>,
    projects_dirs: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_no_api_key(host, port, &api_key);

    // Core engine state (graphs, agents, workflow runs) — shares this port so
    // net/http_request workflow nodes call back into this same server.
    let core = core_app_state(llm_factory, api_key.clone(), port);

    // AgentMirai state (sessions, fleet, picker, UI / projects roots).
    let mut mirai = MiraiState::new();
    mirai.ui_dir = ui_dir.map(std::path::PathBuf::from);
    mirai.projects_dirs = projects_dirs
        .as_deref()
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.trim().is_empty())
        .map(expand_home)
        .collect();
    if !mirai.projects_dirs.is_empty() {
        tracing::info!(
            "projects scan roots: {}",
            mirai
                .projects_dirs
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(dir) = &mirai.ui_dir {
        if dir.is_dir() {
            tracing::info!("serving web UI at /ui from {}", dir.display());
        } else {
            tracing::warn!("--ui-dir {} is not a directory; /ui will 404", dir.display());
        }
    }

    // PRD-013: load the persistent session registry, reconcile against real
    // tmux state, and start the ~2s status poll (no-op without active sessions).
    if let Err(e) = mirai.orchestrator.initialize().await {
        tracing::warn!("orchestrator: failed to load session registry: {e}");
    }
    // M6: sessions must report activity to THIS port.
    mirai.orchestrator.set_server_port(port);
    // M6: with --api-key the activity hook must authenticate too — inject the
    // key into spawned sessions (MIRAI_API_KEY) or every hook POST dies with a
    // silent 401 at the auth middleware.
    mirai.orchestrator.set_api_key(api_key.clone());
    mirai.orchestrator.start_polling();

    // Fleet SoT: swap the in-memory default for a WAL-mode file-backed store so
    // the fleet survives restarts. Falls back to the in-memory store on error.
    let fleet_path = crate::fleet::FleetStore::default_path();
    if let Some(parent) = fleet_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match crate::fleet::FleetStore::open(&fleet_path.to_string_lossy()) {
        Ok(store) => {
            tracing::info!("fleet SoT at {}", fleet_path.display());
            mirai.fleet = Arc::new(store);
        }
        Err(e) => tracing::warn!("fleet: falling back to in-memory store: {e}"),
    }

    // Objective SoT: shares the same WAL file as the fleet (disjoint tables), so
    // objectives + their agent links survive restarts next to the flota.
    match crate::objectives::ObjectiveStore::open(&fleet_path.to_string_lossy()) {
        Ok(store) => {
            tracing::info!("objective SoT at {}", fleet_path.display());
            mirai.objectives = Arc::new(store);
        }
        Err(e) => tracing::warn!("objectives: falling back to in-memory store: {e}"),
    }

    let app = create_router(core, mirai);
    bind_and_serve(host, port, app).await
}
