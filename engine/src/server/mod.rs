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
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use self::handlers::*;
use self::helpers::rag_search;
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
        .route(
            "/api/v1/sessions/{id}/otel-trace",
            get(get_session_otel_trace),
        )
        // Universe
        .route("/api/v1/universe/message", post(universe_message))
        .route("/api/v1/universe/groupchat", post(groupchat))
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
        // Middleware: cross-site guard for mutating requests (defense in depth
        // for endpoints that CORS preflights don't cover — see cross_origin_guard).
        .layer(axum::middleware::from_fn(cross_origin_guard))
        // Middleware: CORS — outermost so it answers preflights itself.
        .layer(cors_layer())
        // State
        .with_state(state)
}

// ---------------------------------------------------------------------------
// CORS + cross-site guard (security gate 0.7.0 — drive-by RCE fix)
// ---------------------------------------------------------------------------

/// CORS restricted to local UIs, replacing `CorsLayer::permissive()`.
///
/// Permissive CORS reflected ANY origin, so a malicious page (https://evil.com)
/// open in the user's browser could pass the preflight and POST to
/// `/api/v1/agents/{id}/execute` or the orchestrator spawn/send endpoints of a
/// locally running `mirai serve` / `mirai edit` — cross-site request that ends
/// in command execution on the developer's machine (drive-by RCE).
///
/// The predicate only trusts:
/// - **loopback origins** — `http(s)://localhost`, `127.0.0.0/8`, `[::1]`, any
///   port: covers the mirai serve/edit UIs and any local dev server.
/// - the literal **`null` origin, ONLY on `/api/v1/orchestrator/*`** — the
///   Claude-Orchestrator single-file web UI is opened via `file://`, and
///   `file://` pages send `Origin: null`. Scoping `null` to the orchestrator
///   API keeps that UI working without re-opening `execute` and the rest.
///
/// Residual risk (documented): sandboxed iframes
/// (`<iframe sandbox="allow-scripts">`) also send `Origin: null`, so a hostile
/// page can still reach the orchestrator endpoints through one when the server
/// runs WITHOUT `--api-key`. With a key configured the auth middleware rejects
/// those requests — running with `MIRAI_API_KEY` closes this gap completely.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, parts| {
            origin_is_trusted(origin, parts.uri.path())
        }))
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
}

/// Shared trust decision for CORS and the cross-site guard.
fn origin_is_trusted(origin: &HeaderValue, path: &str) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if origin == "null" {
        // file:// pages (Claude-Orchestrator web UI) — orchestrator API only.
        return path.starts_with("/api/v1/orchestrator/");
    }
    if origin_is_tauri_app(origin) {
        return true;
    }
    origin_is_loopback(origin)
}

/// AgentMirai's Tauri webview origin.
///
/// The bundled visualizer (`vista-mirai.html`) runs inside the desktop app's
/// webview and fetches this API with absolute URLs, so its origin has to be
/// trusted or every request is blocked by CORS. Tauri serves the app from
/// `tauri://localhost` (macOS/iOS/Linux) and `http://tauri.localhost`
/// (Windows) — neither passes `origin_is_loopback`, because the first is not
/// an http(s) scheme and the second's host is `tauri.localhost`, not
/// `localhost`. Until v0.7.0 the server ran `CorsLayer::permissive()` and this
/// worked by accident; tightening CORS silently broke the desktop app.
///
/// Security note: this is strictly NARROWER than the `null` allowance above.
/// A web page cannot forge a `tauri://` origin — browsers never emit one — so
/// only a Tauri app already running on this machine can present it, whereas
/// any sandboxed iframe can present `null`.
fn origin_is_tauri_app(origin: &str) -> bool {
    origin.eq_ignore_ascii_case("tauri://localhost")
        || origin.eq_ignore_ascii_case("http://tauri.localhost")
        || origin.eq_ignore_ascii_case("https://tauri.localhost")
}

/// `http(s)://localhost|127.x.x.x|[::1]` on any port.
fn origin_is_loopback(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Reject cross-site MUTATING requests from untrusted web origins.
///
/// CORS preflights stop cross-origin JSON POSTs, but CORS never stops
/// "simple" requests — a body-less POST (e.g.
/// `/api/v1/orchestrator/sessions/{id}/stop`) or a `text/plain` form post
/// EXECUTES server-side even though the browser hides the response. Browsers
/// always attach `Origin` to cross-origin (and most same-origin) POSTs, so:
/// - no `Origin` header (curl, SDKs, server-to-server webhooks) → pass;
/// - `Origin` trusted per [`origin_is_trusted`] → pass;
/// - `Origin` matching the request's own `Host` (same-origin UI served over
///   LAN, e.g. `http://192.168.x.x:3000/ui`) → pass;
/// - anything else → 403.
pub(crate) async fn cross_origin_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let Some(origin) = req.headers().get(header::ORIGIN) else {
        return next.run(req).await;
    };
    let trusted = origin_is_trusted(origin, req.uri().path())
        || match (origin.to_str(), req.headers().get(header::HOST)) {
            (Ok(o), Some(h)) => h.to_str().is_ok_and(|h| same_host_origin(o, h)),
            _ => false,
        };
    if trusted {
        return next.run(req).await;
    }
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": "cross-origin request rejected"})),
    )
        .into_response()
}

