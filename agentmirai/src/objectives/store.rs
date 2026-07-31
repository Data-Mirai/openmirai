//! ObjectiveStore — SQLite (WAL) source of truth for fleet objectives.
//!
//! Persists every objective in the `objectives` table and its links to fleet
//! members in the `objective_agents` bridge table. Opened in WAL mode (REGLA-504,
//! same as [`crate::fleet::FleetStore`]) so readers never block a writer; it
//! lives right next to the fleet (same `fleet.db` file) so an objective and the
//! agents working it share one durable store.
//!
//! rusqlite is synchronous; every operation is a single fast statement, so the
//! connection is guarded by a `std::sync::Mutex` and called directly (the lock is
//! never held across an `.await`).

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use thiserror::Error;

use openmirai_engine::utils::{now_epoch, short_id};

use super::types::{NewObjective, Objective, ObjectiveStatus};

/// DDL for the objective SoT + the objective↔agent bridge. `IF NOT EXISTS` keeps
/// it idempotent across restarts and safe to run beside the fleet schema in the
/// same DB file.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS objectives (
    id           TEXT PRIMARY KEY,
    text         TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'open',
    project_dir  TEXT NOT NULL DEFAULT '',
    created_at   REAL NOT NULL,
    updated_at   REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_objectives_status  ON objectives(status);
CREATE INDEX IF NOT EXISTS idx_objectives_created ON objectives(created_at DESC);

-- Bridge: which fleet members work an objective. (objective_id, fleet_id) is the
-- idempotent link key so re-linking the same pair is a no-op.
CREATE TABLE IF NOT EXISTS objective_agents (
    objective_id TEXT NOT NULL,
    fleet_id     TEXT NOT NULL,
    linked_at    REAL NOT NULL,
    PRIMARY KEY (objective_id, fleet_id)
);

CREATE INDEX IF NOT EXISTS idx_obj_agents_fleet ON objective_agents(fleet_id);
"#;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ObjectiveError {
    #[error("objective db error: {0}")]
    Db(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("not found: {0}")]
    NotFound(String),
}

impl From<rusqlite::Error> for ObjectiveError {
    fn from(e: rusqlite::Error) -> Self {
        ObjectiveError::Db(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// ObjectiveStore
// ---------------------------------------------------------------------------

/// SQLite-backed source of truth for objectives + their agent links.
///
/// `Mutex<Connection>` is `Sync` because `Connection` is `Send`, so the store is
/// `Send + Sync` and can live in the shared axum state without any `unsafe`.
pub struct ObjectiveStore {
    conn: Mutex<Connection>,
}

impl ObjectiveStore {
    /// Open (or create) the objective DB at `path` in WAL mode and ensure the
    /// schema exists. Pass `":memory:"` for a throwaway store. Safe to point at
    /// the same file as the fleet store — the tables are disjoint.
    pub fn open(path: &str) -> Result<Self, ObjectiveError> {
        let conn = Connection::open(path)
            .map_err(|e| ObjectiveError::Db(format!("failed to open objective db at {path}: {e}")))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;",
        )
        .map_err(|e| ObjectiveError::Db(format!("failed to set PRAGMAs: {e}")))?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| ObjectiveError::Db(format!("failed to init objective schema: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store — the default for tests and for state construction before
    /// `serve()` swaps in a file-backed one.
    pub fn in_memory() -> Result<Self, ObjectiveError> {
        Self::open(":memory:")
    }

    /// Default on-disk location for the live server — the same `~/.openmirai/`
    /// dir as the fleet, so objectives sit right next to the flota.
    pub fn default_path() -> std::path::PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::Path::new(&home)
            .join(".openmirai")
            .join("fleet.db")
    }

    /// Create a new objective (server-assigned id + timestamps), optionally
    /// linking the given fleet member ids in the same call.
    pub fn create(&self, new: NewObjective) -> Result<Objective, ObjectiveError> {
        let text = new.text.trim();
        if text.is_empty() {
            return Err(ObjectiveError::Invalid("text is required".into()));
        }
        let now = now_epoch();
        let objective = Objective {
            id: short_id(),
            text: text.to_string(),
            status: new.status,
            project_dir: new.project_dir,
            created_at: now,
            updated_at: now,
        };

        let conn = self.conn.lock().expect("objective mutex poisoned");
        conn.execute(
            "INSERT INTO objectives (id, text, status, project_dir, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                objective.id,
                objective.text,
                objective.status.as_str(),
                objective.project_dir,
                objective.created_at,
                objective.updated_at,
            ],
        )?;
        for fleet_id in &new.agents {
            let fleet_id = fleet_id.trim();
            if !fleet_id.is_empty() {
                conn.execute(
                    "INSERT OR IGNORE INTO objective_agents (objective_id, fleet_id, linked_at) \
                     VALUES (?1, ?2, ?3)",
                    params![objective.id, fleet_id, now],
                )?;
            }
        }
        Ok(objective)
    }

    /// Fetch a single objective by id.
    pub fn get(&self, id: &str) -> Result<Option<Objective>, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let objective = conn
            .query_row(
                "SELECT id, text, status, project_dir, created_at, updated_at \
                 FROM objectives WHERE id = ?1",
                params![id],
                row_to_objective,
            )
            .optional()?;
        objective.transpose()
    }

    /// List objectives, newest first.
    pub fn list(&self) -> Result<Vec<Objective>, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, text, status, project_dir, created_at, updated_at \
             FROM objectives ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_objective)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Update an objective's status (stamping `updated_at`). Returns the updated
    /// record, or [`ObjectiveError::NotFound`] if the id does not exist.
    pub fn set_status(
        &self,
        id: &str,
        status: ObjectiveStatus,
    ) -> Result<Objective, ObjectiveError> {
        let now = now_epoch();
        {
            let conn = self.conn.lock().expect("objective mutex poisoned");
            let n = conn.execute(
                "UPDATE objectives SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, id],
            )?;
            if n == 0 {
                return Err(ObjectiveError::NotFound(format!("objective '{id}'")));
            }
        }
        // Re-read so the caller gets the canonical row (single source of truth).
        self.get(id)?
            .ok_or_else(|| ObjectiveError::NotFound(format!("objective '{id}'")))
    }

    /// Link a fleet member to an objective (idempotent). Returns
    /// [`ObjectiveError::NotFound`] if the objective does not exist, `true` if a
    /// new link was created, `false` if the pair was already linked.
    pub fn link_agent(&self, objective_id: &str, fleet_id: &str) -> Result<bool, ObjectiveError> {
        let fleet_id = fleet_id.trim();
        if fleet_id.is_empty() {
            return Err(ObjectiveError::Invalid("fleet_id is required".into()));
        }
        if self.get(objective_id)?.is_none() {
            return Err(ObjectiveError::NotFound(format!(
                "objective '{objective_id}'"
            )));
        }
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let n = conn.execute(
            "INSERT OR IGNORE INTO objective_agents (objective_id, fleet_id, linked_at) \
             VALUES (?1, ?2, ?3)",
            params![objective_id, fleet_id, now_epoch()],
        )?;
        Ok(n > 0)
    }

    /// Remove an objective↔agent link. Returns `true` if a link was removed.
    pub fn unlink_agent(&self, objective_id: &str, fleet_id: &str) -> Result<bool, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let n = conn.execute(
            "DELETE FROM objective_agents WHERE objective_id = ?1 AND fleet_id = ?2",
            params![objective_id, fleet_id],
        )?;
        Ok(n > 0)
    }

