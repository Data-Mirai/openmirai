//! Checkpoints que sobreviven al proceso (PRD-021-A).
//!
//! Hasta 0.7.0 el motor **no podía reanudar nada**: `CheckpointCallback` era un
//! trait sin una sola implementación en el repo y `with_checkpoint_callback()`
//! no se llamaba fuera de los tests. El estado de una ejecución vivía en la RAM
//! del runner y moría con ella.
//!
//! Aquí está la implementación real contra SQLite (misma DB que ya persiste los
//! runs, `~/.openmirai/engine.db`):
//!
//! - [`CheckpointRecord`] — la fila: dónde iba, qué nodos ya corrieron y el
//!   estado compartido. Es lo mínimo (y suficiente) para retomar.
//! - [`SqliteCheckpointStore`] — el `CheckpointCallback` que el runner invoca
//!   al terminar cada nodo/bloque y al detenerse.
//!
//! Escritura **best-effort**: si el checkpoint no se puede guardar se loguea y
//! la ejecución sigue (persistir no puede tumbar un run), igual que el registro
//! de runs de 0.7.0.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::core::runner::{Checkpoint, CheckpointCallback, RunnerError};
use crate::core::state::SharedState;

use super::repositories::SessionStatus;
use super::sqlite::SqliteSessionRepo;

// ---------------------------------------------------------------------------
// CheckpointRecord
// ---------------------------------------------------------------------------

/// Estado persistido de un run en vuelo — un registro por run (se sobreescribe).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    /// Id del run (PK: un checkpoint vivo por run).
    pub session_id: String,
    /// Agente al que pertenece el run — con esto se sabe QUÉ grafo reanudar.
    pub agent_id: String,
    /// Nº de bloque dentro de la ejecución.
    pub step: u32,
    /// Último nodo procesado (o el nodo en el que se detuvo).
    pub node_id: String,
    /// Nodo donde continuaría la ejecución. `None` = no queda nada por correr.
    pub cursor_node_id: Option<String>,
    /// Estado compartido completo: salida de cada nodo.
    pub state_snapshot: HashMap<String, HashMap<String, Value>>,
    /// Nodos ya ejecutados, en orden. Reanudar NO debe repetirlos (efectos).
    pub executed_nodes: Vec<String>,
    pub created_at: f64,
}

impl CheckpointRecord {
    /// Construye el registro desde el [`Checkpoint`] que emite el runner.
    pub fn from_checkpoint(cp: Checkpoint, agent_id: &str) -> Self {
        Self {
            session_id: cp.session_id,
            agent_id: agent_id.to_string(),
            step: cp.step,
            node_id: cp.node_id,
            cursor_node_id: cp.cursor_node_id,
            state_snapshot: cp.state_snapshot,
            executed_nodes: cp.executed_nodes,
            created_at: cp.timestamp,
        }
    }

    /// Nodo por el que va el run: donde continuaría, o el último tocado si ya
    /// no hay siguiente. Es lo que se muestra como "nodo actual".
    pub fn resume_node_id(&self) -> String {
        self.cursor_node_id
            .clone()
            .unwrap_or_else(|| self.node_id.clone())
    }

    /// ¿Este nodo ya corrió? Reanudar no debe volver a ejecutarlo.
    pub fn already_executed(&self, node_id: &str) -> bool {
        self.executed_nodes.iter().any(|n| n == node_id)
    }

    /// Rearma el `SharedState` guardado, listo para `GraphRunner::resume()`.
    pub fn to_shared_state(&self) -> Result<SharedState, crate::core::state::StateError> {
        let state = SharedState::new();
        for (node_id, output) in &self.state_snapshot {
            state.set(node_id, output.clone(), true)?;
        }
        Ok(state)
    }
}

// ---------------------------------------------------------------------------
// RunProgress
// ---------------------------------------------------------------------------

/// Lo que hace falta para mantener al día la fila del run mientras corre.
#[derive(Debug, Clone)]
pub struct RunProgress {
    pub agent_name: String,
    pub started_at: f64,
    pub status: SessionStatus,
}

// ---------------------------------------------------------------------------
// SqliteCheckpointStore
// ---------------------------------------------------------------------------