/// Does the `Origin` header point back at this same server (`Host` header)?
fn same_host_origin(origin: &str, host_header: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    let Some(ohost) = url.host_str() else {
        return false;
    };
    let Some(oport) = url.port_or_known_default() else {
        return false;
    };
    // Host header is `host` or `host:port` (IPv6 host in brackets).
    let (hhost, hport) = match host_header.rsplit_once(':') {
        Some((h, p))
            if !p.contains(']') && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (h, p.parse::<u16>().unwrap_or(0))
        }
        _ => (host_header, if url.scheme() == "https" { 443 } else { 80 }),
    };
    let hhost = hhost.trim_start_matches('[').trim_end_matches(']');
    let ohost = ohost.trim_start_matches('[').trim_end_matches(']');
    hhost.eq_ignore_ascii_case(ohost) && hport == oport
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
        Some(key) if api_key_matches(&key, expected) => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

/// Timing-safe API-key comparison.
///
/// `key == expected` short-circuits at the first differing byte, so response
/// latency leaks how many leading bytes matched — an attacker can recover the
/// key byte-by-byte. Comparing SHA-256 digests removes the signal: any change
/// in the guess scrambles the whole digest, so even if the digest comparison
/// itself short-circuits, its timing says nothing about the key bytes.
fn api_key_matches(provided: &str, expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    Sha256::digest(provided.as_bytes()) == Sha256::digest(expected.as_bytes())
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
    db_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if api_key.is_none() {
        if host_is_loopback(host) {
            tracing::warn!("No API key configured. Server is running without authentication.");
            tracing::warn!("Set MIRAI_API_KEY or use --api-key to enable authentication.");
        } else {
            // Non-loopback bind without auth: the agent-execute and orchestrator
            // endpoints amount to remote command execution for ANYONE who can
            // reach this port. Refuse to start — logging and continuing left the
            // door open while the CHANGELOG claimed this was a startup error.
            tracing::error!(
                "SECURITY: refusing to bind {host}:{port} WITHOUT an API key — \
                 /api/v1/agents/*/execute and /api/v1/orchestrator/* allow \
                 command execution and would be reachable by anyone on the network."
            );
            return Err(format!(
                "refusing to bind {host}:{port} without an API key: \
                 set MIRAI_API_KEY / --api-key, or bind locally with --host 127.0.0.1"
            )
            .into());
        }
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
            tracing::warn!(
                "--ui-dir {} is not a directory; /ui will 404",
                dir.display()
            );
        }
    }

    // 0.7.0: persistencia de runs en SQLite. Precedencia: --db-path >
    // MIRAI_DB_PATH > ~/.openmirai/engine.db. Si la DB no abre, el server
    // arranca igual (solo memoria) con warning — persistir no puede ser
    // motivo de no bootear.
    let resolved_db = db_path
        .or_else(|| std::env::var("MIRAI_DB_PATH").ok())
        .map(|p| expand_home(&p))
        .unwrap_or_else(|| expand_home("~/.openmirai/engine.db"));
    match crate::db::SqliteSessionRepo::open(&resolved_db) {
        Ok(repo) => {
            tracing::info!("runs persistence: {}", resolved_db.display());
            state.session_repo = Some(std::sync::Arc::new(repo));
        }
        Err(e) => {
            tracing::warn!(
                "runs persistence DISABLED — no se pudo abrir {}: {e}",
                resolved_db.display()
            );
        }
    }

    // PRD-013: load the persistent session registry, reconcile against real
    // tmux state, and start the ~2s status poll (no-op without active sessions).
    if let Err(e) = state.orchestrator.initialize().await {
        tracing::warn!("orchestrator: failed to load session registry: {e}");
    }
    // M6: sessions must report activity to THIS port.
    state.orchestrator.set_server_port(port);
    // M6 fix (code review): with --api-key the activity hook must authenticate
    // too — inject the key into spawned sessions (MIRAI_API_KEY) or every hook
    // POST dies with a silent 401 at the auth middleware.
    state.orchestrator.set_api_key(state.api_key.clone());
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

/// Is the bind host loopback-only? (`localhost`, `127.x.x.x`, `::1`.)
fn host_is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// Expand a leading `~` / `~/` to $HOME (M7 projects roots).
fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    if path == "~" || path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::Path::new(&home)
                .join(path.trim_start_matches("~/").trim_start_matches('~'));
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
