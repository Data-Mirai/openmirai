//! Mini-IDE backend (PRD-013) — `mirai edit <file>`.
//!
//! Serves a self-contained single-page editor (embedded in the binary) plus a
//! small `/api/edit/*` API for loading/saving the agent YAML, compiling
//! (parse + validate), and snapshot-based undo/redo. Reuses the existing
//! `/api/v1/*` routes (tool catalog, `agents/from-spec`, `execute`, `stream`)
//! for the palette, validation and the run/step-by-step test.
//!
//! Local-only: bound to loopback, single file, no auth. Cross-site requests
//! from web pages are rejected by the CORS allowlist + `cross_origin_guard`
//! in `super` (drive-by protection); the editor UI is same-origin so it is
//! unaffected.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::core::agent_spec::AgentSpec;
use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use super::state::{AppState, LLMFactory};

/// The embedded single-page editor (HTML + CSS + JS, no external deps).
const EDITOR_HTML: &str = include_str!("editor_index.html");

// ---------------------------------------------------------------------------
// Editor state (file + snapshot history)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct EditorState {
    inner: Arc<Mutex<EditorInner>>,
}

struct EditorInner {
    path: PathBuf,
    /// YAML snapshots; `cursor` points at the current one.
    history: Vec<String>,
    cursor: usize,
    snapshot_dir: PathBuf,
    /// Run config the editor launched with — drives the ▶ Run preflight gate.
    provider: String,
    model: String,
    ollama_host: String,
    has_key: bool,
}

impl EditorState {
    /// Initialise from the target file (blank if it doesn't exist).
    pub fn open(path: &str, provider: &str, model: &str, ollama_host: &str, has_key: bool) -> Self {
        let path = PathBuf::from(path);
        let initial = std::fs::read_to_string(&path).unwrap_or_default();
        let snapshot_dir = snapshot_dir_for(&path);
        EditorState {
            inner: Arc::new(Mutex::new(EditorInner {
                path,
                history: vec![initial],
                cursor: 0,
                snapshot_dir,
                provider: provider.to_string(),
                model: model.to_string(),
                ollama_host: ollama_host.to_string(),
                has_key,
            })),
        }
    }

    /// Snapshot of the launch run context: (provider, model, ollama_host, has_key).
    async fn run_ctx(&self) -> (String, String, String, bool) {
        let i = self.inner.lock().await;
        (
            i.provider.clone(),
            i.model.clone(),
            i.ollama_host.clone(),
            i.has_key,
        )
    }
}

/// `<dir>/.openmirai-history/<filestem>/`
fn snapshot_dir_for(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "agent".to_string());
    dir.join(".openmirai-history").join(stem)
}

// ---------------------------------------------------------------------------
// API payloads
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LoadResponse {
    path: String,
    raw: String,
    /// Parsed AgentSpec as JSON, or null if the YAML doesn't parse yet.
    spec: Value,
    /// Parse error message when `spec` is null.
    parse_error: Option<String>,
}

#[derive(Deserialize)]
struct SaveRequest {
    /// Either the structured spec (from the visual editor) or raw YAML text.
    spec: Option<Value>,
    raw: Option<String>,
}

#[derive(Serialize)]
struct SaveResponse {
    ok: bool,
    raw: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct Diagnostic {
    severity: String,
    location: String,
    message: String,
}

#[derive(Serialize)]
struct CompileResponse {
    ok: bool,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize)]