/// `CheckpointCallback` real: escribe el checkpoint en SQLite y deja el run
/// marcado `running` con su nodo actual.
///
/// Se construye **por run** (como el `stream_tx` del runner) porque necesita la
/// identidad del run: el `Checkpoint` del runner solo trae `session_id`.
pub struct SqliteCheckpointStore {
    repo: Arc<SqliteSessionRepo>,
    agent_id: String,
    agent_name: String,
    started_at: f64,
}

impl SqliteCheckpointStore {
    pub fn new(
        repo: Arc<SqliteSessionRepo>,
        agent_id: impl Into<String>,
        agent_name: impl Into<String>,
        started_at: f64,
    ) -> Self {
        Self {
            repo,
            agent_id: agent_id.into(),
            agent_name: agent_name.into(),
            started_at,
        }
    }
}

#[async_trait]
impl CheckpointCallback for SqliteCheckpointStore {
    async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError> {
        let session_id = checkpoint.session_id.clone();
        let rec = CheckpointRecord::from_checkpoint(checkpoint, &self.agent_id);
        let progress = RunProgress {
            agent_name: self.agent_name.clone(),
            started_at: self.started_at,
            status: SessionStatus::Running,
        };
        match self.repo.save_checkpoint(&rec, &progress).await {
            Ok(()) => Ok(session_id),
            Err(e) => {
                // Best-effort: el runner loguea y sigue; el run no se cae por
                // no haber podido persistir.
                warn!(session_id = %session_id, error = %e, "no se pudo guardar el checkpoint");
                Err(RunnerError::Internal {
                    context: format!("checkpoint del run '{session_id}'"),
                    message: e.to_string(),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(pairs: &[(&str, &str)]) -> HashMap<String, HashMap<String, Value>> {
        pairs
            .iter()
            .map(|(node, val)| {
                let mut out = HashMap::new();
                out.insert("response".to_string(), Value::String(val.to_string()));
                (node.to_string(), out)
            })
            .collect()
    }

    fn make_checkpoint() -> Checkpoint {
        Checkpoint {
            session_id: "run-1".into(),
            step: 2,
            node_id: "n2".into(),
            state_snapshot: snapshot(&[("n1", "uno"), ("n2", "dos")]),
            cursor_node_id: Some("n3".into()),
            executed_nodes: vec!["n1".into(), "n2".into()],
            timestamp: 1700000000.0,
        }
    }

    #[test]
    fn from_checkpoint_keeps_everything_needed_to_resume() {
        let rec = CheckpointRecord::from_checkpoint(make_checkpoint(), "agente-x");
        assert_eq!(rec.session_id, "run-1");
        assert_eq!(rec.agent_id, "agente-x");
        assert_eq!(rec.cursor_node_id.as_deref(), Some("n3"));
        assert_eq!(rec.executed_nodes, vec!["n1", "n2"]);
        assert_eq!(rec.state_snapshot.len(), 2);
    }

    #[test]
    fn resume_node_falls_back_to_last_node() {
        let mut rec = CheckpointRecord::from_checkpoint(make_checkpoint(), "a");
        assert_eq!(rec.resume_node_id(), "n3");
        rec.cursor_node_id = None;
        assert_eq!(rec.resume_node_id(), "n2");
    }

    #[test]
    fn already_executed_marks_the_nodes_that_ran() {
        let rec = CheckpointRecord::from_checkpoint(make_checkpoint(), "a");
        assert!(rec.already_executed("n1"));
        assert!(rec.already_executed("n2"));
        assert!(!rec.already_executed("n3"), "n3 aún no corrió");
    }

    #[test]
    fn to_shared_state_rebuilds_the_state() {
        let rec = CheckpointRecord::from_checkpoint(make_checkpoint(), "a");
        let state = rec.to_shared_state().unwrap();
        assert_eq!(state.get_field("n1", "response").unwrap(), "uno");
        assert_eq!(state.get_field("n2", "response").unwrap(), "dos");
    }

    // -----------------------------------------------------------------------
    // W3 — el checkpoint sobrevive al reinicio del proceso
    // -----------------------------------------------------------------------

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mirai-test-cp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("engine.db")
    }

    #[tokio::test]
    async fn w3_checkpoint_sobrevive_al_reinicio_del_proceso() {
        use crate::db::Repository;
        // W3: guardar checkpoint → simular reinicio (instancia NUEVA del repo
        // contra la MISMA DB, cero cache en memoria) → sigue ahí y trae todo
        // lo necesario para reanudar.
        let path = temp_db("w3");

        {
            let repo = Arc::new(SqliteSessionRepo::open(&path).unwrap());
            let store =
                SqliteCheckpointStore::new(repo.clone(), "agente-1", "Agente Uno", 1700000000.0);
            // Camino real: el runner entrega un `Checkpoint` al callback.
            let id = store.save_checkpoint(make_checkpoint()).await.unwrap();
            assert_eq!(id, "run-1");
        } // Drop: se cierra la conexión. Todo lo que quede en RAM se pierde.

        // "Reinicio": instancia nueva, mismo archivo.
        let reabierto = SqliteSessionRepo::open(&path).unwrap();
        let cp = reabierto
            .get_checkpoint("run-1")
            .await
            .unwrap()
            .expect("el checkpoint debe sobrevivir el reinicio");

        // Lo necesario para reanudar: dónde seguir, qué ya corrió, y el estado.
        assert_eq!(cp.agent_id, "agente-1", "hay que saber QUÉ grafo reanudar");
        assert_eq!(cp.cursor_node_id.as_deref(), Some("n3"));
        assert_eq!(cp.executed_nodes, vec!["n1", "n2"]);
        assert_eq!(cp.step, 2);
        let state = cp.to_shared_state().unwrap();
        assert_eq!(state.get_field("n1", "response").unwrap(), "uno");
        assert_eq!(state.get_field("n2", "response").unwrap(), "dos");

        // Y el run quedó marcado como vivo, en su nodo actual.
        let run = reabierto
            .get("run-1")
            .await
            .unwrap()
            .expect("la fila del run se escribe con el checkpoint");
        assert_eq!(run.status, SessionStatus::Running);
        assert!(run.status.is_live());
        assert_eq!(run.current_node_id.as_deref(), Some("n3"));
        assert!((run.created_at - 1700000000.0).abs() < 1e-6);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn checkpoint_se_sobreescribe_por_run() {
        use crate::db::Repository;
        // El riesgo del PRD: el estado guardado no puede crecer sin límite.
        // Un run = un checkpoint, siempre el último.
        let repo = Arc::new(SqliteSessionRepo::open_in_memory().unwrap());
        let store = SqliteCheckpointStore::new(repo.clone(), "a1", "Agente", 1.0);

        store.save_checkpoint(make_checkpoint()).await.unwrap();
        let mut avanzado = make_checkpoint();
        avanzado.step = 3;
        avanzado.node_id = "n3".into();
        avanzado.cursor_node_id = Some("n4".into());
        avanzado.executed_nodes = vec!["n1".into(), "n2".into(), "n3".into()];
        store.save_checkpoint(avanzado).await.unwrap();

        let cp = repo.get_checkpoint("run-1").await.unwrap().unwrap();
        assert_eq!(cp.step, 3);
        assert_eq!(cp.cursor_node_id.as_deref(), Some("n4"));
        assert_eq!(cp.executed_nodes.len(), 3);

        // Y el nodo actual del run avanzó con él.
        let run = repo.get("run-1").await.unwrap().unwrap();
        assert_eq!(run.current_node_id.as_deref(), Some("n4"));
    }

    #[tokio::test]
    async fn un_run_terminado_no_revive_por_un_checkpoint_tardio() {
        // Guardar un checkpoint no puede devolver a `running` un run que ya
        // cerró: el estado terminal manda.
        use crate::db::Repository;
        let repo = Arc::new(SqliteSessionRepo::open_in_memory().unwrap());
        let store = SqliteCheckpointStore::new(repo.clone(), "a1", "Agente", 1.0);
        store.save_checkpoint(make_checkpoint()).await.unwrap();

        // El run termina y se registra.
        let mut rec = repo.get("run-1").await.unwrap().unwrap();
        rec.status = SessionStatus::Completed;
        rec.finished_at = Some(2.0);
        repo.save(&rec).await.unwrap();

        // Checkpoint tardío (una tarea rezagada del runner).
        store.save_checkpoint(make_checkpoint()).await.unwrap();

        let run = repo.get("run-1").await.unwrap().unwrap();
        assert_eq!(run.status, SessionStatus::Completed);
    }
}
