//! SQLite-backed registry of agent specs.
//!
//! The HTTP server keeps agents in a `HashMap` inside `AppState`, which is fast
//! but dies with the process: every agent registered through
//! `POST /agents/from-spec` is lost on restart, and live agents stop cycling
//! with nobody to restart them. This store is the durable side of that registry:
//! the map stays as the read cache, and every mutation is mirrored here.
//!
//! `rusqlite` is a blocking API, so each operation runs on the blocking pool
//! rather than on the async executor. The operations are single-row reads and
//! writes against a local file — microseconds — but blocking the runtime for
//! them would still be wrong.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde_json::Value;

use super::migrations::MIGRATIONS;
use super::repositories::DbError;
use crate::core::agent_spec::{AgentSpec, MemoryPersistMode};
use crate::utils::now_epoch;

/// Which memory bucket a row belongs to. Mirrors [`MemoryPersistMode`], minus
/// `None` — that mode never persists anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryScope {
    /// Kept within a play session.
    Cycle,
    /// Kept across sessions, restarts and separate executions.
    Execution,
}

impl MemoryScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryScope::Cycle => "cycle",
            MemoryScope::Execution => "execution",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "cycle" => Some(MemoryScope::Cycle),
            "execution" => Some(MemoryScope::Execution),
            _ => None,
        }
    }

    /// The scope a persist mode writes to. `None` → nothing to persist.
    pub fn from_persist_mode(mode: MemoryPersistMode) -> Option<Self> {
        match mode {
            MemoryPersistMode::None => None,
            MemoryPersistMode::Cycle => Some(MemoryScope::Cycle),
            MemoryPersistMode::Execution => Some(MemoryScope::Execution),
        }
    }
}

/// Durable registry of agent specs.
///
/// Cloning is cheap: clones share the same connection.
#[derive(Clone)]
pub struct AgentStore {
    conn: Arc<Mutex<Connection>>,
    path: String,
}