struct HistoryResponse {
    spec: Value,
    raw: String,
    can_undo: bool,
    can_redo: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse YAML → AgentSpec → canonical YAML + JSON. Returns (raw_canonical, json) or Err(msg).
fn spec_from_yaml(yaml: &str) -> Result<(String, Value), String> {
    let spec = AgentSpec::from_yaml(yaml).map_err(|e| e.to_string())?;
    canonical(&spec)
}

fn spec_from_json(value: Value) -> Result<(String, Value), String> {
    let spec: AgentSpec = serde_json::from_value(value).map_err(|e| e.to_string())?;
    canonical(&spec)
}

fn canonical(spec: &AgentSpec) -> Result<(String, Value), String> {
    // Serialize the YAML from the JSON Value (whose object keys are sorted via
    // serde_json's BTreeMap) so the output is DETERMINISTIC. Serializing the
    // spec directly would leak HashMap iteration order (e.g. position x/y),
    // breaking snapshot dedup and producing noisy diffs.
    let json = serde_json::to_value(spec).map_err(|e| e.to_string())?;
    let raw = serde_yaml::to_string(&json).map_err(|e| e.to_string())?;
    Ok((raw, json))
}

/// Convert a save request (spec OR raw) to canonical YAML + JSON.
fn canonical_from_request(req: &SaveRequest) -> Result<(String, Value), String> {
    if let Some(spec) = &req.spec {
        spec_from_json(spec.clone())
    } else if let Some(raw) = &req.raw {
        spec_from_yaml(raw)
    } else {
        Err("empty save request: expected `spec` or `raw`".to_string())
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn serve_spa() -> Html<&'static str> {
    Html(EDITOR_HTML)
}

async fn load_agent(State(editor): State<EditorState>) -> Json<LoadResponse> {
    let inner = editor.inner.lock().await;
    let raw = inner.history[inner.cursor].clone();
    let path = inner.path.to_string_lossy().to_string();

    if raw.trim().is_empty() {
        return Json(LoadResponse {
            path,
            raw,
            spec: Value::Null,
            parse_error: None, // blank → new agent
        });
    }

    match spec_from_yaml(&raw) {
        Ok((canonical_raw, spec)) => Json(LoadResponse {
            path,
            raw: canonical_raw,
            spec,
            parse_error: None,
        }),
        Err(e) => Json(LoadResponse {
            path,
            raw,
            spec: Value::Null,
            parse_error: Some(e),
        }),
    }
}

async fn save_agent(
    State(editor): State<EditorState>,
    Json(req): Json<SaveRequest>,
) -> Json<SaveResponse> {
    let (raw, _json) = match canonical_from_request(&req) {
        Ok(v) => v,
        Err(e) => {
            return Json(SaveResponse {
                ok: false,
                raw: String::new(),
                error: Some(e),
            })
        }
    };

    let mut inner = editor.inner.lock().await;

    // Write canonical YAML to the file (source of truth).
    if let Err(e) = std::fs::write(&inner.path, &raw) {
        return Json(SaveResponse {
            ok: false,
            raw,
            error: Some(format!("write failed: {e}")),
        });
    }

    push_snapshot(&mut inner, raw.clone());

    Json(SaveResponse {
        ok: true,
        raw,
        error: None,
    })
}

async fn compile_agent(Json(req): Json<SaveRequest>) -> Json<CompileResponse> {
    let yaml = match (&req.spec, &req.raw) {
        (Some(spec), _) => match serde_yaml::to_string(spec) {
            Ok(y) => y,
            Err(e) => return Json(diag_error("spec", &e.to_string())),
        },
        (None, Some(raw)) => raw.clone(),
        (None, None) => return Json(diag_error("input", "empty compile request")),
    };

    // Parse.
    let mut spec = match AgentSpec::from_yaml(&yaml) {
        Ok(s) => s,
        Err(e) => return Json(diag_error("yaml", &e.to_string())),
    };

    // Validate structure (refs, self-loops, cycles, empty graph, contracts).
    match spec.validate() {
        Ok(()) => Json(CompileResponse {
            ok: true,
            diagnostics: vec![],
        }),
        Err(e) => Json(diag_error("graph", &e.to_string())),
    }
}

fn diag_error(location: &str, message: &str) -> CompileResponse {
    CompileResponse {
        ok: false,
        diagnostics: vec![Diagnostic {
            severity: "error".to_string(),
            location: location.to_string(),
            message: message.to_string(),
        }],
    }
}

async fn undo(State(editor): State<EditorState>) -> Json<HistoryResponse> {
    let mut inner = editor.inner.lock().await;
    if inner.cursor > 0 {
        inner.cursor -= 1;
        restore_current(&inner);
    }
    history_response(&inner)
}

async fn redo(State(editor): State<EditorState>) -> Json<HistoryResponse> {
    let mut inner = editor.inner.lock().await;
    if inner.cursor + 1 < inner.history.len() {
        inner.cursor += 1;
        restore_current(&inner);
    }
    history_response(&inner)
}

// ---------------------------------------------------------------------------
// Snapshot internals
// ---------------------------------------------------------------------------

fn push_snapshot(inner: &mut EditorInner, raw: String) {
    // Truncate forward history (classic undo semantics).
    inner.history.truncate(inner.cursor + 1);
    if inner.history.last().map(|s| s.as_str()) == Some(raw.as_str()) {
        return; // no change → no new snapshot
    }
    inner.history.push(raw.clone());
    inner.cursor = inner.history.len() - 1;

    // Persist to sidecar (best-effort).
    let _ = std::fs::create_dir_all(&inner.snapshot_dir);
    let file = inner.snapshot_dir.join(format!("{:04}.yaml", inner.cursor));
    let _ = std::fs::write(file, &raw);
}

fn restore_current(inner: &EditorInner) {
    let raw = &inner.history[inner.cursor];
    let _ = std::fs::write(&inner.path, raw);
}

fn history_response(inner: &EditorInner) -> Json<HistoryResponse> {
    let raw = inner.history[inner.cursor].clone();
    let spec = spec_from_yaml(&raw).map(|(_, j)| j).unwrap_or(Value::Null);
    Json(HistoryResponse {
        spec,
        raw,
        can_undo: inner.cursor > 0,
        can_redo: inner.cursor + 1 < inner.history.len(),
    })
}

// ---------------------------------------------------------------------------
// Router + serve
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PRD-014: preflight gate + model catalog (editor)
// ---------------------------------------------------------------------------

/// Is the editor's provider/model ready to run? (mock is always ready.)
async fn preflight_handler(State(editor): State<EditorState>) -> Json<Value> {
    let (provider, model, host, has_key) = editor.run_ctx().await;
    if provider == "mock" {
        return Json(
            json!({ "ok": true, "provider": provider, "model": model, "diagnostics": [] }),
        );
    }
    let pf = crate::preflight::check(&provider, &model, &host, has_key).await;
    Json(serde_json::to_value(pf).unwrap_or_else(|_| json!({ "ok": false })))
}

/// Curated catalog ∪ installed (with `installed` flag + current provider/model).
async fn models_handler(State(editor): State<EditorState>) -> Json<Value> {
    let (provider, model, host, _) = editor.run_ctx().await;
    let installed = crate::preflight::installed_models(&host)
        .await
        .unwrap_or_default();
    let is_installed = |name: &str| {
        installed
            .iter()
            .any(|i| i == name || *i == format!("{name}:latest"))
    };
    let catalog: Vec<Value> = crate::catalog::CATALOG
        .iter()
        .map(|m| {
            json!({
                "name": m.name, "family": m.family, "size_gb": m.size_gb,
                "description": m.description, "recommended": m.recommended,
                "installed": is_installed(m.name),
            })
        })
        .collect();
    Json(
        json!({ "catalog": catalog, "installed": installed, "provider": provider, "model": model }),
    )
}

/// Pull an Ollama model, streaming progress as SSE (`data: {...}\n\n`).
async fn models_pull_handler(
    State(editor): State<EditorState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    use axum::body::{Body, Bytes};
    let host = { editor.inner.lock().await.ollama_host.clone() };
    let name = body
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, std::io::Error>>();
    tokio::spawn(async move {
        let tx2 = tx.clone();
        let res = crate::preflight::pull_model(&host, &name, move |status, completed, total| {
            let payload = json!({ "status": status, "completed": completed, "total": total });
            let _ = tx2.send(Ok(Bytes::from(format!("data: {payload}\n\n"))));
        })
        .await;
        let done = match res {
            Ok(()) => json!({ "status": "success", "done": true }),
            Err(e) => json!({ "status": "error", "error": e, "done": true }),
        };
        let _ = tx.send(Ok(Bytes::from(format!("data: {done}\n\n"))));
    });
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Read the persisted user config (`~/.openmirai/config.toml`).
async fn config_get() -> Json<Value> {
    let c = crate::config::UserConfig::load();
    Json(serde_json::to_value(c).unwrap_or_else(|_| json!({})))
}

/// Persist default provider/model (never stores API keys).
async fn config_put(Json(body): Json<Value>) -> Json<Value> {
    let mut c = crate::config::UserConfig::load();
    if let Some(p) = body.get("default_provider").and_then(|v| v.as_str()) {
        c.default_provider = p.to_string();
    }
    if let Some(m) = body.get("default_model").and_then(|v| v.as_str()) {
        c.default_model = m.to_string();
    }
    match c.save() {
        Ok(_) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

/// Editor routes (SPA + `/api/edit/*`), merged onto the standard API router.
pub fn create_editor_router(app_state: AppState, editor_state: EditorState) -> Router {
    let editor_routes = Router::new()
        .route("/", get(serve_spa))
        .route("/editor", get(serve_spa))
        .route("/api/edit/agent", get(load_agent).put(save_agent))
        .route("/api/edit/compile", post(compile_agent))
        .route("/api/edit/undo", post(undo))
        .route("/api/edit/redo", post(redo))
        .route("/api/edit/preflight", get(preflight_handler))
        .route("/api/edit/models", get(models_handler))
        .route("/api/edit/models/pull", post(models_pull_handler))
        .route("/api/edit/config", get(config_get).put(config_put))
        .with_state(editor_state);

    // Security gate 0.7.0: wrap the WHOLE merged router with the cross-site
    // guard. The `/api/v1/*` routes already carry it inside create_router, but
    // the editor routes merged here would otherwise miss it — and body-less
    // POSTs like /api/edit/undo are "simple" cross-origin requests that
    // execute (write the YAML file!) without any CORS preflight. The editor
    // UI itself is same-origin on loopback, so it always passes the guard.
    super::create_router(app_state)
        .merge(editor_routes)
        .layer(axum::middleware::from_fn(super::cross_origin_guard))
}

/// Start the editor server (loopback, no auth). The CLI opens the browser.
/// Run context validated by the editor's ▶ Run preflight gate (the provider /
/// model the editor was launched with).
pub struct RunCtx {
    pub provider: String,
    pub model: String,
    pub ollama_host: String,
    pub has_key: bool,
}

pub async fn serve_editor(
    host: &str,
    port: u16,
    llm_factory: LLMFactory,
    file_path: &str,
    run: RunCtx,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let app_state = AppState::new(registry, llm_factory, None);
    let editor_state = EditorState::open(
        file_path,
        &run.provider,
        &run.model,
        &run.ollama_host,
        run.has_key,
    );

    let app = create_editor_router(app_state, editor_state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("OpenMirai editor on http://{addr} (file: {file_path})");
    axum::serve(listener, app)
        .with_graceful_shutdown(super::shutdown_signal())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
name: test-agent
version: v1
graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: out
      tool_type: output/response
  edges:
    - source: start
      target: out
"#;

    #[test]
    fn spec_roundtrip_yaml_to_json_to_canonical() {
        let (raw, json) = spec_from_yaml(SAMPLE).expect("parse");
        assert!(raw.contains("name: test-agent"));
        assert_eq!(json["name"], "test-agent");
        // round-trip: json back to canonical equals canonical
        let (raw2, _) = spec_from_json(json).expect("from json");
        assert_eq!(raw, raw2);
    }

    #[test]
    fn compile_reports_invalid_graph() {
        // Edge to a non-existent node. `AgentSpec::from_yaml` validates on parse,
        // so the editor's compile path (spec_from_yaml) rejects it.
        let bad = r#"
name: bad
version: v1
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
  edges:
    - source: a
      target: ghost
"#;
        assert!(
            spec_from_yaml(bad).is_err(),
            "edge to ghost node must be rejected"
        );
    }

    #[test]
    fn snapshot_dir_uses_filestem() {
        let d = snapshot_dir_for(Path::new("/tmp/foo/my-agent.yaml"));
        assert!(d.ends_with(".openmirai-history/my-agent"));
    }

    #[test]
    fn undo_redo_cursor_moves() {
        let mut inner = EditorInner {
            path: PathBuf::from("/tmp/x.yaml"),
            history: vec!["a".to_string()],
            cursor: 0,
            snapshot_dir: PathBuf::from("/tmp/.openmirai-history/x"),
            provider: "mock".to_string(),
            model: String::new(),
            ollama_host: "http://localhost:11434".to_string(),
            has_key: false,
        };
        push_snapshot(&mut inner, "b".to_string());
        assert_eq!(inner.cursor, 1);
        // dedup: pushing same content does not grow history
        push_snapshot(&mut inner, "b".to_string());
        assert_eq!(inner.history.len(), 2);
    }
}
