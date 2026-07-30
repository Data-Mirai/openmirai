//! FleetStore — SQLite (WAL) source of truth for the agent fleet.
//!
//! Persists the last-known state of every fleet member in a single `fleet`
//! table and broadcasts a [`FleetEvent`] on every mutation so the SSE stream
//! can push changes live. Opened in WAL mode (REGLA-504) so the poll/reader
//! side never blocks a heartbeat writer.
//!
//! rusqlite is synchronous; every operation here is a single fast statement, so
//! the connection is guarded by a `std::sync::Mutex` and called directly (the
//! lock is never held across an `.await`).

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::broadcast;

use crate::utils::now_epoch;

use super::types::{FleetEvent, FleetEventKind, FleetMember, FleetQuery, FleetStatus, StatusUpdate};

/// Broadcast backlog kept for slow SSE subscribers before they start lagging.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// DDL for the fleet SoT. `IF NOT EXISTS` keeps it idempotent across restarts.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS fleet (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL DEFAULT '',
    kind         TEXT NOT NULL DEFAULT 'agent',
    host         TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'unknown',
    activity     TEXT NOT NULL DEFAULT '',
    objective    TEXT NOT NULL DEFAULT '',
    project_dir  TEXT NOT NULL DEFAULT '',
    metadata     TEXT NOT NULL DEFAULT '{}',
    first_seen   REAL NOT NULL,
    last_seen    REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_fleet_status    ON fleet(status);
CREATE INDEX IF NOT EXISTS idx_fleet_last_seen ON fleet(last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_fleet_host      ON fleet(host);
"#;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum FleetError {
    #[error("fleet db error: {0}")]
    Db(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("not found: {0}")]
    NotFound(String),
}

impl From<rusqlite::Error> for FleetError {
    fn from(e: rusqlite::Error) -> Self {
        FleetError::Db(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// FleetStore
// ---------------------------------------------------------------------------

/// SQLite-backed source of truth for the fleet.
///
/// `Mutex<Connection>` is `Sync` because `Connection` is `Send`, so the store
/// is `Send + Sync` and can live in the shared axum `AppState` without any
/// `unsafe`.
pub struct FleetStore {
    conn: Mutex<Connection>,
    events: broadcast::Sender<FleetEvent>,
}

impl FleetStore {
    /// Open (or create) the fleet DB at `path` in WAL mode and ensure the
    /// schema exists. Pass `":memory:"` for a throwaway store.
    pub fn open(path: &str) -> Result<Self, FleetError> {
        let conn = Connection::open(path)
            .map_err(|e| FleetError::Db(format!("failed to open fleet db at {path}: {e}")))?;
        // WAL: concurrent reads never block the heartbeat writer. busy_timeout
        // survives brief writer contention instead of failing with SQLITE_BUSY.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;",
        )
        .map_err(|e| FleetError::Db(format!("failed to set PRAGMAs: {e}")))?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| FleetError::Db(format!("failed to init fleet schema: {e}")))?;

        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Ok(Self {
            conn: Mutex::new(conn),
            events,
        })
    }

    /// In-memory store — the default for tests and for `AppState::new` before
    /// `serve()` swaps in a file-backed one.
    pub fn in_memory() -> Result<Self, FleetError> {
        Self::open(":memory:")
    }

    /// Default on-disk location for the live server: `~/.openmirai/fleet.db`.
    pub fn default_path() -> std::path::PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::Path::new(&home)
            .join(".openmirai")
            .join("fleet.db")
    }

    /// Subscribe to the live event stream (SSE).
    pub fn subscribe(&self) -> broadcast::Receiver<FleetEvent> {
        self.events.subscribe()
    }

    /// Apply a status report: insert a new member or update an existing one,
    /// stamping `last_seen`. Returns the resulting member plus whether it was
    /// [`FleetEventKind::Added`] or [`FleetEventKind::Updated`], and broadcasts
    /// the matching event.
    pub fn apply_status(
        &self,
        update: StatusUpdate,
    ) -> Result<(FleetMember, FleetEventKind), FleetError> {
        let id = update.id.trim();
        if id.is_empty() {
            return Err(FleetError::Invalid("id is required".into()));
        }
        let now = now_epoch();

        let existing = self.get(id)?;
        let (member, kind) = match existing {
            Some(prev) => {
                // Merge: only provided fields overwrite the stored ones.
                let member = FleetMember {
                    id: prev.id,
                    name: update.name.unwrap_or(prev.name),
                    kind: update.kind.unwrap_or(prev.kind),
                    host: update.host.unwrap_or(prev.host),
                    status: update.status,
                    activity: update.activity.unwrap_or(prev.activity),
                    objective: update.objective.unwrap_or(prev.objective),
                    project_dir: update.project_dir.unwrap_or(prev.project_dir),
                    metadata: update.metadata.unwrap_or(prev.metadata),
                    first_seen: prev.first_seen,
                    last_seen: now,
                };
                (member, FleetEventKind::Updated)
            }
            None => {
                let member = FleetMember {
                    id: id.to_string(),
                    name: update.name.unwrap_or_else(|| id.to_string()),
                    kind: update.kind.unwrap_or_else(|| "agent".to_string()),
                    host: update.host.unwrap_or_default(),
                    status: update.status,
                    activity: update.activity.unwrap_or_default(),
                    objective: update.objective.unwrap_or_default(),
                    project_dir: update.project_dir.unwrap_or_default(),
                    metadata: update.metadata.unwrap_or_else(|| Value::Object(Default::default())),
                    first_seen: now,
                    last_seen: now,
                };
                (member, FleetEventKind::Added)
            }
        };

        self.upsert(&member)?;
        // A send error just means no live subscribers — not a failure.
        let _ = self.events.send(FleetEvent {
            kind,
            member: member.clone(),
        });
        Ok((member, kind))
    }

