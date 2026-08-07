//! SqliteSessionRepo — persistencia real de runs (sessions) sobre SQLite/WAL.
//!
//! Implementa `Repository<SessionRecord>` contra el schema v1 (`db/migrations.rs`),
//! que existía definido pero sin cablear: las ejecuciones vivían solo en memoria
//! y morían con el proceso. Con esto los runs sobreviven reinicios — la base de
//! la trazabilidad (qué corrió, cuándo, nodo por nodo, con qué resultado).
//!
//! Diseño:
//! - rusqlite + WAL + busy_timeout, llamadas síncronas envueltas en
//!   `spawn_blocking` (calca el patrón de `adapters/sqlite_db.rs`).
//! - El schema tiene FKs `sessions → agents → graphs`: al guardar un run se hace
//!   `INSERT OR IGNORE` de las filas padre — la DB acumula además qué agentes y
//!   grafos corrieron.
//! - `ExecutionResult` round-tripea por serde: las columnas (`trace`,
//!   `transcript`, `state`, `error`) guardan su forma serde y `get()` rearma el
//!   resultado exacto.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::core::runner::ExecutionResult;
use crate::utils::now_epoch;

use super::checkpoints::{CheckpointRecord, RunProgress};
use super::migrations::MIGRATIONS;
use super::repositories::{DbError, Repository, SessionRecord, SessionStatus};

/// Repositorio de sesiones (runs) persistente sobre SQLite.
pub struct SqliteSessionRepo {
    conn: Arc<Mutex<Connection>>,
}

// rusqlite::Connection es Send pero no Sync; el Mutex lo hace Send + Sync
// (mismo razonamiento que SqliteDBResource).
unsafe impl Send for SqliteSessionRepo {}
unsafe impl Sync for SqliteSessionRepo {}