impl AgentStore {
    /// Open (creating it if needed) the database at `path` and apply migrations.
    ///
    /// Pass `":memory:"` for an ephemeral database — useful in tests.
    pub fn open(path: &str) -> Result<Self, DbError> {
        if path != ":memory:" {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        DbError::ConnectionError(format!("cannot create directory for {path}: {e}"))
                    })?;
                }
            }
        }

        let conn = Connection::open(path)
            .map_err(|e| DbError::ConnectionError(format!("cannot open {path}: {e}")))?;

        // WAL keeps readers from blocking the writer, and survives an abrupt
        // process kill better than the default journal.
        if path != ":memory:" {
            let _ = conn.pragma_update(None, "journal_mode", "WAL");
        }
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| DbError::ConnectionError(format!("cannot enable foreign keys: {e}")))?;

        for migration in MIGRATIONS {
            conn.execute_batch(migration.up_sql).map_err(|e| {
                DbError::ConnectionError(format!(
                    "migration {} ({}) failed: {e}",
                    migration.version, migration.description
                ))
            })?;
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: path.to_string(),
        })
    }

    /// Path this store was opened with.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Run a blocking closure against the connection on the blocking pool.
    async fn with_conn<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, DbError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| DbError::ConnectionError("connection mutex poisoned".into()))?;
            f(&guard)
        })
        .await
        .map_err(|e| DbError::ConnectionError(format!("blocking task failed: {e}")))?
    }

    /// Insert or update an agent spec. Keeps `playing` and `created_at` intact
    /// when the agent already exists.
    pub async fn upsert(&self, id: &str, spec: &AgentSpec) -> Result<(), DbError> {
        let json = serde_json::to_string(spec)
            .map_err(|e| DbError::SerializationError(format!("cannot serialize spec: {e}")))?;
        let id = id.to_string();
        let name = spec.name.clone();
        let now = now_epoch();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO agent_specs (id, name, spec, playing, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name,
                     spec = excluded.spec,
                     updated_at = excluded.updated_at",
                params![id, name, json, now],
            )
            .map_err(|e| DbError::ConnectionError(format!("cannot save agent: {e}")))?;
            Ok(())
        })
        .await
    }

    /// Remove an agent. Returns whether a row was actually deleted.
    pub async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let n = conn
                .execute("DELETE FROM agent_specs WHERE id = ?1", params![id])
                .map_err(|e| DbError::ConnectionError(format!("cannot delete agent: {e}")))?;
            Ok(n > 0)
        })
        .await
    }

    /// Every stored agent, as `(id, spec)`.
    ///
    /// A row whose JSON no longer deserializes — a spec written by a newer
    /// build, say — is skipped rather than failing the whole load: one bad row
    /// must not stop the server from coming up.
    pub async fn load_all(&self) -> Result<Vec<(String, AgentSpec)>, DbError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, spec FROM agent_specs ORDER BY created_at")
                .map_err(|e| DbError::ConnectionError(format!("cannot prepare query: {e}")))?;

            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| DbError::ConnectionError(format!("cannot read agents: {e}")))?;

            let mut out = Vec::new();
            for row in rows {
                let (id, json) =
                    row.map_err(|e| DbError::ConnectionError(format!("cannot read row: {e}")))?;
                match serde_json::from_str::<AgentSpec>(&json) {
                    Ok(spec) => out.push((id, spec)),
                    Err(e) => {
                        tracing::warn!(agent_id = %id, error = %e, "skipping unreadable agent spec");
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    /// Mark whether an agent was cycling, so it can be rescheduled on startup.
    pub async fn set_playing(&self, id: &str, playing: bool) -> Result<(), DbError> {
        let id = id.to_string();
        let now = now_epoch();
        self.with_conn(move |conn| {
            conn.execute(
                "UPDATE agent_specs SET playing = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, playing as i64, now],
            )
            .map_err(|e| DbError::ConnectionError(format!("cannot update playing flag: {e}")))?;
            Ok(())
        })
        .await
    }

    /// Ids of the agents that were cycling when the process last stopped.
    pub async fn playing_ids(&self) -> Result<Vec<String>, DbError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM agent_specs WHERE playing = 1")
                .map_err(|e| DbError::ConnectionError(format!("cannot prepare query: {e}")))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    DbError::ConnectionError(format!("cannot read playing agents: {e}"))
                })?;

            let mut out = Vec::new();
            for row in rows {
                out.push(
                    row.map_err(|e| DbError::ConnectionError(format!("cannot read row: {e}")))?,
                );
            }
            Ok(out)
        })
        .await
    }

    // -----------------------------------------------------------------------
    // Memoria de agentes
    // -----------------------------------------------------------------------

    /// Store an agent's memory for a scope, replacing whatever was there.
    ///
    /// The map arrives already filtered to the keys the agent declares — this
    /// only mirrors it.
    pub async fn save_memory(
        &self,
        agent_id: &str,
        scope: MemoryScope,
        memory: &HashMap<String, Value>,
    ) -> Result<(), DbError> {
        let json = serde_json::to_string(memory)
            .map_err(|e| DbError::SerializationError(format!("cannot serialize memory: {e}")))?;
        let id = agent_id.to_string();
        let scope = scope.as_str();
        let now = now_epoch();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO agent_memory (agent_id, scope, memory, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id, scope) DO UPDATE SET
                     memory = excluded.memory,
                     updated_at = excluded.updated_at",
                params![id, scope, json, now],
            )
            .map_err(|e| DbError::ConnectionError(format!("cannot save memory: {e}")))?;
            Ok(())
        })
        .await
    }

    /// Every stored memory row, as `(agent_id, scope, memory)`.
    ///
    /// Used at startup to warm the in-memory store. Rows that no longer
    /// deserialize are skipped: bad memory must not stop the server from
    /// booting, the agent just starts from its initial values.
    #[allow(clippy::type_complexity)]
    pub async fn load_all_memory(
        &self,
    ) -> Result<Vec<(String, MemoryScope, HashMap<String, Value>)>, DbError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT agent_id, scope, memory FROM agent_memory")
                .map_err(|e| DbError::ConnectionError(format!("cannot prepare query: {e}")))?;

            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| DbError::ConnectionError(format!("cannot read memory: {e}")))?;

            let mut out = Vec::new();
            for row in rows {
                let (id, scope, json) =
                    row.map_err(|e| DbError::ConnectionError(format!("cannot read row: {e}")))?;
                let scope = match MemoryScope::from_str(&scope) {
                    Some(s) => s,
                    None => {
                        tracing::warn!(agent_id = %id, scope = %scope, "skipping memory row with unknown scope");
                        continue;
                    }
                };
                match serde_json::from_str::<HashMap<String, Value>>(&json) {
                    Ok(memory) => out.push((id, scope, memory)),
                    Err(e) => {
                        tracing::warn!(agent_id = %id, error = %e, "skipping unreadable agent memory");
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    /// Drop stored memory. `None` clears every scope for that agent.
    pub async fn clear_memory(
        &self,
        agent_id: &str,
        scope: Option<MemoryScope>,
    ) -> Result<(), DbError> {
        let id = agent_id.to_string();
        self.with_conn(move |conn| {
            match scope {
                Some(s) => conn.execute(
                    "DELETE FROM agent_memory WHERE agent_id = ?1 AND scope = ?2",
                    params![id, s.as_str()],
                ),
                None => conn.execute("DELETE FROM agent_memory WHERE agent_id = ?1", params![id]),
            }
            .map_err(|e| DbError::ConnectionError(format!("cannot clear memory: {e}")))?;
            Ok(())
        })
        .await
    }

    /// Number of stored agents.
    pub async fn count(&self) -> Result<usize, DbError> {
        self.with_conn(move |conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM agent_specs", [], |row| row.get(0))
                .map_err(|e| DbError::ConnectionError(format!("cannot count agents: {e}")))?;
            Ok(n as usize)
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
    use tempfile::TempDir;

    fn spec(name: &str) -> AgentSpec {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [{"source": "trigger", "target": "out"}]
            }
        }))
        .expect("spec de prueba válido")
    }

    #[tokio::test]
    async fn guarda_y_recupera_un_agente() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("a1", &spec("agente-uno")).await.unwrap();

        let todos = store.load_all().await.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].0, "a1");
        assert_eq!(todos[0].1.name, "agente-uno");
    }

    #[tokio::test]
    async fn conserva_el_grafo_completo_del_spec() {
        let store = AgentStore::open(":memory:").unwrap();
        let original = spec("con-grafo");
        store.upsert("a1", &original).await.unwrap();

        let (_, recuperado) = store.load_all().await.unwrap().pop().unwrap();
        assert_eq!(recuperado.graph.nodes.len(), original.graph.nodes.len());
        assert_eq!(recuperado.graph.edges.len(), original.graph.edges.len());
        assert_eq!(recuperado.graph.nodes[0].id, "trigger");
    }

    #[tokio::test]
    async fn upsert_actualiza_sin_duplicar() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("a1", &spec("v-vieja")).await.unwrap();
        store.upsert("a1", &spec("v-nueva")).await.unwrap();

        let todos = store.load_all().await.unwrap();
        assert_eq!(todos.len(), 1, "el upsert duplicó la fila");
        assert_eq!(todos[0].1.name, "v-nueva");
    }

    #[tokio::test]
    async fn upsert_no_pisa_el_flag_playing() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("a1", &spec("live")).await.unwrap();
        store.set_playing("a1", true).await.unwrap();

        // Volver a guardar el spec no debe apagar el agente.
        store.upsert("a1", &spec("live")).await.unwrap();
        assert_eq!(store.playing_ids().await.unwrap(), vec!["a1".to_string()]);
    }

    #[tokio::test]
    async fn playing_ids_solo_devuelve_los_que_ciclan() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("quieto", &spec("quieto")).await.unwrap();
        store.upsert("live", &spec("live")).await.unwrap();
        store.set_playing("live", true).await.unwrap();

        assert_eq!(store.playing_ids().await.unwrap(), vec!["live".to_string()]);

        store.set_playing("live", false).await.unwrap();
        assert!(store.playing_ids().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn borra_agentes() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("a1", &spec("uno")).await.unwrap();

        assert!(store.delete("a1").await.unwrap());
        assert!(!store.delete("a1").await.unwrap(), "borrar dos veces");
        assert_eq!(store.count().await.unwrap(), 0);
    }

    /// El punto de todo esto: los agentes sobreviven a que el proceso muera.
    #[tokio::test]
    async fn los_agentes_sobreviven_a_reabrir_la_base() {
        let dir = TempDir::new().unwrap();
        let ruta = dir.path().join("agentes.db");
        let ruta = ruta.to_str().unwrap();

        {
            let store = AgentStore::open(ruta).unwrap();
            store.upsert("a1", &spec("persistente")).await.unwrap();
            store.upsert("a2", &spec("live")).await.unwrap();
            store.set_playing("a2", true).await.unwrap();
        } // se cierra la conexión, como si cayera el proceso

        let store = AgentStore::open(ruta).unwrap();
        let todos = store.load_all().await.unwrap();
        assert_eq!(todos.len(), 2, "los agentes no sobrevivieron");
        assert_eq!(store.playing_ids().await.unwrap(), vec!["a2".to_string()]);
    }

    // --- memoria de agentes -------------------------------------------------

    fn memoria(clave: &str, valor: i64) -> HashMap<String, Value> {
        let mut m = HashMap::new();
        m.insert(clave.to_string(), serde_json::json!(valor));
        m
    }

    #[tokio::test]
    async fn guarda_y_recupera_memoria() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("a1", &spec("con-memoria")).await.unwrap();
        store
            .save_memory("a1", MemoryScope::Execution, &memoria("total", 7))
            .await
            .unwrap();

        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 1);
        assert_eq!(filas[0].0, "a1");
        assert_eq!(filas[0].1, MemoryScope::Execution);
        assert_eq!(filas[0].2["total"], serde_json::json!(7));
    }

    #[tokio::test]
    async fn los_ambitos_no_se_pisan() {
        let store = AgentStore::open(":memory:").unwrap();
        store
            .save_memory("a1", MemoryScope::Cycle, &memoria("total", 1))
            .await
            .unwrap();
        store
            .save_memory("a1", MemoryScope::Execution, &memoria("total", 99))
            .await
            .unwrap();

        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 2, "cycle y execution son filas distintas");
        let ciclo = filas.iter().find(|f| f.1 == MemoryScope::Cycle).unwrap();
        let ejec = filas
            .iter()
            .find(|f| f.1 == MemoryScope::Execution)
            .unwrap();
        assert_eq!(ciclo.2["total"], serde_json::json!(1));
        assert_eq!(ejec.2["total"], serde_json::json!(99));
    }

    #[tokio::test]
    async fn guardar_reemplaza_el_mapa_entero() {
        let store = AgentStore::open(":memory:").unwrap();
        store
            .save_memory("a1", MemoryScope::Cycle, &memoria("total", 1))
            .await
            .unwrap();
        store
            .save_memory("a1", MemoryScope::Cycle, &memoria("total", 2))
            .await
            .unwrap();

        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 1, "el upsert duplicó la fila");
        assert_eq!(filas[0].2["total"], serde_json::json!(2));
    }

    #[tokio::test]
    async fn borra_memoria_por_ambito_y_completa() {
        let store = AgentStore::open(":memory:").unwrap();
        for ambito in [MemoryScope::Cycle, MemoryScope::Execution] {
            store
                .save_memory("a1", ambito, &memoria("total", 5))
                .await
                .unwrap();
        }

        store
            .clear_memory("a1", Some(MemoryScope::Cycle))
            .await
            .unwrap();
        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 1);
        assert_eq!(filas[0].1, MemoryScope::Execution);

        store.clear_memory("a1", None).await.unwrap();
        assert!(store.load_all_memory().await.unwrap().is_empty());
    }

    /// Lo que pidió esta función: que la memoria no muera con el proceso.
    #[tokio::test]
    async fn la_memoria_sobrevive_a_reabrir_la_base() {
        let dir = TempDir::new().unwrap();
        let ruta = dir.path().join("memoria.db");
        let ruta = ruta.to_str().unwrap();

        {
            let store = AgentStore::open(ruta).unwrap();
            store.upsert("a1", &spec("live")).await.unwrap();
            store
                .save_memory("a1", MemoryScope::Cycle, &memoria("ultimo_ciclo", 42))
                .await
                .unwrap();
        } // muere el proceso

        let store = AgentStore::open(ruta).unwrap();
        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 1, "la memoria no sobrevivió");
        assert_eq!(filas[0].2["ultimo_ciclo"], serde_json::json!(42));
    }

    #[tokio::test]
    async fn una_memoria_ilegible_no_tumba_la_carga() {
        let store = AgentStore::open(":memory:").unwrap();
        store
            .save_memory("bueno", MemoryScope::Cycle, &memoria("total", 1))
            .await
            .unwrap();
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO agent_memory (agent_id, scope, memory, updated_at)
                     VALUES ('roto', 'cycle', '{no es json}', 0.0),
                            ('raro', 'inventado', '{}', 0.0)",
                    [],
                )
                .map_err(|e| DbError::ConnectionError(e.to_string()))?;
                Ok(())
            })
            .await
            .unwrap();

        let filas = store.load_all_memory().await.unwrap();
        assert_eq!(filas.len(), 1, "las filas rotas debían saltearse");
        assert_eq!(filas[0].0, "bueno");
    }

    #[tokio::test]
    async fn abrir_dos_veces_no_rompe_el_esquema() {
        let dir = TempDir::new().unwrap();
        let ruta = dir.path().join("idempotente.db");
        let ruta = ruta.to_str().unwrap();

        let primera = AgentStore::open(ruta).unwrap();
        primera.upsert("a1", &spec("uno")).await.unwrap();
        drop(primera);

        // Las migraciones se vuelven a aplicar: deben ser idempotentes.
        let segunda = AgentStore::open(ruta).unwrap();
        assert_eq!(segunda.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn crea_el_directorio_si_falta() {
        let dir = TempDir::new().unwrap();
        let ruta = dir.path().join("sub/carpeta/agentes.db");
        let store = AgentStore::open(ruta.to_str().unwrap()).unwrap();
        store.upsert("a1", &spec("uno")).await.unwrap();
        assert!(ruta.exists());
    }

    #[tokio::test]
    async fn una_fila_ilegible_no_tumba_la_carga() {
        let store = AgentStore::open(":memory:").unwrap();
        store.upsert("bueno", &spec("bueno")).await.unwrap();

        // Fila corrupta escrita a mano, como la dejaría un build más nuevo.
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO agent_specs (id, name, spec, playing, created_at, updated_at)
                     VALUES ('roto', 'roto', '{no es json}', 0, 0.0, 0.0)",
                    [],
                )
                .map_err(|e| DbError::ConnectionError(e.to_string()))?;
                Ok(())
            })
            .await
            .unwrap();

        let todos = store.load_all().await.unwrap();
        assert_eq!(todos.len(), 1, "la fila rota debía saltearse, no fallar");
        assert_eq!(todos[0].0, "bueno");
    }
}
