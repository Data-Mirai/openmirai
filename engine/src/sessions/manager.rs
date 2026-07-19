//! SessionManager — create / list / direct / stop orchestrated sessions,
//! with a persistent JSON registry reconciled against real tmux state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};

use crate::core::events::{EventEmitter, EventType};
use crate::utils::short_id;

use super::backend::TMUX_SESSION_PREFIX;
use super::status::detect_status;
use super::{SessionBackend, SessionError, SessionStatus};

/// How many pane lines are captured for status detection / output events.
const POLL_CAPTURE_LINES: usize = 40;

/// Poll interval while there are active sessions.
pub const POLL_INTERVAL_SECS: u64 = 2;

// ---------------------------------------------------------------------------
// SessionRecord
// ---------------------------------------------------------------------------

/// One orchestrated session — the persisted registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub project_dir: String,
    pub objective: String,
    pub status: SessionStatus,
    /// Model id passed as `claude --model <id>`; `None` = the CLI's default.
    pub model: Option<String>,
    /// Effort level passed as `claude --effort <level>` (the flag exists in
    /// the Claude Code CLI); `None` = the CLI's default.
    pub effort: Option<String>,
    /// When true, the word "ultracode" is prepended to the FIRST prompt sent.
    pub ultracode: bool,
    pub tmux_session: String,
    /// RFC-3339 timestamps.
    pub created_at: String,
    pub last_activity: String,
    /// Internal: whether the first prompt was already sent (ultracode gate).
    #[serde(default)]
    pub first_prompt_sent: bool,
    /// Session that spawned this one (M6: orchestrator→child edges). Nullable.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// EXTERNAL node (bridge): registered by an external substrate (e.g. the
    /// Claude Code FleetView subagents of a chat session) rather than spawned
    /// by the engine over tmux. External nodes have NO tmux session behind them
    /// (`tmux_session` is empty), so they must be skipped by the tmux
    /// reconciliation on startup and by the status poll — otherwise the engine
    /// would immediately mark them `stopped` (their tmux session "doesn't
    /// exist"). They still appear in the registry, `GET /sessions` and the SSE
    /// stream exactly like normal sessions, so the UI renders them unchanged.
    /// Internal bookkeeping — excluded from `to_api_json` (the UI shape stays
    /// pinned to the PRD-013 contract).
    #[serde(default)]
    pub external: bool,
}

impl SessionRecord {
    /// The exact wire shape pinned in the PRD-013 contract (internal
    /// bookkeeping fields like `first_prompt_sent` are excluded).
    pub fn to_api_json(&self) -> serde_json::Value {
        json!({
            "id": self.id,
            "name": self.name,
            "project_dir": self.project_dir,
            "objective": self.objective,
            "status": self.status,
            "model": self.model,
            "effort": self.effort,
            "ultracode": self.ultracode,
            "tmux_session": self.tmux_session,
            "created_at": self.created_at,
            "last_activity": self.last_activity,
            "parent_id": self.parent_id,
        })
    }
}

/// Parameters to spawn a new orchestrated session.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpawnParams {
    #[serde(default)]
    pub name: Option<String>,
    pub project_dir: String,
    pub objective: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub ultracode: bool,
    /// Parent session id (M6). The CLI fills it from MIRAI_SESSION_ID.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Opt out of the activity-reporting Claude Code hooks (M6).
    #[serde(default)]
    pub no_hooks: bool,
    /// M7: create the project_dir (`mkdir -p`) before spawning. When false
    /// and the dir does not exist, spawn fails with a clear error (before
    /// this, tmux silently accepted a bad `-c` and claude started in the
    /// wrong directory).
    #[serde(default)]
    pub create_dir: bool,
    /// MCP wiring: when true, the engine materializes a per-session
    /// `<id>-mcp.json` (default catalog for now — see `hooks::default_mcp_servers`)
    /// and appends `--mcp-config <file> --strict-mcp-config` to the `claude`
    /// command, so the worker starts with ONLY those MCP servers. Off by
    /// default (opt-in) so existing behavior is unchanged until a universe
    /// enables it. Future: the catalog is compiled per-universe.
    #[serde(default)]
    pub mcp: bool,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// SessionManager
// ---------------------------------------------------------------------------

/// Orchestrates Claude Code sessions over a [`SessionBackend`].
///
/// The registry is persisted as pretty JSON at `registry_path` on every
/// mutation, so the live directory survives `mirai serve` restarts. Call
/// [`SessionManager::initialize`] once at startup to load it and reconcile
/// against real tmux state (registry entries whose tmux session no longer
/// exists are marked `stopped`).
pub struct SessionManager {
    backend: Arc<dyn SessionBackend>,
    registry_path: PathBuf,
    sessions: RwLock<HashMap<String, SessionRecord>>,
    events: EventEmitter,
    /// Last captured pane per session, to diff NEW lines for session_output.
    last_pane: Mutex<HashMap<String, Vec<String>>>,
    polling_started: AtomicBool,
    /// Port `mirai serve` listens on — injected as MIRAI_PORT at spawn so the
    /// activity hook knows where to report. Set by `serve()`.
    server_port: std::sync::atomic::AtomicU16,
    /// API key of `mirai serve` (if auth is on) — injected as MIRAI_API_KEY at
    /// spawn so the activity hook can authenticate (it sends X-API-Key when the
    /// var exists; without it the POST dies with a silent 401). Set by `serve()`.
    /// std Mutex (not tokio): only short lock in setters/spawn, never across await.
    api_key: std::sync::Mutex<Option<String>>,
    /// Recent resource activity per session (M6). Ring buffer, in-memory only.
    activity: Mutex<HashMap<String, std::collections::VecDeque<serde_json::Value>>>,
}

/// Max entries kept per session in the activity ring buffer.
const ACTIVITY_RING_MAX: usize = 200;

impl SessionManager {
    pub fn new(backend: Arc<dyn SessionBackend>, registry_path: PathBuf) -> Self {
        Self {
            backend,
            registry_path,
            sessions: RwLock::new(HashMap::new()),
            events: EventEmitter::new(256),
            last_pane: Mutex::new(HashMap::new()),
            polling_started: AtomicBool::new(false),
            server_port: std::sync::atomic::AtomicU16::new(3000),
            api_key: std::sync::Mutex::new(None),
            activity: Mutex::new(HashMap::new()),
        }
    }

    /// Record the real server port (used for the MIRAI_PORT env injection).
    pub fn set_server_port(&self, port: u16) {
        self.server_port.store(port, Ordering::SeqCst);
    }

    /// Record the server's API key (used for the MIRAI_API_KEY env injection,
    /// so the M6 activity hook can pass the auth middleware). `None` = no auth.
    pub fn set_api_key(&self, key: Option<String>) {
        *self.api_key.lock().expect("api_key mutex poisoned") = key;
    }

    /// Base dir for hook artifacts: the registry's directory (`~/.openmirai`).
    fn hooks_base(&self) -> PathBuf {
        self.registry_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Default registry location, following the engine's `~/.openmirai/`
    /// convention (same directory as `config.toml`).
    pub fn default_registry_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Path::new(&home)
            .join(".openmirai")
            .join("orchestrator_sessions.json")
    }

