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
pub const SCHEMA_VERSION: u32 = 1;

/// Complete DDL for schema v1.  Using `IF NOT EXISTS` makes it idempotent.
pub const SCHEMA_SQL: &str = r#"
-- Open Mirai — Standalone Schema v1

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
// Migration registry
// ---------------------------------------------------------------------------

/// Ordered list of all migrations.  For now there is only the initial schema.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description: "Initial standalone engine schema (graphs, agents, sessions)",
    up_sql: SCHEMA_SQL,
}];

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
