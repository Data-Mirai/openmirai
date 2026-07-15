//! HTTP server — axum-based API with auth, streaming, and graceful shutdown.
//!
//! Split into focused modules:
//! - [`state`]: AppState, LLMFactory, request/response types
//! - [`handlers`]: All endpoint implementations
//! - [`helpers`]: Agent execution helpers shared by handlers

pub mod editor;
pub mod handlers;
pub mod helpers;
pub mod orchestrator;
pub mod state;

#[cfg(test)]
mod tests;

// Public API re-exports.
pub use state::*;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use self::handlers::*;
use self::helpers::rag_search;
use self::orchestrator::{
    orchestrator_create_session, orchestrator_events, orchestrator_get_activity,
    orchestrator_get_session, orchestrator_list_sessions, orchestrator_output,
    orchestrator_list_projects, orchestrator_pick_folder, orchestrator_record_activity,
    orchestrator_send, orchestrator_stop, serve_ui, serve_ui_index,
};

// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------

/// Build the axum router with all endpoints wired up.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health / version — always public (no auth, no version prefix)
        .route("/health", get(health))
        .route("/version", get(version))
        // ---- API v1 ----
        // Graphs CRUD
        .route("/api/v1/graphs", post(create_graph).get(list_graphs))
        .route("/api/v1/graphs/{id}", get(get_graph).delete(delete_graph))
        // Agents CRUD
        .route("/api/v1/agents", post(create_agent).get(list_agents))
        .route("/api/v1/agents/from-spec", post(create_agent_from_spec))
        .route("/api/v1/agents/{id}", get(get_agent))
        .route("/api/v1/agents/{id}/execute", post(execute_agent))
        .route("/api/v1/agents/{id}/stream", post(stream_agent))
        .route("/api/v1/agents/{id}/spec", get(get_agent_spec))
        .route("/api/v1/agents/{id}/schema", get(get_agent_schema))
        // Live agent lifecycle (PRD-008)
        .route("/api/v1/agents/{id}/play", post(play_agent))
        .route("/api/v1/agents/{id}/stop", post(stop_agent))
        .route("/api/v1/agents/{id}/cycles", get(get_agent_cycles))
        .route(
            "/api/v1/agents/{id}/memory",
            get(get_agent_memory).delete(clear_agent_memory),
        )
        // Tools
        .route("/api/v1/tools", get(list_tools))
        // Templates
        .route("/api/v1/templates", get(list_templates_handler))
        // Sessions
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{id}", get(get_session))
        .route("/api/v1/sessions/{id}/otel-trace", get(get_session_otel_trace))
        // Universe
        .route("/api/v1/universe/message", post(universe_message))
        .route("/api/v1/universe/groupchat", post(groupchat))
        // Orchestrated Claude sessions over tmux (PRD-013)
        .route(
            "/api/v1/orchestrator/sessions",
            post(orchestrator_create_session).get(orchestrator_list_sessions),
        )
        .route(
            "/api/v1/orchestrator/sessions/{id}",
            get(orchestrator_get_session),
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
        // Static web UI served by the engine (PRD-013 M6) — no auth (localhost)
        .route("/ui", get(serve_ui_index))
        .route("/ui/{*path}", get(serve_ui))
        // Metrics
        .route("/api/v1/metrics", get(get_metrics))
        // RAG + Eval
        .route("/api/v1/rag/search", post(rag_search))
        .route("/api/v1/eval", post(eval_session))
        // Webhooks (no version prefix)
        .route("/webhooks/{*path}", post(webhook_handler))
        // Middleware: API key auth (if configured)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        // Middleware: CORS
        .layer(CorsLayer::permissive())
        // State
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let expected = match &state.api_key {
        Some(k) => k,
        None => return next.run(req).await,
    };

    let path = req.uri().path();
    if path == "/health" || path == "/version" {
        return next.run(req).await;
    }
    // PRD-013 M6: the static UI is public (localhost convenience); the API
    // it talks to keeps its auth.
    if path == "/ui" || path.starts_with("/ui/") {
        return next.run(req).await;
    }

    let provided = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

    // PRD-013: EventSource cannot set headers, so the orchestrator SSE
    // endpoint also accepts the key as a `?api_key=` query param.
    let query_key: Option<String> = if path == "/api/v1/orchestrator/events" {
        req.uri().query().and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "api_key")
                .map(|(_, v)| v.into_owned())
        })
    } else {
        None
    };
    let provided = provided.map(str::to_string).or(query_key);

    match provided {
        Some(key) if key == *expected => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// serve()
// ---------------------------------------------------------------------------

/// Start the HTTP server. Supports graceful shutdown on SIGTERM / SIGINT.
///
/// `ui_dir` (PRD-013 M6): directory served as static files under `/ui`
/// (`--ui-dir` flag or `MIRAI_UI_DIR` env). `None` → `/ui` answers 404 with
/// a clear message.
pub async fn serve(
    host: &str,
    port: u16,
    llm_factory: state::LLMFactory,
    api_key: Option<String>,
    ui_dir: Option<String>,
    projects_dirs: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if api_key.is_none() {
        tracing::warn!("No API key configured. Server is running without authentication.");
        tracing::warn!("Set MIRAI_API_KEY or use --api-key to enable authentication.");
    }

    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let mut state = AppState::new(registry, llm_factory, api_key);
    state.ui_dir = ui_dir.map(std::path::PathBuf::from);
    // M7: roots for the /projects scan — colon-separated, `~` expanded.
    state.projects_dirs = projects_dirs
        .as_deref()
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.trim().is_empty())
        .map(expand_home)
        .collect();
    if !state.projects_dirs.is_empty() {
        tracing::info!(
            "projects scan roots: {}",
            state
                .projects_dirs
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(dir) = &state.ui_dir {
        if dir.is_dir() {
            tracing::info!("serving web UI at /ui from {}", dir.display());
        } else {
            tracing::warn!("--ui-dir {} is not a directory; /ui will 404", dir.display());
        }
    }

    // PRD-013: load the persistent session registry, reconcile against real
    // tmux state, and start the ~2s status poll (no-op without active sessions).
    if let Err(e) = state.orchestrator.initialize().await {
        tracing::warn!("orchestrator: failed to load session registry: {e}");
    }
    // M6: sessions must report activity to THIS port.
    state.orchestrator.set_server_port(port);
    state.orchestrator.start_polling();

    let app = create_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("openmirai-engine listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("Server stopped.");
    Ok(())
}

/// Expand a leading `~` / `~/` to $HOME (M7 projects roots).
fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    if path == "~" || path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::Path::new(&home).join(path.trim_start_matches("~/").trim_start_matches('~'));
        }
    }
    std::path::PathBuf::from(path)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, finishing in-flight requests...");
}