    /// Subscribe to orchestrator events (session_created, session_status_changed,
    /// session_output, session_stopped).
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::core::events::ExecutionEvent> {
        self.events.subscribe()
    }

    // -- persistence --------------------------------------------------------

    /// Load the registry from disk and reconcile against real tmux state.
    pub async fn initialize(&self) -> Result<(), SessionError> {
        let loaded: Vec<SessionRecord> = match std::fs::read_to_string(&self.registry_path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| SessionError::Backend(format!("corrupt registry: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };

        {
            let mut sessions = self.sessions.write().await;
            for mut rec in loaded {
                // Reconcile: active in registry but gone in tmux → stopped.
                // EXTERNAL nodes have no tmux session behind them (they live on
                // another substrate), so the tmux check would wrongly stop them
                // — skip reconciliation for them. They're ephemeral: if a
                // restart loses them, the external substrate re-registers.
                if !rec.external
                    && rec.status.is_active()
                    && !self.backend.session_exists(&rec.tmux_session)
                {
                    rec.status = SessionStatus::Stopped;
                }
                sessions.insert(rec.id.clone(), rec);
            }
        }
        self.persist().await
    }

    /// Write the registry to disk (sorted by created_at for stable diffs).
    async fn persist(&self) -> Result<(), SessionError> {
        let sessions = self.sessions.read().await;
        let mut list: Vec<&SessionRecord> = sessions.values().collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let raw = serde_json::to_string_pretty(&list)
            .map_err(|e| SessionError::Backend(format!("serialize registry: {e}")))?;
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.registry_path, raw)?;
        Ok(())
    }

    // -- operations ----------------------------------------------------------

    /// Spawn a new session: `tmux new-session -d -s mirai-<id> -c <dir> "claude …"`.
    pub async fn spawn(&self, params: SpawnParams) -> Result<SessionRecord, SessionError> {
        if params.project_dir.trim().is_empty() {
            return Err(SessionError::Invalid("project_dir is required".into()));
        }
        if params.objective.trim().is_empty() {
            return Err(SessionError::Invalid("objective is required".into()));
        }

        // M7: the project dir must exist (tmux silently accepts a bad `-c`,
        // leaving claude in the wrong directory). `create_dir` opts into
        // creating it.
        let project_path = Path::new(params.project_dir.trim());
        if params.create_dir {
            std::fs::create_dir_all(project_path).map_err(|e| {
                SessionError::Invalid(format!(
                    "could not create project_dir '{}': {e}",
                    params.project_dir
                ))
            })?;
        } else if !project_path.is_dir() {
            return Err(SessionError::Invalid(format!(
                "project_dir '{}' does not exist (pass create_dir: true to create it)",
                params.project_dir
            )));
        }

        // Unknown parent → normalized to None (contract: never reject on it).
        let parent_id = match params.parent_id {
            Some(p) if self.sessions.read().await.contains_key(&p) => Some(p),
            _ => None,
        };

        let id = short_id();
        let tmux_session = format!("{TMUX_SESSION_PREFIX}{id}");

        // SEGURIDAD (anti command-injection): `command` es el shell-command que tmux
        // ejecuta vía `/bin/sh -c` (ver backend.rs). `model`/`effort` llegan del body
        // del request y se interpolan crudos → RCE. Allowlist estricta: solo letras
        // ASCII, dígitos y `.`/`_`/`-`; se rechazan espacios y metacaracteres de shell
        // (`;`, `$`, backtick, `|`, `&`, comillas, etc.).
        fn is_safe_arg(s: &str) -> bool {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        }
        if let Some(model) = &params.model {
            if !is_safe_arg(model) {
                return Err(SessionError::Invalid(format!(
                    "model contiene caracteres no permitidos: '{model}'"
                )));
            }
        }
        if let Some(effort) = &params.effort {
            if !is_safe_arg(effort) {
                return Err(SessionError::Invalid(format!(
                    "effort contiene caracteres no permitidos: '{effort}'"
                )));
            }
        }

        let mut command = String::from("claude");
        if let Some(model) = &params.model {
            command.push_str(&format!(" --model {model}"));
        }
        if let Some(effort) = &params.effort {
            command.push_str(&format!(" --effort {effort}"));
        }

        // M6: register the activity-reporting hook unless opted out. Failure
        // to write the hook files must not block the spawn — log and continue.
        if !params.no_hooks {
            match super::hooks::write_session_settings(&self.hooks_base(), &id) {
                Ok(settings) => {
                    command.push_str(&format!(" --settings '{}'", settings.display()));
                }
                Err(e) => {
                    tracing::warn!("orchestrator: could not write hook settings: {e}");
                }
            }
        }

        // MCP wiring (opt-in): materialize the per-session MCP config and pass
        // `--mcp-config <file> --strict-mcp-config` so the worker uses ONLY the
        // servers we compiled (strong per-universe isolation). Same
        // per-session-artifact + append-flag pattern as `--settings`. The path
        // is engine-controlled (never from the request body), so no injection
        // surface. Failure to write must not block the spawn — log and continue
        // without MCP.
        if params.mcp {
            let servers = super::hooks::default_mcp_servers();
            match super::hooks::write_session_mcp_config(&self.hooks_base(), &id, &servers) {
                Ok(mcp_cfg) => {
                    command.push_str(&format!(
                        " --mcp-config '{}' --strict-mcp-config",
                        mcp_cfg.display()
                    ));
                }
                Err(e) => {
                    tracing::warn!("orchestrator: could not write mcp config: {e}");
                }
            }
        }

        // M6: the session (and its hook subprocesses) must know who it is and
        // where the engine listens.
        let mut env = vec![
            ("MIRAI_SESSION_ID".to_string(), id.clone()),
            (
                "MIRAI_PORT".to_string(),
                self.server_port.load(Ordering::SeqCst).to_string(),
            ),
        ];
        // M6 fix (code review): when the server runs with --api-key, the
        // activity hook must authenticate too — without MIRAI_API_KEY in the
        // session env the hook's POST hits the auth middleware and dies with
        // a silent 401 (the hook never fails loudly by design).
        if let Some(key) = self.api_key.lock().expect("api_key mutex poisoned").clone() {
            env.push(("MIRAI_API_KEY".to_string(), key));
        }

        self.backend
            .spawn(&tmux_session, &params.project_dir, &command, &env)?;

        let now = now_rfc3339();
        let record = SessionRecord {
            id: id.clone(),
            name: params.name.unwrap_or_else(|| tmux_session.clone()),
            project_dir: params.project_dir,
            objective: params.objective,
            status: SessionStatus::Starting,
            model: params.model,
            effort: params.effort,
            ultracode: params.ultracode,
            tmux_session,
            created_at: now.clone(),
            last_activity: now,
            first_prompt_sent: false,
            parent_id,
            external: false,
        };

        self.sessions
            .write()
            .await
            .insert(id.clone(), record.clone());
        if let Err(e) = self.persist().await {
            // Code review: if the registry write fails right after spawning,
            // don't leak an orphan tmux session that no registry entry tracks.
            // Best-effort kill + rollback of the in-memory entry, then bubble
            // up the original persist error.
            let _ = self.backend.kill(&record.tmux_session);
            self.sessions.write().await.remove(&id);
            return Err(e);
        }

        self.emit(EventType::SessionCreated, &id, record.to_api_json());
        Ok(record)
    }

    /// All sessions, newest first.
    pub async fn list(&self) -> Vec<SessionRecord> {
        let sessions = self.sessions.read().await;
        let mut list: Vec<SessionRecord> = sessions.values().cloned().collect();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        list
    }

    pub async fn get(&self, id: &str) -> Option<SessionRecord> {
        self.sessions.read().await.get(id).cloned()
    }

    /// Send a prompt to the session (tmux send-keys + Enter).
    ///
    /// With `ultracode: true`, the word "ultracode" is prepended to the FIRST
    /// prompt only.
    pub async fn send(&self, id: &str, text: &str) -> Result<(), SessionError> {
        // Code review: payload computation AND the first_prompt_sent flip
        // happen under ONE write lock (send_text included), so two concurrent
        // send()s on the first prompt can't both see the flag unset and each
        // prepend "ultracode". On backend error the flag stays unset (the `?`
        // returns before flipping it), preserving the previous semantics.
        {
            let mut sessions = self.sessions.write().await;
            let rec = sessions
                .get_mut(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
            if !rec.status.is_active() {
                return Err(SessionError::Stopped(id.to_string()));
            }
            let payload = if rec.ultracode && !rec.first_prompt_sent {
                format!("ultracode {text}")
            } else {
                text.to_string()
            };

            self.backend.send_text(&rec.tmux_session, &payload)?;

            rec.first_prompt_sent = true;
            rec.last_activity = now_rfc3339();
        }
        self.persist().await
    }

    /// Capture the last `lines` lines of the session's pane.
    pub async fn output(&self, id: &str, lines: usize) -> Result<Vec<String>, SessionError> {
        let rec = self
            .get(id)
            .await
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
        if !rec.status.is_active() {
            return Ok(Vec::new());
        }
        self.backend.capture_output(&rec.tmux_session, lines)
    }

    /// Kill the tmux session and mark the registry entry `stopped`.
    pub async fn stop(&self, id: &str) -> Result<SessionRecord, SessionError> {
        let rec = self
            .get(id)
            .await
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;

        // Best-effort kill: the tmux session may already be gone.
        if self.backend.session_exists(&rec.tmux_session) {
            self.backend.kill(&rec.tmux_session)?;
        }

        let updated = {
            let mut sessions = self.sessions.write().await;
            let rec = sessions
                .get_mut(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
            rec.status = SessionStatus::Stopped;
            rec.last_activity = now_rfc3339();
            rec.clone()
        };
        self.persist().await?;

        // Contract shape (PRD-013 UI): session_stopped → {id}.
        self.emit(EventType::SessionStopped, id, json!({ "id": id }));
        Ok(updated)
    }

    // -- external nodes / bridge ---------------------------------------------

    /// Register an EXTERNAL node — a session running on another substrate (the
    /// Claude Code FleetView subagents of a chat session) that the engine did
    /// NOT spawn over tmux. It appears in the registry, `GET /sessions` and the
    /// SSE stream exactly like a normal session so the visualizer renders it
    /// with no UI change, but there is no tmux behind it: `tmux_session` is
    /// empty and it's skipped by the tmux reconciliation and the status poll.
    ///
    /// Emits `session_created` with the full node record (contract shape).
    /// `id` may be provided (idempotent re-register updates the existing node);
    /// otherwise a short id is minted like the tmux sessions.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_external(
        &self,
        id: Option<String>,
        name: Option<String>,
        objective: String,
        parent_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        project_dir: Option<String>,
        status: Option<SessionStatus>,
    ) -> Result<SessionRecord, SessionError> {
        if objective.trim().is_empty() {
            return Err(SessionError::Invalid("objective is required".into()));
        }

        let id = id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(short_id);

        // Unknown parent → normalized to None (same contract as spawn: never
        // reject on a stale/unknown parent). A node may point at another
        // external node or at a real tmux session.
        let parent_id = match parent_id {
            Some(p) if self.sessions.read().await.contains_key(&p) => Some(p),
            _ => None,
        };

        let now = now_rfc3339();
        let record = SessionRecord {
            id: id.clone(),
            name: name.unwrap_or_else(|| id.clone()),
            project_dir: project_dir.unwrap_or_default(),
            objective,
            status: status.unwrap_or(SessionStatus::Working),
            model,
            effort,
            ultracode: false,
            // No tmux behind an external node.
            tmux_session: String::new(),
            created_at: now.clone(),
            last_activity: now,
            first_prompt_sent: false,
            parent_id,
            external: true,
        };

        self.sessions
            .write()
            .await
            .insert(id.clone(), record.clone());
        self.persist().await?;

        self.emit(EventType::SessionCreated, &id, record.to_api_json());
        Ok(record)
    }

    /// Update an external node's status (bridge). Emits `session_status_changed`
    /// → `{id, status, last_activity}`. When `activity` is provided it ALSO
    /// records a resource-activity entry and emits `session_activity` (a free
    /// text label under the `tool` field, `action: "exec"`), so the visualizer
    /// can show what the node is doing. Works on external nodes only — a
    /// tmux-backed session is driven by the poll loop, not this endpoint.
    pub async fn set_external_status(
        &self,
        id: &str,
        status: SessionStatus,
        activity: Option<&str>,
    ) -> Result<SessionRecord, SessionError> {
        let last_activity = now_rfc3339();
        let updated = {
            let mut sessions = self.sessions.write().await;
            let rec = sessions
                .get_mut(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
            if !rec.external {
                return Err(SessionError::Invalid(format!(
                    "session '{id}' is not an external node; use /stop for tmux sessions"
                )));
            }
            rec.status = status;
            rec.last_activity = last_activity.clone();
            rec.clone()
        };
        self.persist().await?;

        self.emit(
            EventType::SessionStatusChanged,
            id,
            json!({
                "id": id,
                "status": status,
                "last_activity": last_activity,
            }),
        );

        // Optional free-text activity label → session_activity, so the graph
        // can show the node's current step (mirrors the M6 activity shape).
        if let Some(label) = activity {
            let label = label.trim();
            if !label.is_empty() {
                let entry = json!({
                    "id": id,
                    "tool": label,
                    "action": "exec",
                    "path": serde_json::Value::Null,
                    "ts": last_activity,
                });
                {
                    let mut act = self.activity.lock().await;
                    let ring = act.entry(id.to_string()).or_default();
                    ring.push_back(entry.clone());
                    while ring.len() > ACTIVITY_RING_MAX {
                        ring.pop_front();
                    }
                }
                self.emit(EventType::SessionActivity, id, entry);
            }
        }

        Ok(updated)
    }

    /// Unregister an external node (bridge): mark it `stopped` and emit
    /// `session_stopped` → `{id}`. Only valid for external nodes.
    pub async fn unregister_external(&self, id: &str) -> Result<SessionRecord, SessionError> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let rec = sessions
                .get_mut(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
            if !rec.external {
                return Err(SessionError::Invalid(format!(
                    "session '{id}' is not an external node; use /stop for tmux sessions"
                )));
            }
            rec.status = SessionStatus::Stopped;
            rec.last_activity = now_rfc3339();
            rec.clone()
        };
        self.persist().await?;

        self.emit(EventType::SessionStopped, id, json!({ "id": id }));
        Ok(updated)
    }

    // -- resource activity (M6) ----------------------------------------------

    /// Record a resource-activity event reported by a Claude Code hook and
    /// emit `session_activity` → `{id, tool, action, path, ts}`.
    ///
    /// Kept in an in-memory ring buffer (~200 entries per session) — activity
    /// is ephemeral visualization data, it does NOT persist to the registry.
    pub async fn record_activity(
        &self,
        id: &str,
        tool: &str,
        action: &str,
        path: Option<&str>,
    ) -> Result<serde_json::Value, SessionError> {
        if !matches!(action, "read" | "write" | "exec" | "net") {
            return Err(SessionError::Invalid(format!(
                "action must be read|write|exec|net, got '{action}'"
            )));
        }
        if tool.trim().is_empty() {
            return Err(SessionError::Invalid("tool is required".into()));
        }
        if self.get(id).await.is_none() {
            return Err(SessionError::NotFound(id.to_string()));
        }

        let entry = json!({
            "id": id,
            "tool": tool,
            "action": action,
            "path": path,
            "ts": now_rfc3339(),
        });

        {
            let mut activity = self.activity.lock().await;
            let ring = activity.entry(id.to_string()).or_default();
            ring.push_back(entry.clone());
            while ring.len() > ACTIVITY_RING_MAX {
                ring.pop_front();
            }
        }

        self.emit(EventType::SessionActivity, id, entry.clone());
        Ok(entry)
    }

    /// Recent activity for a session: the last `limit` entries in
    /// CHRONOLOGICAL order (contract fixed by the M6 web UI).
    pub async fn get_activity(&self, id: &str, limit: usize) -> Result<Vec<serde_json::Value>, SessionError> {
        if self.get(id).await.is_none() {
            return Err(SessionError::NotFound(id.to_string()));
        }
        let activity = self.activity.lock().await;
        Ok(activity
            .get(id)
            .map(|ring| {
                let skip = ring.len().saturating_sub(limit);
                ring.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default())
    }

    // -- projects (M7) ---------------------------------------------------------

    /// Union of known projects (contract pineado M7):
    /// (a) distinct `project_dir`s of registry sessions (`source: "session"`),
    /// (b) first-level subdirectories of the configured scan roots
    ///     (`source: "scan"`) — hidden dirs, `node_modules` and non-dirs
    ///     excluded.
    /// Dedup by path (session wins). Order: session entries first, then scan,
    /// each group alphabetical by path.
    pub async fn list_projects(&self, scan_roots: &[PathBuf]) -> Vec<serde_json::Value> {
        let mut seen = std::collections::HashSet::new();
        let mut session_entries: Vec<(String, serde_json::Value)> = Vec::new();
        let mut scan_entries: Vec<(String, serde_json::Value)> = Vec::new();

        let entry = |path: &str, source: &str| -> serde_json::Value {
            let p = Path::new(path);
            json!({
                "path": path,
                "name": p.file_name().and_then(|n| n.to_str()).unwrap_or(path),
                "source": source,
                "exists": p.is_dir(),
            })
        };

        for rec in self.sessions.read().await.values() {
            let dir = rec.project_dir.trim_end_matches('/').to_string();
            if !dir.is_empty() && seen.insert(dir.clone()) {
                session_entries.push((dir.clone(), entry(&dir, "session")));
            }
        }

        for root in scan_roots {
            let Ok(read) = std::fs::read_dir(root) else {
                continue;
            };
            for item in read.flatten() {
                let name = item.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "node_modules" {
                    continue;
                }
                if !item.path().is_dir() {
                    continue;
                }
                let dir = item.path().to_string_lossy().trim_end_matches('/').to_string();
                if seen.insert(dir.clone()) {
                    scan_entries.push((dir.clone(), entry(&dir, "scan")));
                }
            }
        }

        session_entries.sort_by(|a, b| a.0.cmp(&b.0));
        scan_entries.sort_by(|a, b| a.0.cmp(&b.0));
        session_entries
            .into_iter()
            .chain(scan_entries)
            .map(|(_, v)| v)
            .collect()
    }

    // -- status polling ------------------------------------------------------

    /// Number of sessions whose tmux session should still be alive.
    /// External nodes are excluded — they aren't backed by tmux, so they must
    /// not by themselves keep the ~2s poll loop awake.
    pub async fn active_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|r| r.status.is_active() && !r.external)
            .count()
    }

    /// One polling pass over all active sessions: reconcile deaths, detect
    /// status transitions from pane content, and emit output events.
    ///
    /// Public so tests (and the background loop) can drive it directly.
    pub async fn poll_once(&self) {
        let active: Vec<(String, String, SessionStatus)> = {
            let sessions = self.sessions.read().await;
            sessions
                .values()
                // Skip EXTERNAL nodes: they have no tmux session behind them,
                // so `session_exists` would fail and the poll would mark them
                // stopped. Their status is driven by the bridge via
                // `set_external_status` / `unregister_external`, not by polling.
                .filter(|r| r.status.is_active() && !r.external)
                .map(|r| (r.id.clone(), r.tmux_session.clone(), r.status))
                .collect()
        };
        if active.is_empty() {
            return;
        }

        let mut dirty = false;
        for (id, tmux_session, current) in active {
            // Died outside of us (claude exited, manual kill) → stopped.
            if !self.backend.session_exists(&tmux_session) {
                let last_activity = now_rfc3339();
                {
                    let mut sessions = self.sessions.write().await;
                    if let Some(rec) = sessions.get_mut(&id) {
                        rec.status = SessionStatus::Stopped;
                        rec.last_activity = last_activity.clone();
                    }
                }
                dirty = true;
                self.emit(
                    EventType::SessionStatusChanged,
                    &id,
                    json!({
                        "id": id,
                        "status": SessionStatus::Stopped,
                        "last_activity": last_activity,
                    }),
                );
                self.emit(EventType::SessionStopped, &id, json!({ "id": id }));
                continue;
            }

            let Ok(pane) = self.backend.capture_output(&tmux_session, POLL_CAPTURE_LINES) else {
                continue;
            };

            // Status transition from UI heuristics.
            if let Some(new_status) = detect_status(&pane) {
                if new_status != current {
                    let last_activity = now_rfc3339();
                    {
                        let mut sessions = self.sessions.write().await;
                        if let Some(rec) = sessions.get_mut(&id) {
                            rec.status = new_status;
                            rec.last_activity = last_activity.clone();
                        }
                    }
                    dirty = true;
                    self.emit(
                        EventType::SessionStatusChanged,
                        &id,
                        json!({
                            "id": id,
                            "status": new_status,
                            "last_activity": last_activity,
                        }),
                    );
                }
            }

            // NEW output lines only (contract: session_output → {id, lines}).
            let new_lines = {
                let mut last = self.last_pane.lock().await;
                let previous = last.insert(id.clone(), pane.clone()).unwrap_or_default();
                diff_new_lines(&previous, &pane)
            };
            if !new_lines.is_empty() {
                self.emit(
                    EventType::SessionOutput,
                    &id,
                    json!({ "id": id, "lines": new_lines }),
                );
            }
        }

        if dirty {
            let _ = self.persist().await;
        }
    }

    /// Start the background poll loop (~2s). Idempotent; the loop is a no-op
    /// while there are no active sessions.
    pub fn start_polling(self: &Arc<Self>) {
        if self.polling_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if manager.active_count().await > 0 {
                    manager.poll_once().await;
                }
            }
        });
    }