    /// Fetch a single member by id.
    pub fn get(&self, id: &str) -> Result<Option<FleetMember>, FleetError> {
        let conn = self.conn.lock().expect("fleet mutex poisoned");
        let member = conn
            .query_row(
                "SELECT id, name, kind, host, status, activity, objective, \
                 project_dir, metadata, first_seen, last_seen \
                 FROM fleet WHERE id = ?1",
                params![id],
                row_to_member,
            )
            .optional()?;
        member.transpose()
    }

    /// List members, newest heartbeat first, with optional filters.
    pub fn list(&self, query: &FleetQuery) -> Result<Vec<FleetMember>, FleetError> {
        let conn = self.conn.lock().expect("fleet mutex poisoned");

        // Build the WHERE clause dynamically but with bound params only.
        let mut sql = String::from(
            "SELECT id, name, kind, host, status, activity, objective, \
             project_dir, metadata, first_seen, last_seen FROM fleet",
        );
        let mut clauses: Vec<String> = Vec::new();
        let mut binds: Vec<String> = Vec::new();
        if let Some(status) = query.status {
            clauses.push(format!("status = ?{}", binds.len() + 1));
            binds.push(status.as_str().to_string());
        }
        if let Some(host) = &query.host {
            clauses.push(format!("host = ?{}", binds.len() + 1));
            binds.push(host.clone());
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY last_seen DESC");
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            binds.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(param_refs.as_slice(), row_to_member)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Remove a member. Returns `true` if a row was deleted and broadcasts a
    /// [`FleetEventKind::Removed`] event for it.
    pub fn remove(&self, id: &str) -> Result<bool, FleetError> {
        let existing = self.get(id)?;
        let Some(member) = existing else {
            return Ok(false);
        };
        {
            let conn = self.conn.lock().expect("fleet mutex poisoned");
            conn.execute("DELETE FROM fleet WHERE id = ?1", params![id])?;
        }
        let _ = self.events.send(FleetEvent {
            kind: FleetEventKind::Removed,
            member,
        });
        Ok(true)
    }

    /// Total number of members in the SoT.
    pub fn count(&self) -> Result<usize, FleetError> {
        let conn = self.conn.lock().expect("fleet mutex poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM fleet", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// The connection's active `journal_mode` — used by tests to prove WAL.
    pub fn journal_mode(&self) -> Result<String, FleetError> {
        let conn = self.conn.lock().expect("fleet mutex poisoned");
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        Ok(mode)
    }

    // -- internal ----------------------------------------------------------

    fn upsert(&self, m: &FleetMember) -> Result<(), FleetError> {
        let metadata = serde_json::to_string(&m.metadata)
            .map_err(|e| FleetError::Serialization(e.to_string()))?;
        let conn = self.conn.lock().expect("fleet mutex poisoned");
        conn.execute(
            "INSERT INTO fleet \
             (id, name, kind, host, status, activity, objective, project_dir, \
              metadata, first_seen, last_seen) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(id) DO UPDATE SET \
              name=excluded.name, kind=excluded.kind, host=excluded.host, \
              status=excluded.status, activity=excluded.activity, \
              objective=excluded.objective, project_dir=excluded.project_dir, \
              metadata=excluded.metadata, last_seen=excluded.last_seen",
            params![
                m.id,
                m.name,
                m.kind,
                m.host,
                m.status.as_str(),
                m.activity,
                m.objective,
                m.project_dir,
                metadata,
                m.first_seen,
                m.last_seen,
            ],
        )?;
        Ok(())
    }
}

/// Map a DB row to a [`FleetMember`]. Returns an inner `Result` so a malformed
/// `metadata`/`status` blob surfaces as a `FleetError` rather than a panic.
fn row_to_member(row: &Row<'_>) -> rusqlite::Result<Result<FleetMember, FleetError>> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let kind: String = row.get(2)?;
    let host: String = row.get(3)?;
    let status_raw: String = row.get(4)?;
    let activity: String = row.get(5)?;
    let objective: String = row.get(6)?;
    let project_dir: String = row.get(7)?;
    let metadata_raw: String = row.get(8)?;
    let first_seen: f64 = row.get(9)?;
    let last_seen: f64 = row.get(10)?;

    Ok((|| {
        let status = FleetStatus::parse(&status_raw)
            .ok_or_else(|| FleetError::Serialization(format!("bad status '{status_raw}'")))?;
        let metadata: Value = serde_json::from_str(&metadata_raw)
            .map_err(|e| FleetError::Serialization(format!("bad metadata: {e}")))?;
        Ok(FleetMember {
            id,
            name,
            kind,
            host,
            status,
            activity,
            objective,
            project_dir,
            metadata,
            first_seen,
            last_seen,
        })
    })())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(id: &str, status: FleetStatus) -> StatusUpdate {
        StatusUpdate {
            id: id.to_string(),
            status,
            name: None,
            kind: None,
            host: None,
            activity: None,
            objective: None,
            project_dir: None,
            metadata: None,
        }
    }

    #[test]
    fn open_enables_wal_on_disk() {
        let dir = std::env::temp_dir().join(format!("mirai-fleet-wal-{}", crate::utils::short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fleet.db");
        let store = FleetStore::open(path.to_str().unwrap()).unwrap();
        assert_eq!(store.journal_mode().unwrap().to_lowercase(), "wal");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn first_status_adds_then_second_updates() {
        let store = FleetStore::in_memory().unwrap();

        let mut u = update("mac-centro", FleetStatus::Working);
        u.name = Some("Centro".into());
        u.host = Some("mac".into());
        let (m, kind) = store.apply_status(u).unwrap();
        assert_eq!(kind, FleetEventKind::Added);
        assert_eq!(m.name, "Centro");
        assert_eq!(m.status, FleetStatus::Working);
        assert_eq!(m.first_seen, m.last_seen);

        // Bare heartbeat keeps name/host, updates status + last_seen.
        let (m2, kind2) = store.apply_status(update("mac-centro", FleetStatus::Idle)).unwrap();
        assert_eq!(kind2, FleetEventKind::Updated);
        assert_eq!(m2.name, "Centro", "omitted field preserved");
        assert_eq!(m2.host, "mac");
        assert_eq!(m2.status, FleetStatus::Idle);
        assert!(m2.last_seen >= m.last_seen);
        assert_eq!(m2.first_seen, m.first_seen, "first_seen never moves");

        assert_eq!(store.count().unwrap(), 1, "upsert, not duplicate");
    }

    #[test]
    fn empty_id_is_invalid() {
        let store = FleetStore::in_memory().unwrap();
        let err = store.apply_status(update("  ", FleetStatus::Online));
        assert!(matches!(err, Err(FleetError::Invalid(_))));
    }

    #[test]
    fn metadata_roundtrips() {
        let store = FleetStore::in_memory().unwrap();
        let mut u = update("vps-worker", FleetStatus::Online);
        u.metadata = Some(json!({"pid": 3750, "port": 4321}));
        store.apply_status(u).unwrap();
        let got = store.get("vps-worker").unwrap().unwrap();
        assert_eq!(got.metadata, json!({"pid": 3750, "port": 4321}));
    }

    #[test]
    fn list_filters_and_orders_by_last_seen() {
        let store = FleetStore::in_memory().unwrap();
        let mut a = update("a", FleetStatus::Working);
        a.host = Some("mac".into());
        store.apply_status(a).unwrap();
        let mut b = update("b", FleetStatus::Idle);
        b.host = Some("vps".into());
        store.apply_status(b).unwrap();
        // Touch `a` again so it becomes the most-recent heartbeat.
        store.apply_status(update("a", FleetStatus::Waiting)).unwrap();

        let all = store.list(&FleetQuery::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "a", "newest last_seen first");

        let mac = store
            .list(&FleetQuery {
                host: Some("mac".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(mac.len(), 1);
        assert_eq!(mac[0].id, "a");

        let idle = store
            .list(&FleetQuery {
                status: Some(FleetStatus::Idle),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].id, "b");
    }

    #[test]
    fn remove_deletes_and_reports() {
        let store = FleetStore::in_memory().unwrap();
        store.apply_status(update("gone", FleetStatus::Online)).unwrap();
        assert!(store.remove("gone").unwrap());
        assert!(store.get("gone").unwrap().is_none());
        assert!(!store.remove("gone").unwrap(), "second remove is a no-op");
    }

    #[tokio::test]
    async fn apply_status_broadcasts_event() {
        let store = FleetStore::in_memory().unwrap();
        let mut rx = store.subscribe();
        store.apply_status(update("x", FleetStatus::Working)).unwrap();
        let ev = rx.try_recv().expect("event delivered");
        assert_eq!(ev.kind, FleetEventKind::Added);
        assert_eq!(ev.member.id, "x");
    }

    #[test]
    fn persists_across_reopen() {
        let dir =
            std::env::temp_dir().join(format!("mirai-fleet-persist-{}", crate::utils::short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fleet.db");
        let p = path.to_str().unwrap();
        {
            let store = FleetStore::open(p).unwrap();
            store.apply_status(update("survivor", FleetStatus::Online)).unwrap();
        }
        // Reopen — the SoT survived the drop (WAL checkpointed on close).
        let store = FleetStore::open(p).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.get("survivor").unwrap().unwrap().status, FleetStatus::Online);
        let _ = std::fs::remove_dir_all(dir);
    }
}
