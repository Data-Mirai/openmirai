//! Schema version tracking and migration definitions.
//!
//! Mirrors the Python `db/migrations.py`: defines the DDL for the standalone
//! engine schema (graphs, agents, sessions, schema-version tracking).  The SQL
//! strings are backend-agnostic *constants* -- actual execution is delegated to
//! whatever database driver the caller chooses.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Tracks which schema version has been applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub version: u32,
    pub applied_at: f64,
}

/// A single forward migration.
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub up_sql: &'static str,
}

// ---------------------------------------------------------------------------
// Schema v1 — full DDL
// ---------------------------------------------------------------------------

/// Current schema version shipped with this build.
pub const SCHEMA_VERSION: u32 = 3;

/// Complete DDL for schema v1.  Using `IF NOT EXISTS` makes it idempotent.
pub const SCHEMA_SQL: &str = r#"
-- OpenMirai — Standalone Schema v1

-- Graphs
CREATE TABLE IF NOT EXISTS graphs (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    version     TEXT NOT NULL DEFAULT '1.0',
    nodes       TEXT NOT NULL DEFAULT '[]',
    edges       TEXT NOT NULL DEFAULT '[]',
    metadata    TEXT NOT NULL DEFAULT '{}',
    created_at  REAL NOT NULL,
    updated_at  REAL NOT NULL
);

-- Agents
CREATE TABLE IF NOT EXISTS agents (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    graph_id    TEXT NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'disabled',
    triggers    TEXT NOT NULL DEFAULT '[]',
    metadata    TEXT NOT NULL DEFAULT '{}',
    created_at  REAL NOT NULL,
    updated_at  REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);

-- Sessions
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    agent_name  TEXT NOT NULL,
    graph_id    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running',
    trace       TEXT NOT NULL DEFAULT '[]',
    transcript  TEXT NOT NULL DEFAULT '[]',
    state       TEXT NOT NULL DEFAULT '{}',
    error       TEXT,
    started_at  REAL NOT NULL,
    finished_at REAL,
    duration_ms REAL
);

CREATE INDEX IF NOT EXISTS idx_sessions_agent ON sessions(agent_id);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

-- Schema version tracking
CREATE TABLE IF NOT EXISTS _schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  REAL NOT NULL
);

INSERT OR IGNORE INTO _schema_version (version, applied_at)
VALUES (1, 0.0);
"#;

// ---------------------------------------------------------------------------
// Schema v2 — agent specs
// ---------------------------------------------------------------------------

/// DDL for schema v2.
///
/// The `agents` table from v1 models an agent as a *reference* to a row in
/// `graphs`. The HTTP server does not work that way: `POST /agents/from-spec`
/// registers a whole [`crate::core::agent_spec::AgentSpec`], which carries its
/// own graph plus inputs, outputs, schedule, memory declaration and config.
/// Splitting that across `agents` + `graphs` would drop every field that has no
/// column, so specs get their own table and travel as JSON.
///
/// `playing` records whether the agent was cycling when the process went down,
/// so live agents can be rescheduled on startup.
pub const SCHEMA_SQL_V2: &str = r#"
-- OpenMirai — Standalone Schema v2

CREATE TABLE IF NOT EXISTS agent_specs (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    spec        TEXT NOT NULL,
    playing     INTEGER NOT NULL DEFAULT 0,
    created_at  REAL NOT NULL,
    updated_at  REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_specs_playing ON agent_specs(playing);

INSERT OR IGNORE INTO _schema_version (version, applied_at)
VALUES (2, 0.0);
"#;

// ---------------------------------------------------------------------------
// Schema v3 — agent memory
// ---------------------------------------------------------------------------

/// DDL for schema v3.
///
/// Agent memory (PRD-008) lived only in RAM, so a restart wiped it: a live agent
/// came back and kept cycling, but with its declared keys reset to their initial
/// values. This table is the durable side of it.
///
/// One row per (agent, scope) holding the whole key/value map as JSON, because
/// that is the unit the engine writes — `set_cycle_memory` / `set_execution_memory`
/// replace the map wholesale after filtering it to the declared keys.
///
/// `scope` mirrors the persist mode: `cycle` (kept within a play session) and
/// `execution` (kept across everything). `none` never reaches this table.
///
/// No foreign key to `agent_specs` on purpose: memory must not be what makes a
/// write fail, and the agent delete path clears these rows explicitly.
pub const SCHEMA_SQL_V3: &str = r#"
-- OpenMirai — Standalone Schema v3

CREATE TABLE IF NOT EXISTS agent_memory (
    agent_id    TEXT NOT NULL,
    scope       TEXT NOT NULL,
    memory      TEXT NOT NULL,
    updated_at  REAL NOT NULL,
    PRIMARY KEY (agent_id, scope)
);

INSERT OR IGNORE INTO _schema_version (version, applied_at)
VALUES (3, 0.0);
"#;

// ---------------------------------------------------------------------------
// Migration registry
// ---------------------------------------------------------------------------

/// Ordered list of all migrations. Every `up_sql` is idempotent (`IF NOT
/// EXISTS`), so applying the whole list on an existing database is a no-op.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial standalone engine schema (graphs, agents, sessions)",
        up_sql: SCHEMA_SQL,
    },
    Migration {
        version: 2,
        description: "Agent specs table for the HTTP server registry",
        up_sql: SCHEMA_SQL_V2,
    },
    Migration {
        version: 3,
        description: "Agent memory table (cycle + execution scopes)",
        up_sql: SCHEMA_SQL_V3,
    },
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_ordered() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1, "migration order mismatch");
        }
    }

    #[test]
    fn schema_version_matches_last_migration() {
        let last = MIGRATIONS.last().expect("no migrations defined");
        assert_eq!(last.version, SCHEMA_VERSION);
    }

    #[test]
    fn schema_sql_contains_tables() {
        assert!(SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS graphs"));
        assert!(SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS agents"));
        assert!(SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS sessions"));
        assert!(SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS _schema_version"));
    }

    #[test]
    fn schema_version_serde_roundtrip() {
        let sv = SchemaVersion {
            version: 1,
            applied_at: 1700000000.0,
        };
        let json = serde_json::to_string(&sv).unwrap();
        let back: SchemaVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, 1);
        assert!((back.applied_at - 1700000000.0).abs() < f64::EPSILON);
    }
}
