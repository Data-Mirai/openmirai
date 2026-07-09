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
}

impl SessionManager {
    pub fn new(backend: Arc<dyn SessionBackend>, registry_path: PathBuf) -> Self {
        Self {
            backend,
            registry_path,
            sessions: RwLock::new(HashMap::new()),
            events: EventEmitter::new(256),
            last_pane: Mutex::new(HashMap::new()),
            polling_started: AtomicBool::new(false),
        }
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
                if rec.status.is_active() && !self.backend.session_exists(&rec.tmux_session) {
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

        let id = short_id();
        let tmux_session = format!("{TMUX_SESSION_PREFIX}{id}");

        let mut command = String::from("claude");
        if let Some(model) = &params.model {
            command.push_str(&format!(" --model {model}"));
        }
        if let Some(effort) = &params.effort {
            command.push_str(&format!(" --effort {effort}"));
        }

        self.backend
            .spawn(&tmux_session, &params.project_dir, &command)?;

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
        };

        self.sessions
            .write()
            .await
            .insert(id.clone(), record.clone());
        self.persist().await?;

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
        let (tmux_session, payload) = {
            let sessions = self.sessions.read().await;
            let rec = sessions
                .get(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
            if !rec.status.is_active() {
                return Err(SessionError::Stopped(id.to_string()));
            }
            let payload = if rec.ultracode && !rec.first_prompt_sent {
                format!("ultracode {text}")
            } else {
                text.to_string()
            };
            (rec.tmux_session.clone(), payload)
        };

        self.backend.send_text(&tmux_session, &payload)?;

        {
            let mut sessions = self.sessions.write().await;
            if let Some(rec) = sessions.get_mut(id) {
                rec.first_prompt_sent = true;
                rec.last_activity = now_rfc3339();
            }
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

    // -- status polling ------------------------------------------------------

    /// Number of sessions whose tmux session should still be alive.
    pub async fn active_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|r| r.status.is_active())
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
                .filter(|r| r.status.is_active())
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
            project_dir: "/tmp/project".into(),
            objective: "do the thing".into(),
            model: Some("claude-opus-4-8".into()),
            effort: Some("high".into()),
            ultracode: false,
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
        let (session, dir, command) = &spawned[0];
        assert_eq!(session, &rec.tmux_session);
        assert_eq!(dir, "/tmp/project");
        assert_eq!(command, "claude --model claude-opus-4-8 --effort high");
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
        ];
        assert_eq!(obj.len(), expected.len());
        for key in expected {
            assert!(obj.contains_key(key), "missing contract field {key}");
        }
        // Internal bookkeeping must NOT leak into the API shape.
        assert!(!obj.contains_key("first_prompt_sent"));
    }
}