    fn emit(&self, event_type: EventType, session_id: &str, data: serde_json::Value) {
        let map: HashMap<String, serde_json::Value> = match data {
            serde_json::Value::Object(m) => m.into_iter().collect(),
            other => HashMap::from([("data".to_string(), other)]),
        };
        self.events
            .emit(event_type, session_id.to_string(), None, map);
    }
}

/// Compute the NEW lines between two consecutive pane captures.
///
/// Two situations to cover:
/// - **Scroll**: the new capture starts with a suffix of the previous one
///   (old lines scrolled off the top) — new lines are what follows the
///   overlap.
/// - **In-place rewrite**: the last line(s) changed (spinner, streaming
///   token) — new lines are what follows the common prefix.
///
/// We take whichever interpretation explains more of the new capture
/// (largest overlap), so only genuinely new/changed lines are emitted.
fn diff_new_lines(previous: &[String], current: &[String]) -> Vec<String> {
    if previous == current {
        return Vec::new();
    }

    // Longest suffix of `previous` that is a prefix of `current` (scroll).
    let max_overlap = previous.len().min(current.len());
    let mut overlap = 0;
    for k in (1..=max_overlap).rev() {
        if previous[previous.len() - k..] == current[..k] {
            overlap = k;
            break;
        }
    }

    // Longest common prefix (in-place rewrite of the tail).
    let common_prefix = previous
        .iter()
        .zip(current.iter())
        .take_while(|(a, b)| a == b)
        .count();

    current[overlap.max(common_prefix)..].to_vec()
}


