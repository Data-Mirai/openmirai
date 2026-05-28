//! HTTP server — axum-based API with auth, streaming, and graceful shutdown.
//!
//! Split into focused modules:
//! - [`state`]: AppState, LLMFactory, request/response types
//! - [`handlers`]: All endpoint implementations
//! - [`helpers`]: Agent execution helpers shared by handlers

pub mod handlers;
pub mod helpers;
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
        .route("/api/v1/agents/{id}/memory", get(get_agent_memory).delete(clear_agent_memory))
        // Tools
        .route("/api/v1/tools", get(list_tools))
        // Templates
        .route("/api/v1/templates", get(list_templates_handler))
        // Sessions
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{id}", get(get_session))
        // Universe
        .route("/api/v1/universe/message", post(universe_message))
        .route("/api/v1/universe/groupchat", post(groupchat))
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

    let provided = req
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok());

    match provided {
        Some(key) if key == expected => next.run(req).await,
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
pub async fn serve(
    host: &str,
    port: u16,
    llm_factory: state::LLMFactory,
    api_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if api_key.is_none() {
        tracing::warn!("No API key configured. Server is running without authentication.");
        tracing::warn!("Set MIRAI_API_KEY or use --api-key to enable authentication.");
    }

    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let state = AppState::new(registry, llm_factory, api_key);
    let app = create_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("datamirai-engine listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("Server stopped.");
    Ok(())
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
