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
pub const SCHEMA_VERSION: u32 = 2;

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
// Schema v2 — checkpoints (PRD-021-A)
// ---------------------------------------------------------------------------

/// DDL de la v2: la tabla de checkpoints + el nodo actual del run.
///
/// **Aditiva y compatible**: no reescribe ni borra nada de la v1. Una DB
/// creada por la v1 se abre con este binario, recibe el `ALTER TABLE` y sus
/// filas siguen legibles (`current_node_id` queda `NULL`).
///
/// Un checkpoint por run (`session_id` es la PK): el estado del grafo se
/// **sobreescribe**, no se acumula historial — así el tamaño no crece con la
/// duración de la ejecución (riesgo del PRD).
///
/// Sin FK a `sessions`: el checkpoint se escribe *durante* la ejecución, y en
/// esa ventana la fila del run puede no existir todavía. Un checkpoint tiene
/// que poder sobrevivir por sí solo.
pub const SCHEMA_SQL_V2: &str = r#"
-- OpenMirai — Schema v2 (PRD-021-A): estado de ejecución reanudable

CREATE TABLE IF NOT EXISTS checkpoints (
    session_id      TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL DEFAULT '',
    step            INTEGER NOT NULL DEFAULT 0,
    node_id         TEXT NOT NULL DEFAULT '',
    cursor_node_id  TEXT,
    state_snapshot  TEXT NOT NULL DEFAULT '{}',
    executed_nodes  TEXT NOT NULL DEFAULT '[]',
    created_at      REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_checkpoints_agent ON checkpoints(agent_id);
CREATE INDEX IF NOT EXISTS idx_checkpoints_created ON checkpoints(created_at DESC);

-- Nodo en el que va (o quedó) el run. NULL para runs de la v1.
ALTER TABLE sessions ADD COLUMN current_node_id TEXT;

INSERT OR IGNORE INTO _schema_version (version, applied_at)
VALUES (2, 0.0);
"#;

// ---------------------------------------------------------------------------
// Migration registry
// ---------------------------------------------------------------------------

/// Lista ordenada de migraciones. El runner (`db/sqlite.rs`) aplica solo las
/// de versión mayor a la registrada en `_schema_version`: por eso una
/// migración puede tener DDL no idempotente (`ALTER TABLE`) sin romper al
/// reabrir la misma DB.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial standalone engine schema (graphs, agents, sessions)",
        up_sql: SCHEMA_SQL,
    },
    Migration {
        version: 2,
        description: "Checkpoints reanudables + nodo actual del run (PRD-021-A)",
        up_sql: SCHEMA_SQL_V2,
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
    fn v2_is_additive_over_v1() {
        // La v2 no puede reescribir la v1: nada de DROP/RENAME sobre lo viejo.
        let up = SCHEMA_SQL_V2.to_uppercase();
        assert!(!up.contains("DROP TABLE"), "v2 no puede borrar tablas v1");
        assert!(
            !up.contains("DROP COLUMN"),
            "v2 no puede borrar columnas v1"
        );
        assert!(!up.contains("RENAME"), "v2 no puede renombrar lo de v1");
        assert!(SCHEMA_SQL_V2.contains("CREATE TABLE IF NOT EXISTS checkpoints"));
        assert!(SCHEMA_SQL_V2.contains("ALTER TABLE sessions ADD COLUMN current_node_id"));
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