    /// The fleet ids linked to an objective, in link order (oldest first).
    pub fn agent_ids(&self, objective_id: &str) -> Result<Vec<String>, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT fleet_id FROM objective_agents WHERE objective_id = ?1 ORDER BY linked_at ASC",
        )?;
        let rows = stmt.query_map(params![objective_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Total number of objectives in the SoT.
    pub fn count(&self) -> Result<usize, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM objectives", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// The connection's active `journal_mode` — used by tests to prove WAL.
    pub fn journal_mode(&self) -> Result<String, ObjectiveError> {
        let conn = self.conn.lock().expect("objective mutex poisoned");
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        Ok(mode)
    }
}

/// Map a DB row to an [`Objective`]. Returns an inner `Result` so a malformed
/// `status` surfaces as an `ObjectiveError` rather than a panic.
fn row_to_objective(row: &Row<'_>) -> rusqlite::Result<Result<Objective, ObjectiveError>> {
    let id: String = row.get(0)?;
    let text: String = row.get(1)?;
    let status_raw: String = row.get(2)?;
    let project_dir: String = row.get(3)?;
    let created_at: f64 = row.get(4)?;
    let updated_at: f64 = row.get(5)?;

    Ok((|| {
        let status = ObjectiveStatus::parse(&status_raw)
            .ok_or_else(|| ObjectiveError::Db(format!("bad status '{status_raw}'")))?;
        Ok(Objective {
            id,
            text,
            status,
            project_dir,
            created_at,
            updated_at,
        })
    })())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn new(text: &str) -> NewObjective {
        NewObjective {
            text: text.to_string(),
            project_dir: String::new(),
            status: ObjectiveStatus::Open,
            agents: Vec::new(),
        }
    }

    #[test]
    fn open_enables_wal_on_disk() {
        let dir = std::env::temp_dir().join(format!("mirai-obj-wal-{}", short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("obj.db");
        let store = ObjectiveStore::open(path.to_str().unwrap()).unwrap();
        assert_eq!(store.journal_mode().unwrap().to_lowercase(), "wal");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_assigns_id_and_defaults() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store.create(new("ship the SoT")).unwrap();
        assert!(!obj.id.is_empty());
        assert_eq!(obj.text, "ship the SoT");
        assert_eq!(obj.status, ObjectiveStatus::Open);
        assert_eq!(obj.created_at, obj.updated_at);

        // GET returns it back.
        let got = store.get(&obj.id).unwrap().unwrap();
        assert_eq!(got, obj);
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn create_trims_and_rejects_empty_text() {
        let store = ObjectiveStore::in_memory().unwrap();
        let err = store.create(new("   "));
        assert!(matches!(err, Err(ObjectiveError::Invalid(_))));
    }

    #[test]
    fn create_with_project_dir_and_status() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store
            .create(NewObjective {
                text: "x".into(),
                project_dir: "/proj".into(),
                status: ObjectiveStatus::Working,
                agents: Vec::new(),
            })
            .unwrap();
        assert_eq!(obj.project_dir, "/proj");
        assert_eq!(obj.status, ObjectiveStatus::Working);
    }

    #[test]
    fn list_orders_newest_first() {
        let store = ObjectiveStore::in_memory().unwrap();
        let a = store.create(new("first")).unwrap();
        let b = store.create(new("second")).unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        // created_at DESC — b (later) first. Ties are possible on a fast clock,
        // so only assert both are present.
        let ids: Vec<&str> = all.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&a.id.as_str()) && ids.contains(&b.id.as_str()));
    }

    #[test]
    fn set_status_updates_and_stamps() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store.create(new("goal")).unwrap();
        let updated = store.set_status(&obj.id, ObjectiveStatus::Done).unwrap();
        assert_eq!(updated.status, ObjectiveStatus::Done);
        assert!(updated.updated_at >= obj.updated_at);
        assert_eq!(updated.created_at, obj.created_at, "created never moves");
    }

    #[test]
    fn set_status_missing_is_not_found() {
        let store = ObjectiveStore::in_memory().unwrap();
        let err = store.set_status("nope", ObjectiveStatus::Done);
        assert!(matches!(err, Err(ObjectiveError::NotFound(_))));
    }

    #[test]
    fn link_and_list_agents_is_idempotent() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store.create(new("goal")).unwrap();
        assert!(store.link_agent(&obj.id, "agent-a").unwrap(), "new link");
        assert!(!store.link_agent(&obj.id, "agent-a").unwrap(), "re-link no-op");
        store.link_agent(&obj.id, "agent-b").unwrap();

        let ids = store.agent_ids(&obj.id).unwrap();
        assert_eq!(ids, vec!["agent-a".to_string(), "agent-b".to_string()]);
    }

    #[test]
    fn link_requires_existing_objective_and_nonempty_id() {
        let store = ObjectiveStore::in_memory().unwrap();
        assert!(matches!(
            store.link_agent("ghost", "a"),
            Err(ObjectiveError::NotFound(_))
        ));
        let obj = store.create(new("goal")).unwrap();
        assert!(matches!(
            store.link_agent(&obj.id, "  "),
            Err(ObjectiveError::Invalid(_))
        ));
    }

    #[test]
    fn create_links_agents_in_one_call() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store
            .create(NewObjective {
                text: "goal".into(),
                project_dir: String::new(),
                status: ObjectiveStatus::Open,
                agents: vec!["a".into(), "b".into(), "  ".into()],
            })
            .unwrap();
        // Blank ids are skipped.
        assert_eq!(store.agent_ids(&obj.id).unwrap(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn unlink_removes_link() {
        let store = ObjectiveStore::in_memory().unwrap();
        let obj = store.create(new("goal")).unwrap();
        store.link_agent(&obj.id, "a").unwrap();
        assert!(store.unlink_agent(&obj.id, "a").unwrap());
        assert!(!store.unlink_agent(&obj.id, "a").unwrap(), "second unlink no-op");
        assert!(store.agent_ids(&obj.id).unwrap().is_empty());
    }

    #[test]
    fn persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("mirai-obj-persist-{}", short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("obj.db");
        let p = path.to_str().unwrap();
        let id = {
            let store = ObjectiveStore::open(p).unwrap();
            let obj = store.create(new("survivor")).unwrap();
            store.link_agent(&obj.id, "a").unwrap();
            obj.id
        };
        let store = ObjectiveStore::open(p).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.get(&id).unwrap().unwrap().text, "survivor");
        assert_eq!(store.agent_ids(&id).unwrap(), vec!["a".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn coexists_with_fleet_table_in_same_db() {
        // The objective store must be safe to open on the SAME file the fleet
        // store manages — disjoint tables, both WAL.
        let dir = std::env::temp_dir().join(format!("mirai-obj-coexist-{}", short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fleet.db");
        let p = path.to_str().unwrap();

        let fleet = crate::fleet::FleetStore::open(p).unwrap();
        let objectives = ObjectiveStore::open(p).unwrap();

        fleet
            .apply_status(crate::fleet::StatusUpdate {
                id: "agent-a".into(),
                status: crate::fleet::FleetStatus::Working,
                name: None,
                kind: None,
                host: None,
                activity: None,
                objective: None,
                parent_id: None,
                project_dir: None,
                metadata: None,
            })
            .unwrap();
        let obj = objectives.create(new("goal")).unwrap();
        objectives.link_agent(&obj.id, "agent-a").unwrap();

        assert_eq!(fleet.count().unwrap(), 1);
        assert_eq!(objectives.count().unwrap(), 1);
        assert_eq!(objectives.agent_ids(&obj.id).unwrap(), vec!["agent-a".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