// ---------------------------------------------------------------------------
// Tests (fake backend — no tmux required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::FakeBackend;

    fn temp_registry() -> PathBuf {
        std::env::temp_dir().join(format!("mirai-sessions-test-{}.json", short_id()))
    }

    fn manager_with_fake() -> (Arc<SessionManager>, Arc<FakeBackend>, PathBuf) {
        let backend = Arc::new(FakeBackend::new());
        let path = temp_registry();
        let manager = Arc::new(SessionManager::new(backend.clone(), path.clone()));
        (manager, backend, path)
    }

    fn spawn_params() -> SpawnParams {
        SpawnParams {
            name: Some("test".into()),
            project_dir: "/tmp".into(),
            objective: "do the thing".into(),
            model: Some("claude-opus-4-8".into()),
            effort: Some("high".into()),
            ultracode: false,
            parent_id: None,
            // Most tests assert the bare command; hook wiring has its own tests.
            no_hooks: true,
            create_dir: false,
            // MCP is opt-in; its own test flips this on.
            mcp: false,
        }
    }

    #[tokio::test]
    async fn spawn_creates_tmux_session_with_model_and_effort() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        assert_eq!(rec.status, SessionStatus::Starting);
        assert_eq!(rec.tmux_session, format!("mirai-{}", rec.id));

        let spawned = backend.spawned.lock().unwrap();
        assert_eq!(spawned.len(), 1);
        let (session, dir, command, env) = &spawned[0];
        assert_eq!(session, &rec.tmux_session);
        assert_eq!(dir, "/tmp");
        assert_eq!(command, "claude --model claude-opus-4-8 --effort high");
        // M6: identity + engine port injected into the tmux session env.
        assert!(env.contains(&("MIRAI_SESSION_ID".to_string(), rec.id.clone())));
        assert!(env.contains(&("MIRAI_PORT".to_string(), "3000".to_string())));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_rejects_shell_injection_in_model_and_effort() {
        // Regresión de seguridad: model/effort se interpolan en el shell-command que
        // tmux corre vía /bin/sh -c; un valor con metacaracteres sería RCE.
        let (manager, backend, path) = manager_with_fake();

        let mut evil = spawn_params();
        evil.model = Some("opus; touch /tmp/pwned".into());
        assert!(manager.spawn(evil).await.is_err(), "model con ';' debe rechazarse");

        let mut evil2 = spawn_params();
        evil2.effort = Some("high$(id)".into());
        assert!(manager.spawn(evil2).await.is_err(), "effort con $(...) debe rechazarse");

        let mut evil3 = spawn_params();
        evil3.model = Some("opus foo".into()); // espacio → arg extra al shell
        assert!(manager.spawn(evil3).await.is_err(), "model con espacio debe rechazarse");

        // Nada malicioso llegó al backend.
        assert!(
            backend.spawned.lock().unwrap().is_empty(),
            "ningún spawn debió ejecutarse con input malicioso"
        );

        // El camino feliz (model/effort válidos) sigue funcionando.
        manager.spawn(spawn_params()).await.unwrap();
        assert_eq!(backend.spawned.lock().unwrap().len(), 1);

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_injects_configured_server_port() {
        let (manager, backend, path) = manager_with_fake();
        manager.set_server_port(4321);
        manager.spawn(spawn_params()).await.unwrap();

        let spawned = backend.spawned.lock().unwrap();
        assert!(spawned[0]
            .3
            .contains(&("MIRAI_PORT".to_string(), "4321".to_string())));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_injects_api_key_only_when_configured() {
        // M6 fix (code review): with --api-key, the activity hook needs
        // MIRAI_API_KEY in the session env or its POST 401s silently.
        let (manager, backend, path) = manager_with_fake();

        // No key configured → no MIRAI_API_KEY in the env.
        manager.spawn(spawn_params()).await.unwrap();
        {
            let spawned = backend.spawned.lock().unwrap();
            assert!(
                !spawned[0].3.iter().any(|(k, _)| k == "MIRAI_API_KEY"),
                "sin api key configurada no debe inyectarse MIRAI_API_KEY"
            );
        }

        // Key configured → injected alongside MIRAI_SESSION_ID / MIRAI_PORT.
        manager.set_api_key(Some("sekreto-123".into()));
        manager.spawn(spawn_params()).await.unwrap();
        {
            let spawned = backend.spawned.lock().unwrap();
            let env = &spawned[1].3;
            assert!(env.contains(&("MIRAI_API_KEY".to_string(), "sekreto-123".to_string())));
            assert!(env.iter().any(|(k, _)| k == "MIRAI_SESSION_ID"));
            assert!(env.iter().any(|(k, _)| k == "MIRAI_PORT"));
        }
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_with_hooks_appends_settings_and_writes_files() {
        let (manager, backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.no_hooks = false;
        let rec = manager.spawn(params).await.unwrap();

        let spawned = backend.spawned.lock().unwrap();
        let command = &spawned[0].2;
        assert!(
            command.contains("--settings '"),
            "hooks on → --settings in command: {command}"
        );
        assert!(command.contains(&format!("{}-settings.json", rec.id)));

        // Both artifacts exist under <registry dir>/hooks/.
        let base = path.parent().unwrap();
        assert!(base.join("hooks/report-activity.py").exists());
        let settings_path = base.join(format!("hooks/{}-settings.json", rec.id));
        assert!(settings_path.exists());
        let _ = std::fs::remove_file(&settings_path);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_with_mcp_appends_config_flags_and_writes_file() {
        // MCP wiring: with mcp=true the command gains
        // `--mcp-config '<...>-mcp.json' --strict-mcp-config` and the config
        // file materializes under <registry dir>/hooks/ with ONLY our servers.
        let (manager, backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.mcp = true;
        let rec = manager.spawn(params).await.unwrap();

        let spawned = backend.spawned.lock().unwrap();
        let command = &spawned[0].2;
        assert!(
            command.contains("--mcp-config '"),
            "mcp on → --mcp-config in command: {command}"
        );
        assert!(
            command.contains("--strict-mcp-config"),
            "mcp on → --strict-mcp-config in command: {command}"
        );
        assert!(command.contains(&format!("{}-mcp.json", rec.id)));

        // The config file exists and declares the atlassian server only.
        let base = path.parent().unwrap();
        let cfg_path = base.join(format!("hooks/{}-mcp.json", rec.id));
        assert!(cfg_path.exists(), "mcp config file must be written");
        let cfg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
        let servers = cfg["mcpServers"].as_object().unwrap();
        assert!(servers.contains_key("atlassian"), "atlassian server present");
        assert_eq!(servers["atlassian"]["type"], "http");
        assert_eq!(
            servers["atlassian"]["url"],
            crate::sessions::hooks::ATLASSIAN_MCP_URL
        );
        let _ = std::fs::remove_file(&cfg_path);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_without_mcp_omits_config_flags() {
        // Default (opt-out) → no MCP flags, unchanged prior behavior.
        let (manager, backend, path) = manager_with_fake();
        manager.spawn(spawn_params()).await.unwrap();
        let spawned = backend.spawned.lock().unwrap();
        let command = &spawned[0].2;
        assert!(!command.contains("--mcp-config"), "no mcp flags by default: {command}");
        assert!(!command.contains("--strict-mcp-config"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn parent_id_round_trips_registry_and_api_json() {
        let backend = Arc::new(FakeBackend::new());
        let path = temp_registry();
        let manager = SessionManager::new(backend.clone(), path.clone());

        let parent = manager.spawn(spawn_params()).await.unwrap();
        let mut child_params = spawn_params();
        child_params.parent_id = Some(parent.id.clone());
        let child = manager.spawn(child_params).await.unwrap();

        assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(
            child.to_api_json()["parent_id"],
            serde_json::json!(parent.id)
        );
        assert_eq!(parent.to_api_json()["parent_id"], serde_json::Value::Null);

        // Survives a registry reload.
        let manager2 = SessionManager::new(backend, path.clone());
        manager2.initialize().await.unwrap();
        assert_eq!(
            manager2.get(&child.id).await.unwrap().parent_id.as_deref(),
            Some(parent.id.as_str())
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_without_model_uses_bare_claude() {
        let (manager, backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.model = None;
        params.effort = None;
        manager.spawn(params).await.unwrap();

        let spawned = backend.spawned.lock().unwrap();
        assert_eq!(spawned[0].2, "claude");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_validates_required_fields() {
        let (manager, _backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.project_dir = "  ".into();
        assert!(matches!(
            manager.spawn(params).await,
            Err(SessionError::Invalid(_))
        ));

        let mut params = spawn_params();
        params.objective = String::new();
        assert!(matches!(
            manager.spawn(params).await,
            Err(SessionError::Invalid(_))
        ));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_and_get_roundtrip() {
        let (manager, _backend, path) = manager_with_fake();
        let a = manager.spawn(spawn_params()).await.unwrap();
        let b = manager.spawn(spawn_params()).await.unwrap();

        let list = manager.list().await;
        assert_eq!(list.len(), 2);
        assert!(manager.get(&a.id).await.is_some());
        assert!(manager.get(&b.id).await.is_some());
        assert!(manager.get("ghost").await.is_none());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn send_forwards_text_and_marks_first_prompt() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        manager.send(&rec.id, "hello").await.unwrap();
        let sent = backend.sent.lock().unwrap().clone();
        assert_eq!(sent, vec![(rec.tmux_session.clone(), "hello".to_string())]);

        let rec = manager.get(&rec.id).await.unwrap();
        assert!(rec.first_prompt_sent);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn ultracode_prepends_keyword_on_first_prompt_only() {
        let (manager, backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.ultracode = true;
        let rec = manager.spawn(params).await.unwrap();

        manager.send(&rec.id, "first task").await.unwrap();
        manager.send(&rec.id, "second task").await.unwrap();

        let sent = backend.sent.lock().unwrap().clone();
        assert_eq!(sent[0].1, "ultracode first task");
        assert_eq!(sent[1].1, "second task");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn send_to_unknown_or_stopped_session_fails() {
        let (manager, _backend, path) = manager_with_fake();
        assert!(matches!(
            manager.send("ghost", "hi").await,
            Err(SessionError::NotFound(_))
        ));

        let rec = manager.spawn(spawn_params()).await.unwrap();
        manager.stop(&rec.id).await.unwrap();
        assert!(matches!(
            manager.send(&rec.id, "hi").await,
            Err(SessionError::Stopped(_))
        ));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn stop_kills_backend_session_and_marks_stopped() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();
        assert!(backend.session_exists(&rec.tmux_session));

        let stopped = manager.stop(&rec.id).await.unwrap();
        assert_eq!(stopped.status, SessionStatus::Stopped);
        assert!(!backend.session_exists(&rec.tmux_session));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn output_returns_pane_tail() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();
        backend.set_pane(&rec.tmux_session, &["line1", "line2", "line3"]);

        let out = manager.output(&rec.id, 2).await.unwrap();
        assert_eq!(out, vec!["line2".to_string(), "line3".to_string()]);

        // Stopped session → empty output, no error.
        manager.stop(&rec.id).await.unwrap();
        assert!(manager.output(&rec.id, 10).await.unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn registry_persists_roundtrip() {
        let backend = Arc::new(FakeBackend::new());
        let path = temp_registry();

        let manager = SessionManager::new(backend.clone(), path.clone());
        let rec = manager.spawn(spawn_params()).await.unwrap();

        // A fresh manager over the same file sees the session (tmux alive).
        let manager2 = SessionManager::new(backend.clone(), path.clone());
        manager2.initialize().await.unwrap();
        let loaded = manager2.get(&rec.id).await.unwrap();
        assert_eq!(loaded.objective, "do the thing");
        assert_eq!(loaded.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(loaded.effort.as_deref(), Some("high"));
        assert_eq!(loaded.status, SessionStatus::Starting);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn initialize_reconciles_dead_tmux_sessions_to_stopped() {
        let backend = Arc::new(FakeBackend::new());
        let path = temp_registry();

        let manager = SessionManager::new(backend.clone(), path.clone());
        let alive = manager.spawn(spawn_params()).await.unwrap();
        let dead = manager.spawn(spawn_params()).await.unwrap();

        // Simulate the second tmux session dying while the server is down.
        backend.kill_externally(&dead.tmux_session);

        let manager2 = SessionManager::new(backend.clone(), path.clone());
        manager2.initialize().await.unwrap();

        assert_eq!(
            manager2.get(&alive.id).await.unwrap().status,
            SessionStatus::Starting
        );
        assert_eq!(
            manager2.get(&dead.id).await.unwrap().status,
            SessionStatus::Stopped
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn initialize_with_missing_registry_is_empty() {
        let (manager, _backend, path) = manager_with_fake();
        manager.initialize().await.unwrap();
        assert!(manager.list().await.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn poll_transitions_status_from_pane_content_and_emits_events() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();
        let mut rx = manager.subscribe();

        // Claude booted into an idle prompt → waiting.
        backend.set_pane(&rec.tmux_session, &["Welcome to Claude Code", "│ > "]);
        manager.poll_once().await;
        assert_eq!(
            manager.get(&rec.id).await.unwrap().status,
            SessionStatus::Waiting
        );
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionStatusChanged);
        assert_eq!(ev.data["status"], serde_json::json!("waiting"));
        // Contract shape: {id, status, last_activity}.
        assert_eq!(ev.data["id"], serde_json::json!(rec.id));
        assert!(ev.data["last_activity"].is_string());
        // The pane change also produces a session_output event.
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionOutput);

        // Now it is working.
        backend.set_pane(&rec.tmux_session, &["✻ Cranking… (esc to interrupt)"]);
        manager.poll_once().await;
        assert_eq!(
            manager.get(&rec.id).await.unwrap().status,
            SessionStatus::Working
        );

        // Permission dialog appears.
        backend.set_pane(
            &rec.tmux_session,
            &["Do you want to run this command?", "1. Yes", "2. No"],
        );
        manager.poll_once().await;
        assert_eq!(
            manager.get(&rec.id).await.unwrap().status,
            SessionStatus::Permission
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn poll_detects_externally_killed_session() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();
        let mut rx = manager.subscribe();

        backend.kill_externally(&rec.tmux_session);
        manager.poll_once().await;

        assert_eq!(
            manager.get(&rec.id).await.unwrap().status,
            SessionStatus::Stopped
        );
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionStatusChanged);
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionStopped);
        // Contract shape: session_stopped → {id}.
        assert_eq!(ev.data["id"], serde_json::json!(rec.id));
        assert_eq!(ev.data.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn poll_emits_output_only_when_tail_changes() {
        let (manager, backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        backend.set_pane(&rec.tmux_session, &["hello", "│ > "]);
        manager.poll_once().await;
        let mut rx = manager.subscribe();

        // Same pane → no new output event (only subscribe after first poll).
        manager.poll_once().await;
        assert!(rx.try_recv().is_err());

        // Changed pane → output event with ONLY the new lines.
        backend.set_pane(&rec.tmux_session, &["hello", "world", "│ > "]);
        manager.poll_once().await;
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionOutput);
        let lines = ev.data["lines"].as_array().unwrap();
        assert!(lines.iter().any(|l| l == "world"));
        // "hello" was already seen in the previous capture → not re-emitted.
        assert!(!lines.iter().any(|l| l == "hello"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diff_new_lines_covers_scroll_rewrite_and_noop() {
        let l = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };

        // No change → nothing new.
        assert!(diff_new_lines(&l(&["a", "b"]), &l(&["a", "b"])).is_empty());

        // Append at the bottom.
        assert_eq!(
            diff_new_lines(&l(&["a", "b"]), &l(&["a", "b", "c"])),
            l(&["c"])
        );

        // Scroll: old top lines fell off the window.
        assert_eq!(
            diff_new_lines(&l(&["a", "b", "c"]), &l(&["b", "c", "d"])),
            l(&["d"])
        );

        // In-place rewrite of the last line (spinner / streaming).
        assert_eq!(
            diff_new_lines(&l(&["a", "spinner1"]), &l(&["a", "spinner2"])),
            l(&["spinner2"])
        );

        // First capture: everything is new.
        assert_eq!(diff_new_lines(&[], &l(&["x", "y"])), l(&["x", "y"]));
    }

    #[tokio::test]
    async fn spawn_emits_session_created() {
        let (manager, _backend, path) = manager_with_fake();
        let mut rx = manager.subscribe();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionCreated);
        assert_eq!(ev.session_id, rec.id);
        assert_eq!(ev.data["tmux_session"], serde_json::json!(rec.tmux_session));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn api_json_matches_contract_fields() {
        let rec = SessionRecord {
            id: "abc".into(),
            name: "n".into(),
            project_dir: "/p".into(),
            objective: "o".into(),
            status: SessionStatus::Waiting,
            model: None,
            effort: None,
            ultracode: true,
            tmux_session: "mirai-abc".into(),
            created_at: "2026-07-09T00:00:00Z".into(),
            last_activity: "2026-07-09T00:00:00Z".into(),
            first_prompt_sent: true,
            parent_id: Some("root".into()),
            external: false,
        };
        let v = rec.to_api_json();
        let obj = v.as_object().unwrap();
        let expected = [
            "id",
            "name",
            "project_dir",
            "objective",
            "status",
            "model",
            "effort",
            "ultracode",
            "tmux_session",
            "created_at",
            "last_activity",
            "parent_id",
        ];
        assert_eq!(obj.len(), expected.len());
        for key in expected {
            assert!(obj.contains_key(key), "missing contract field {key}");
        }
        // Internal bookkeeping must NOT leak into the API shape.
        assert!(!obj.contains_key("first_prompt_sent"));
    }

    // -- M6: resource activity ------------------------------------------------

    #[tokio::test]
    async fn spawn_rejects_missing_project_dir_without_create_dir() {
        let (manager, _backend, path) = manager_with_fake();
        let missing = std::env::temp_dir().join(format!("mirai-m7-missing-{}", short_id()));

        let mut params = spawn_params();
        params.project_dir = missing.to_string_lossy().into_owned();
        let err = manager.spawn(params).await.unwrap_err();
        match err {
            SessionError::Invalid(msg) => {
                assert!(msg.contains("does not exist"), "clear message: {msg}");
                assert!(msg.contains("create_dir"), "hints the fix: {msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn spawn_with_create_dir_makes_the_directory() {
        let (manager, _backend, path) = manager_with_fake();
        let target = std::env::temp_dir()
            .join(format!("mirai-m7-new-{}", short_id()))
            .join("nested");

        let mut params = spawn_params();
        params.project_dir = target.to_string_lossy().into_owned();
        params.create_dir = true;
        let rec = manager.spawn(params).await.unwrap();

        assert!(target.is_dir(), "mkdir -p happened");
        assert_eq!(rec.project_dir, target.to_string_lossy());
        let _ = std::fs::remove_dir_all(target.parent().unwrap());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_projects_unions_sessions_and_scan_with_dedup() {
        let (manager, _backend, path) = manager_with_fake();

        // Scan root with: two real projects, a hidden dir, node_modules, a file.
        let root = std::env::temp_dir().join(format!("mirai-m7-root-{}", short_id()));
        for sub in ["beta-app", "alpha-app", ".hidden", "node_modules"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(root.join("not-a-dir.txt"), "x").unwrap();

        // One session in a scanned project (dedup: session wins), one outside
        // whose dir does NOT exist on disk.
        let mut in_scan = spawn_params();
        in_scan.project_dir = root.join("beta-app").to_string_lossy().into_owned();
        manager.spawn(in_scan).await.unwrap();

        let ghost_dir = std::env::temp_dir().join(format!("mirai-m7-ghost-{}", short_id()));
        let mut ghost = spawn_params();
        ghost.project_dir = ghost_dir.to_string_lossy().into_owned();
        ghost.create_dir = true;
        manager.spawn(ghost).await.unwrap();
        std::fs::remove_dir_all(&ghost_dir).unwrap(); // dir vanished after spawn

        let projects = manager.list_projects(&[root.clone()]).await;

        // session entries first (alphabetical), then scan (alphabetical).
        let sources: Vec<&str> = projects
            .iter()
            .map(|p| p["source"].as_str().unwrap())
            .collect();
        assert_eq!(sources, vec!["session", "session", "scan"]);

        let paths: Vec<&str> = projects
            .iter()
            .map(|p| p["path"].as_str().unwrap())
            .collect();
        // beta-app deduped into the session group; only alpha-app from scan.
        assert!(paths.contains(&root.join("beta-app").to_str().unwrap()));
        assert_eq!(paths[2], root.join("alpha-app").to_str().unwrap());
        assert!(!paths.iter().any(|p| p.contains(".hidden")));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.iter().any(|p| p.contains("not-a-dir")));

        // exists reflects disk truth; name = basename.
        let ghost_entry = projects
            .iter()
            .find(|p| p["path"] == ghost_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(ghost_entry["exists"], false);
        let alpha = &projects[2];
        assert_eq!(alpha["name"], "alpha-app");
        assert_eq!(alpha["exists"], true);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_projects_without_roots_uses_sessions_only() {
        let (manager, _backend, path) = manager_with_fake();
        manager.spawn(spawn_params()).await.unwrap();
        let projects = manager.list_projects(&[]).await;
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["source"], "session");
        assert_eq!(projects[0]["path"], "/tmp");
        // Missing scan root is tolerated silently.
        let bogus = std::env::temp_dir().join("mirai-m7-no-root-xyz");
        let projects = manager.list_projects(&[bogus]).await;
        assert_eq!(projects.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn unknown_parent_id_is_normalized_to_null() {
        let (manager, _backend, path) = manager_with_fake();
        let mut params = spawn_params();
        params.parent_id = Some("no-such-session".into());
        let rec = manager.spawn(params).await.unwrap();
        assert_eq!(rec.parent_id, None);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn record_activity_stores_emits_and_lists_newest_first() {
        let (manager, _backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();
        let mut rx = manager.subscribe();

        let entry = manager
            .record_activity(&rec.id, "Write", "write", Some("/tmp/foo.rs"))
            .await
            .unwrap();
        assert_eq!(entry["action"], "write");
        assert_eq!(entry["path"], "/tmp/foo.rs");
        assert!(entry["ts"].is_string());

        // SSE event with the contract shape {id, tool, action, path, ts}.
        // (skip the session_created event from the spawn — we subscribed after,
        // so the first event IS the activity one)
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event_type, EventType::SessionActivity);
        assert_eq!(ev.data["id"], serde_json::json!(rec.id));
        assert_eq!(ev.data["tool"], serde_json::json!("Write"));
        assert_eq!(ev.data["action"], serde_json::json!("write"));
        assert_eq!(ev.data["path"], serde_json::json!("/tmp/foo.rs"));
        assert!(ev.data["ts"].is_string());

        // Bash without a path → path null.
        manager
            .record_activity(&rec.id, "Bash", "exec", None)
            .await
            .unwrap();

        let recent = manager.get_activity(&rec.id, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Chronological order (contract fixed by the M6 web UI).
        assert_eq!(recent[0]["tool"], "Write");
        assert_eq!(recent[1]["tool"], "Bash");
        assert_eq!(recent[1]["path"], serde_json::Value::Null);

        // Limit keeps the MOST RECENT entries (still chronological).
        let limited = manager.get_activity(&rec.id, 1).await.unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0]["tool"], "Bash");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn activity_ring_buffer_caps_at_max() {
        let (manager, _backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        for i in 0..(ACTIVITY_RING_MAX + 50) {
            manager
                .record_activity(&rec.id, "Read", "read", Some(&format!("/f/{i}")))
                .await
                .unwrap();
        }
        let all = manager.get_activity(&rec.id, usize::MAX).await.unwrap();
        assert_eq!(all.len(), ACTIVITY_RING_MAX);
        // Oldest entries were evicted; chronological → the LAST is the newest.
        assert_eq!(
            all[ACTIVITY_RING_MAX - 1]["path"],
            format!("/f/{}", ACTIVITY_RING_MAX + 49)
        );
        assert_eq!(all[0]["path"], format!("/f/{}", 50));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn activity_validates_action_and_session() {
        let (manager, _backend, path) = manager_with_fake();
        let rec = manager.spawn(spawn_params()).await.unwrap();

        assert!(matches!(
            manager.record_activity(&rec.id, "Write", "delete", None).await,
            Err(SessionError::Invalid(_))
        ));
        assert!(manager
            .record_activity(&rec.id, "WebFetch", "net", Some("https://example.com"))
            .await
            .is_ok());
        assert!(matches!(
            manager.record_activity(&rec.id, " ", "read", None).await,
            Err(SessionError::Invalid(_))
        ));
        assert!(matches!(
            manager.record_activity("ghost", "Write", "write", None).await,
            Err(SessionError::NotFound(_))
        ));
        assert!(matches!(
            manager.get_activity("ghost", 10).await,
            Err(SessionError::NotFound(_))
        ));
        let _ = std::fs::remove_file(path);
    }
}