impl SqliteSessionRepo {
    /// Abre (o crea) la DB en `path`, aplica migraciones y deja WAL activo.
    /// Crea el directorio padre si no existe.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    DbError::ConnectionError(format!(
                        "cannot create db dir {}: {e}",
                        parent.display()
                    ))
                })?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|e| DbError::ConnectionError(format!("failed to open SQLite: {e}")))?;
        Self::init(conn)
    }

    /// DB en memoria (tests).
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| DbError::ConnectionError(format!("failed to open :memory:: {e}")))?;
        Self::init(conn)
    }

    fn init(mut conn: Connection) -> Result<Self, DbError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=10000; PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| DbError::ConnectionError(format!("failed to set PRAGMAs: {e}")))?;

        // Migración VERSIONADA (antes se re-ejecutaba todo el DDL en cada
        // apertura: funcionaba solo porque la v1 era 100% `IF NOT EXISTS`).
        // Con la v2 aparece `ALTER TABLE`, que NO es idempotente: reaplicarlo
        // revienta con "duplicate column name". Por eso ahora se lee la
        // versión aplicada y solo se corren las migraciones más nuevas, cada
        // una en su transacción (o entra completa, o no entra).
        let current = Self::applied_version(&conn)?;
        for m in MIGRATIONS {
            if m.version <= current {
                continue;
            }
            let tx = conn.transaction().map_err(|e| {
                DbError::ConnectionError(format!("migration v{}: begin: {e}", m.version))
            })?;
            tx.execute_batch(m.up_sql)
                .map_err(|e| DbError::ConnectionError(format!("migration v{}: {e}", m.version)))?;
            tx.commit().map_err(|e| {
                DbError::ConnectionError(format!("migration v{}: commit: {e}", m.version))
            })?;
        }

        // Sella el timestamp real de aplicación (el DDL inserta 0.0 idempotente).
        conn.execute(
            "UPDATE _schema_version SET applied_at = ?1 WHERE applied_at = 0.0",
            params![now_epoch()],
        )
        .map_err(|e| DbError::ConnectionError(format!("schema version stamp: {e}")))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Versión de esquema ya aplicada. `0` si la DB está vacía (sin
    /// `_schema_version`), que es lo que dispara la migración inicial.
    fn applied_version(conn: &Connection) -> Result<u32, DbError> {
        let has_table: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = '_schema_version'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| DbError::ConnectionError(format!("schema probe: {e}")))?;
        if !has_table {
            return Ok(0);
        }
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v as u32)
        .map_err(|e| DbError::ConnectionError(format!("schema version read: {e}")))
    }

    /// Versión de esquema de esta DB (para diagnóstico y tests de migración).
    pub async fn schema_version(&self) -> Result<u32, DbError> {
        self.with_conn(Self::applied_version).await
    }

    fn with_conn<T, F>(&self, f: F) -> impl std::future::Future<Output = Result<T, DbError>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, DbError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        async move {
            tokio::task::spawn_blocking(move || {
                let conn = conn
                    .lock()
                    .map_err(|e| DbError::ConnectionError(format!("mutex poisoned: {e}")))?;
                f(&conn)
            })
            .await
            .map_err(|e| DbError::ConnectionError(format!("spawn_blocking join: {e}")))?
        }
    }

    /// Lista runs (opcionalmente por agente), ordenados por inicio descendente.
    pub async fn list_by_agent(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, DbError> {
        let agent_id = agent_id.map(str::to_string);
        self.with_conn(move |conn| {
            let (sql, args): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &agent_id {
                Some(aid) => (
                    "SELECT * FROM sessions WHERE agent_id = ?1
                     ORDER BY started_at DESC LIMIT ?2",
                    vec![Box::new(aid.clone()), Box::new(limit as i64)],
                ),
                None => (
                    "SELECT * FROM sessions ORDER BY started_at DESC LIMIT ?1",
                    vec![Box::new(limit as i64)],
                ),
            };
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| DbError::ConnectionError(format!("prepare: {e}")))?;
            let arg_refs: Vec<&dyn rusqlite::types::ToSql> =
                args.iter().map(|b| b.as_ref()).collect();
            let mut rows = stmt
                .query(arg_refs.as_slice())
                .map_err(|e| DbError::ConnectionError(format!("query: {e}")))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| DbError::ConnectionError(format!("next: {e}")))?
            {
                out.push(row_to_record(row)?);
            }
            Ok(out)
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Checkpoints (PRD-021-A)
// ---------------------------------------------------------------------------

impl SqliteSessionRepo {
    /// Guarda (o sobreescribe) el checkpoint del run **y** deja la fila del run
    /// al día con su estado y nodo actual — las dos escrituras en UNA
    /// transacción: un run nunca queda marcado en un nodo que su checkpoint no
    /// respalda.
    ///
    /// La fila del run se **upserta**: si todavía no existe (lo normal, se
    /// registra al terminar) se crea con `started_at` y el estado en vuelo; si
    /// ya existe y sigue viva (`running`/`paused`) se le actualizan estado,
    /// nodo actual y estado compartido. Un run ya terminado NO se revive.
    pub async fn save_checkpoint(
        &self,
        cp: &CheckpointRecord,
        run: &RunProgress,
    ) -> Result<(), DbError> {
        let cp = cp.clone();
        let run = run.clone();
        self.with_conn(move |conn| {
            let ser = |e: serde_json::Error| DbError::SerializationError(e.to_string());
            let sql = |e: rusqlite::Error| DbError::ConnectionError(format!("checkpoint: {e}"));

            let snapshot_json = serde_json::to_string(&cp.state_snapshot).map_err(ser)?;
            let executed_json = serde_json::to_string(&cp.executed_nodes).map_err(ser)?;

            conn.execute("BEGIN IMMEDIATE", []).map_err(sql)?;
            let out = (|| -> Result<(), DbError> {
                conn.execute(
                    "INSERT OR REPLACE INTO checkpoints
                       (session_id, agent_id, step, node_id, cursor_node_id,
                        state_snapshot, executed_nodes, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        cp.session_id,
                        cp.agent_id,
                        cp.step,
                        cp.node_id,
                        cp.cursor_node_id,
                        snapshot_json,
                        executed_json,
                        cp.created_at,
                    ],
                )
                .map_err(sql)?;

                let graph_id = ensure_parent_rows(conn, &cp.agent_id, &run.agent_name, "")?;
                conn.execute(
                    "INSERT INTO sessions
                       (id, agent_id, agent_name, graph_id, status, state,
                        started_at, current_node_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                        status          = excluded.status,
                        state           = excluded.state,
                        current_node_id = excluded.current_node_id
                     WHERE sessions.status IN ('running', 'paused')",
                    params![
                        cp.session_id,
                        cp.agent_id,
                        run.agent_name,
                        graph_id,
                        run.status.to_string(),
                        snapshot_json,
                        run.started_at,
                        cp.resume_node_id(),
                    ],
                )
                .map_err(sql)?;
                Ok(())
            })();
            match out {
                Ok(()) => {
                    conn.execute("COMMIT", []).map_err(sql)?;
                    Ok(())
                }
                Err(e) => {
                    let _ = conn.execute("ROLLBACK", []);
                    Err(e)
                }
            }
        })
        .await
    }

    /// Lee el checkpoint de un run. `None` si nunca se guardó uno.
    pub async fn get_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<CheckpointRecord>, DbError> {
        let session_id = session_id.to_string();
        self.with_conn(move |conn| {
            conn.query_row(
                "SELECT * FROM checkpoints WHERE session_id = ?1",
                params![session_id],
                |row| Ok(row_to_checkpoint(row)),
            )
            .optional()
            .map_err(|e| DbError::ConnectionError(format!("get_checkpoint: {e}")))?
            .transpose()
        })
        .await
    }

    /// Borra el checkpoint de un run (al completarlo ya no hay qué reanudar).
    pub async fn delete_checkpoint(&self, session_id: &str) -> Result<bool, DbError> {
        let session_id = session_id.to_string();
        self.with_conn(move |conn| {
            let n = conn
                .execute(
                    "DELETE FROM checkpoints WHERE session_id = ?1",
                    params![session_id],
                )
                .map_err(|e| DbError::ConnectionError(format!("delete_checkpoint: {e}")))?;
            Ok(n > 0)
        })
        .await
    }
}

/// Rearma un `CheckpointRecord` desde una fila de `checkpoints`.
fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> Result<CheckpointRecord, DbError> {
    let ser = |e: serde_json::Error| DbError::SerializationError(e.to_string());
    let col = |e: rusqlite::Error| DbError::ConnectionError(format!("column: {e}"));

    let snapshot_txt: String = row.get("state_snapshot").map_err(col)?;
    let executed_txt: String = row.get("executed_nodes").map_err(col)?;
    Ok(CheckpointRecord {
        session_id: row.get("session_id").map_err(col)?,
        agent_id: row.get("agent_id").map_err(col)?,
        step: row.get::<_, i64>("step").map_err(col)? as u32,
        node_id: row.get("node_id").map_err(col)?,
        cursor_node_id: row.get("cursor_node_id").map_err(col)?,
        state_snapshot: serde_json::from_str(&snapshot_txt).map_err(ser)?,
        executed_nodes: serde_json::from_str(&executed_txt).map_err(ser)?,
        created_at: row.get("created_at").map_err(col)?,
    })
}

/// Inserta las filas padre (`graphs`, `agents`) que exigen las FKs de
/// `sessions`. Devuelve el `graph_id` efectivo. Idempotente.
fn ensure_parent_rows(
    conn: &Connection,
    agent_id: &str,
    agent_name: &str,
    graph_id: &str,
) -> Result<String, DbError> {
    let sql = |e: rusqlite::Error| DbError::ConnectionError(format!("parent rows: {e}"));
    let now = now_epoch();
    let graph_id = if graph_id.is_empty() {
        format!("graph-{agent_id}")
    } else {
        graph_id.to_string()
    };
    conn.execute(
        "INSERT OR IGNORE INTO graphs (id, name, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?2)",
        params![graph_id, now],
    )
    .map_err(sql)?;
    conn.execute(
        "INSERT OR IGNORE INTO agents (id, name, graph_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![agent_id, agent_name, graph_id, now],
    )
    .map_err(sql)?;
    Ok(graph_id)
}

/// Rearma un `SessionRecord` desde una fila de `sessions`.
fn row_to_record(row: &rusqlite::Row<'_>) -> Result<SessionRecord, DbError> {
    let ser = |e: serde_json::Error| DbError::SerializationError(e.to_string());
    let col = |e: rusqlite::Error| DbError::ConnectionError(format!("column: {e}"));

    let id: String = row.get("id").map_err(col)?;
    let agent_id: String = row.get("agent_id").map_err(col)?;
    let agent_name: String = row.get("agent_name").map_err(col)?;
    let graph_id: String = row.get("graph_id").map_err(col)?;
    let status_txt: String = row.get("status").map_err(col)?;
    let trace_txt: String = row.get("trace").map_err(col)?;
    let transcript_txt: String = row.get("transcript").map_err(col)?;
    let state_txt: String = row.get("state").map_err(col)?;
    let error: Option<String> = row.get("error").map_err(col)?;
    let started_at: f64 = row.get("started_at").map_err(col)?;
    let finished_at: Option<f64> = row.get("finished_at").map_err(col)?;
    let duration_ms: Option<f64> = row.get("duration_ms").map_err(col)?;
    // v2: NULL en runs escritos por la v1.
    let current_node_id: Option<String> = row.get("current_node_id").map_err(col)?;

    let status = SessionStatus::parse(&status_txt);

    // ExecutionResult round-tripea por serde: se rearma desde las columnas.
    let result_json = serde_json::json!({
        "status": status.to_execution(),
        "state": serde_json::from_str::<Value>(&state_txt).map_err(ser)?,
        "trace": serde_json::from_str::<Value>(&trace_txt).map_err(ser)?,
        "transcript": serde_json::from_str::<Value>(&transcript_txt).map_err(ser)?,
        "error": error,
    });
    let result: ExecutionResult = serde_json::from_value(result_json).map_err(ser)?;

    Ok(SessionRecord {
        id,
        agent_id,
        agent_name,
        graph_id,
        result,
        created_at: started_at,
        finished_at,
        duration_ms,
        status,
        current_node_id,
    })
}

#[async_trait]
impl Repository<SessionRecord> for SqliteSessionRepo {
    async fn save(&self, item: &SessionRecord) -> Result<(), DbError> {
        let rec = item.clone();
        self.with_conn(move |conn| {
            let ser = |e: serde_json::Error| DbError::SerializationError(e.to_string());
            let sql = |e: rusqlite::Error| DbError::ConnectionError(format!("save: {e}"));

            // El grafo puede no estar registrado (agentes cargados de YAML):
            // filas padre mínimas para satisfacer las FKs del schema.
            let graph_id = ensure_parent_rows(conn, &rec.agent_id, &rec.agent_name, &rec.graph_id)?;

            let state_json = serde_json::to_string(&rec.result.state).map_err(ser)?;
            let trace_json = serde_json::to_string(&rec.result.trace).map_err(ser)?;
            let transcript_json = serde_json::to_string(&rec.result.transcript).map_err(ser)?;

            conn.execute(
                "INSERT OR REPLACE INTO sessions
                   (id, agent_id, agent_name, graph_id, status, trace, transcript,
                    state, error, started_at, finished_at, duration_ms, current_node_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    rec.id,
                    rec.agent_id,
                    rec.agent_name,
                    graph_id,
                    rec.status.to_string(),
                    trace_json,
                    transcript_json,
                    state_json,
                    rec.result.error,
                    rec.created_at,
                    rec.finished_at,
                    rec.duration_ms,
                    rec.current_node_id,
                ],
            )
            .map_err(sql)?;
            Ok(())
        })
        .await
    }

    async fn get(&self, id: &str) -> Result<Option<SessionRecord>, DbError> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            conn.query_row("SELECT * FROM sessions WHERE id = ?1", params![id], |row| {
                Ok(row_to_record(row))
            })
            .optional()
            .map_err(|e| DbError::ConnectionError(format!("get: {e}")))?
            .transpose()
        })
        .await
    }

    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<SessionRecord>, DbError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM sessions ORDER BY started_at DESC LIMIT ?1 OFFSET ?2")
                .map_err(|e| DbError::ConnectionError(format!("prepare: {e}")))?;
            let mut rows = stmt
                .query(params![limit as i64, offset as i64])
                .map_err(|e| DbError::ConnectionError(format!("query: {e}")))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| DbError::ConnectionError(format!("next: {e}")))?
            {
                out.push(row_to_record(row)?);
            }
            Ok(out)
        })
        .await
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let n = conn
                .execute("DELETE FROM sessions WHERE id = ?1", params![id])
                .map_err(|e| DbError::ConnectionError(format!("delete: {e}")))?;
            Ok(n > 0)
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runner::{
        ExecutionResult, ExecutionStatus, TraceEntry, TraceStatus, TranscriptEntry,
    };
    use crate::core::state::SharedState;
    use std::collections::HashMap;

    fn make_result(status: ExecutionStatus, error: Option<&str>) -> ExecutionResult {
        let state = SharedState::new();
        let mut out = HashMap::new();
        out.insert("response".to_string(), serde_json::json!("hola"));
        state.set("n1", out, true).unwrap();
        ExecutionResult {
            status,
            state,
            trace: vec![TraceEntry {
                node_id: "n1".into(),
                tool_type: "ai/llm_call".into(),
                status: TraceStatus::Ok,
                duration_ms: 120,
                retries: 0,
                started_at: 1700000000.0,
                finished_at: 1700000000.12,
                error: None,
            }],
            transcript: vec![TranscriptEntry {
                entry_type: "info".into(),
                message: "node done".into(),
                timestamp: 1700000000.12,
                node_id: Some("n1".into()),
                metadata: HashMap::new(),
            }],
            error: error.map(str::to_string),
            interrupt_node_id: None,
            interrupt_info: None,
        }
    }

    fn make_record(id: &str, agent_id: &str, created_at: f64) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            agent_id: agent_id.into(),
            agent_name: format!("Agente {agent_id}"),
            graph_id: String::new(),
            result: make_result(ExecutionStatus::Completed, None),
            created_at,
            finished_at: Some(created_at + 0.12),
            duration_ms: Some(120.0),
            status: SessionStatus::Completed,
            current_node_id: None,
        }
    }

    #[tokio::test]
    async fn save_and_get_roundtrip_exact() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        let rec = make_record("s1", "a1", 1700000000.0);
        repo.save(&rec).await.unwrap();

        let got = repo.get("s1").await.unwrap().expect("run persistido");
        assert_eq!(got.id, "s1");
        assert_eq!(got.agent_id, "a1");
        assert_eq!(got.agent_name, "Agente a1");
        assert_eq!(got.status, SessionStatus::Completed);
        assert_eq!(got.result.status, ExecutionStatus::Completed);
        // Timeline por nodo intacto — timestamps reales incluidos.
        assert_eq!(got.result.trace.len(), 1);
        let t = &got.result.trace[0];
        assert_eq!(t.node_id, "n1");
        assert_eq!(t.duration_ms, 120);
        assert!((t.started_at - 1700000000.0).abs() < 1e-6);
        assert!((t.finished_at - 1700000000.12).abs() < 1e-6);
        // Estado (outputs de nodos) intacto.
        let snap = got.result.state.snapshot();
        assert_eq!(snap["n1"]["response"], serde_json::json!("hola"));
        // Transcript intacto.
        assert_eq!(got.result.transcript.len(), 1);
        assert_eq!(got.finished_at, Some(1700000000.12));
    }

    #[tokio::test]
    async fn runs_survive_reopen() {
        // La prueba de fuego: cerrar la conexión y reabrir el MISMO archivo.
        let dir = std::env::temp_dir().join(format!("mirai-test-runs-{}", std::process::id()));
        let path = dir.join("engine-test.db");
        let _ = std::fs::remove_file(&path);

        {
            let repo = SqliteSessionRepo::open(&path).unwrap();
            repo.save(&make_record("s-persist", "a1", 1700000100.0))
                .await
                .unwrap();
        } // conexión cerrada (drop)

        let reopened = SqliteSessionRepo::open(&path).unwrap();
        let got = reopened.get("s-persist").await.unwrap();
        assert!(got.is_some(), "el run debe sobrevivir el reinicio");
        assert_eq!(got.unwrap().result.trace[0].node_id, "n1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_orders_by_started_at_desc_with_limit_offset() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        for (i, id) in ["viejo", "medio", "nuevo"].iter().enumerate() {
            repo.save(&make_record(id, "a1", 1700000000.0 + i as f64))
                .await
                .unwrap();
        }
        let all = repo.list(10, 0).await.unwrap();
        assert_eq!(
            all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["nuevo", "medio", "viejo"]
        );
        let paged = repo.list(1, 1).await.unwrap();
        assert_eq!(paged[0].id, "medio");
    }

    #[tokio::test]
    async fn list_by_agent_filters() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        repo.save(&make_record("s1", "a1", 1.0)).await.unwrap();
        repo.save(&make_record("s2", "a2", 2.0)).await.unwrap();
        repo.save(&make_record("s3", "a1", 3.0)).await.unwrap();

        let a1 = repo.list_by_agent(Some("a1"), 10).await.unwrap();
        assert_eq!(
            a1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["s3", "s1"]
        );
        let todos = repo.list_by_agent(None, 10).await.unwrap();
        assert_eq!(todos.len(), 3);
    }

    #[tokio::test]
    async fn save_creates_parent_rows_for_fks() {
        // El schema exige agents/graphs: save() los upserta (INSERT OR IGNORE).
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        repo.save(&make_record("s1", "agente-yaml", 1.0))
            .await
            .unwrap();

        let (agents, graphs) = repo
            .with_conn(|conn| {
                let a: i64 = conn
                    .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
                    .unwrap();
                let g: i64 = conn
                    .query_row("SELECT COUNT(*) FROM graphs", [], |r| r.get(0))
                    .unwrap();
                Ok((a, g))
            })
            .await
            .unwrap();
        assert_eq!(agents, 1);
        assert_eq!(graphs, 1);

        // Guardar dos runs del mismo agente no duplica padres.
        repo.save(&make_record("s2", "agente-yaml", 2.0))
            .await
            .unwrap();
        let n = repo
            .with_conn(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get::<_, i64>(0))
                    .unwrap())
            })
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn failed_run_preserves_error_and_status() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        let mut rec = make_record("s-fail", "a1", 1.0);
        rec.status = SessionStatus::Failed;
        rec.result = make_result(ExecutionStatus::Failed, Some("boom en n2"));
        repo.save(&rec).await.unwrap();

        let got = repo.get("s-fail").await.unwrap().unwrap();
        assert_eq!(got.status, SessionStatus::Failed);
        assert_eq!(got.result.status, ExecutionStatus::Failed);
        assert_eq!(got.result.error.as_deref(), Some("boom en n2"));
    }

    #[tokio::test]
    async fn delete_removes_run() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        repo.save(&make_record("s1", "a1", 1.0)).await.unwrap();
        assert!(repo.delete("s1").await.unwrap());
        assert!(!repo.delete("s1").await.unwrap());
        assert!(repo.get("s1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        assert!(repo.get("nope").await.unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // Migración v1 → v2 (W8)
    // -----------------------------------------------------------------------

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mirai-test-mig-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("engine.db")
    }

    /// Crea una DB EXACTAMENTE como la escribía la v1 (0.7.0) y le mete un run.
    fn crear_db_v1(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(super::super::migrations::SCHEMA_SQL)
            .unwrap();
        conn.execute(
            "INSERT INTO graphs (id, name, created_at, updated_at) VALUES ('g1','g1',1.0,1.0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agents (id, name, graph_id, created_at, updated_at)
             VALUES ('a-v1','Agente v1','g1',1.0,1.0)",
            [],
        )
        .unwrap();
        let trace = serde_json::to_string(&make_result(ExecutionStatus::Completed, None).trace)
            .expect("trace");
        conn.execute(
            "INSERT INTO sessions
               (id, agent_id, agent_name, graph_id, status, trace, transcript,
                state, started_at, finished_at, duration_ms)
             VALUES ('run-v1','a-v1','Agente v1','g1','completed', ?1, '[]',
                     '{\"n1\":{\"response\":\"hola\"}}', 1700000000.0, 1700000000.5, 500.0)",
            params![trace],
        )
        .unwrap();

        // Comprobación de que esto ES la v1: sin checkpoints, sin nodo actual.
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('sessions')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !cols.iter().any(|c| c == "current_node_id"),
            "la v1 no tiene current_node_id"
        );
        let tablas: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checkpoints'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tablas, 0, "la v1 no tiene tabla de checkpoints");
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM _schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[tokio::test]
    async fn w8_db_v1_migra_a_v2_y_sus_datos_siguen_legibles() {
        // W8: una DB del esquema anterior tiene que abrir con el binario nuevo,
        // migrar sola, y NO perder nada de lo que ya tenía.
        let path = temp_db("w8");
        crear_db_v1(&path);

        // El binario nuevo la abre.
        let repo = SqliteSessionRepo::open(&path).expect("una DB v1 debe abrir sin romperse");
        assert_eq!(repo.schema_version().await.unwrap(), 2, "migró a v2");

        // Los datos de la v1 siguen ahí y se leen igual.
        let viejo = repo
            .get("run-v1")
            .await
            .unwrap()
            .expect("el run de la v1 debe seguir legible");
        assert_eq!(viejo.agent_id, "a-v1");
        assert_eq!(viejo.agent_name, "Agente v1");
        assert_eq!(viejo.status, SessionStatus::Completed);
        assert_eq!(viejo.result.trace[0].node_id, "n1");
        assert_eq!(
            viejo.result.state.snapshot()["n1"]["response"],
            serde_json::json!("hola")
        );
        assert!(
            viejo.current_node_id.is_none(),
            "la columna nueva llega vacía, no inventada"
        );

        // Y lo nuevo ya funciona sobre esa misma DB.
        let cp = CheckpointRecord {
            session_id: "run-nuevo".into(),
            agent_id: "a-v1".into(),
            step: 1,
            node_id: "n1".into(),
            cursor_node_id: Some("n2".into()),
            state_snapshot: HashMap::new(),
            executed_nodes: vec!["n1".into()],
            created_at: 1700000100.0,
        };
        let progress = RunProgress {
            agent_name: "Agente v1".into(),
            started_at: 1700000100.0,
            status: SessionStatus::Running,
        };
        repo.save_checkpoint(&cp, &progress).await.unwrap();
        assert!(repo.get_checkpoint("run-nuevo").await.unwrap().is_some());
        drop(repo);

        // Y "abrir el binario nuevo" pasa cada vez que arranca el proceso, no
        // una sola: la segunda apertura tiene que ser igual de sana.
        let otra_vez = SqliteSessionRepo::open(&path)
            .expect("una DB ya migrada tiene que volver a abrir sin romperse");
        assert_eq!(otra_vez.schema_version().await.unwrap(), 2);
        assert!(otra_vez.get("run-v1").await.unwrap().is_some());
        assert!(otra_vez
            .get_checkpoint("run-nuevo")
            .await
            .unwrap()
            .is_some());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn reabrir_una_db_ya_migrada_no_reaplica_la_migracion() {
        // La v2 trae `ALTER TABLE`, que NO es idempotente: si el runner de
        // migraciones no respeta la versión aplicada, la segunda apertura
        // muere con "duplicate column name".
        let path = temp_db("reabrir");
        crear_db_v1(&path);

        let repo = SqliteSessionRepo::open(&path).unwrap();
        assert_eq!(repo.schema_version().await.unwrap(), 2);
        drop(repo);

        let otra_vez = SqliteSessionRepo::open(&path).expect("reabrir no puede romper");
        assert_eq!(otra_vez.schema_version().await.unwrap(), 2);
        assert!(otra_vez.get("run-v1").await.unwrap().is_some());
        drop(otra_vez);

        // Y una tercera, por si acaso.
        let tercera = SqliteSessionRepo::open(&path).expect("idempotente de verdad");
        assert_eq!(tercera.schema_version().await.unwrap(), 2);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn db_nueva_arranca_directo_en_v2() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        assert_eq!(
            repo.schema_version().await.unwrap(),
            super::super::migrations::SCHEMA_VERSION
        );
    }

    // -----------------------------------------------------------------------
    // Estados explícitos del run + nodo actual
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn los_cinco_estados_del_run_round_tripean_con_su_nodo_actual() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        let estados = [
            SessionStatus::Running,
            SessionStatus::Paused,
            SessionStatus::Completed,
            SessionStatus::Failed,
            SessionStatus::Cancelled,
        ];
        for (i, estado) in estados.iter().enumerate() {
            let mut rec = make_record(&format!("s{i}"), "a1", i as f64);
            rec.status = estado.clone();
            rec.current_node_id = Some(format!("nodo-{i}"));
            repo.save(&rec).await.unwrap();

            let got = repo.get(&format!("s{i}")).await.unwrap().unwrap();
            assert_eq!(&got.status, estado, "estado {estado} debe persistir");
            assert_eq!(got.current_node_id.as_deref(), Some(&*format!("nodo-{i}")));
        }
    }

    #[tokio::test]
    async fn borrar_checkpoint_no_toca_el_run() {
        let repo = SqliteSessionRepo::open_in_memory().unwrap();
        let cp = CheckpointRecord {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            step: 1,
            node_id: "n1".into(),
            cursor_node_id: None,
            state_snapshot: HashMap::new(),
            executed_nodes: vec![],
            created_at: 1.0,
        };
        let progress = RunProgress {
            agent_name: "Agente".into(),
            started_at: 1.0,
            status: SessionStatus::Running,
        };
        repo.save_checkpoint(&cp, &progress).await.unwrap();

        assert!(repo.delete_checkpoint("s1").await.unwrap());
        assert!(!repo.delete_checkpoint("s1").await.unwrap());
        assert!(repo.get_checkpoint("s1").await.unwrap().is_none());
        assert!(repo.get("s1").await.unwrap().is_some(), "el run sigue ahí");
    }
}
