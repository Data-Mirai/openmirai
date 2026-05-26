//! Repositories — generic async CRUD for graphs, agents, sessions.
//!
//! Defines a `Repository<T>` trait that can be backed by any storage engine.
//! Ships with in-memory implementations (`InMemoryGraphRepo`,
//! `InMemoryAgentRepo`, `InMemorySessionRepo`) suitable for dev/testing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::core::agent_spec::AgentSpec;
use crate::core::graph::GraphDef;
use crate::core::runner::ExecutionResult;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DbError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("duplicate id: {0}")]
    DuplicateId(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("connection error: {0}")]
    ConnectionError(String),
}

// ---------------------------------------------------------------------------
// Generic Repository trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Repository<T>: Send + Sync {
    async fn save(&self, item: &T) -> Result<(), DbError>;
    async fn get(&self, id: &str) -> Result<Option<T>, DbError>;
    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<T>, DbError>;
    async fn delete(&self, id: &str) -> Result<bool, DbError>;
}

// ---------------------------------------------------------------------------
// AgentRecord
// ---------------------------------------------------------------------------

/// Status of an agent in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Enabled,
    Disabled,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Enabled => write!(f, "enabled"),
            AgentStatus::Disabled => write!(f, "disabled"),
        }
    }
}

/// Persisted agent record wrapping an `AgentSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub spec: AgentSpec,
    pub status: AgentStatus,
    pub created_at: f64,
    pub updated_at: f64,
}

impl AgentRecord {
    /// Helper to extract the record id.
    pub fn record_id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// SessionRecord
// ---------------------------------------------------------------------------

/// Lifecycle status of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Persisted session record wrapping an `ExecutionResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub agent_id: String,
    pub result: ExecutionResult,
    pub created_at: f64,
    pub status: SessionStatus,
}

impl SessionRecord {
    pub fn record_id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// InMemoryGraphRepo
// ---------------------------------------------------------------------------

/// In-memory implementation of `Repository<GraphDef>`.
///
/// Stores graphs keyed by `GraphDef.id` in an `Arc<RwLock<HashMap>>`.
#[derive(Debug, Clone)]
pub struct InMemoryGraphRepo {
    store: Arc<RwLock<HashMap<String, GraphDef>>>,
}

impl InMemoryGraphRepo {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryGraphRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Repository<GraphDef> for InMemoryGraphRepo {
    async fn save(&self, item: &GraphDef) -> Result<(), DbError> {
        let mut store = self.store.write().await;
        store.insert(item.id.clone(), item.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<GraphDef>, DbError> {
        let store = self.store.read().await;
        Ok(store.get(id).cloned())
    }

    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<GraphDef>, DbError> {
        let store = self.store.read().await;
        let items: Vec<GraphDef> = store.values().cloned().collect();
        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let mut store = self.store.write().await;
        Ok(store.remove(id).is_some())
    }
}

// ---------------------------------------------------------------------------
// InMemoryAgentRepo
// ---------------------------------------------------------------------------

/// In-memory implementation of `Repository<AgentRecord>`.
#[derive(Debug, Clone)]
pub struct InMemoryAgentRepo {
    store: Arc<RwLock<HashMap<String, AgentRecord>>>,
}

impl InMemoryAgentRepo {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update only the status field.
    pub async fn update_status(&self, id: &str, status: AgentStatus) -> Result<(), DbError> {
        let mut store = self.store.write().await;
        match store.get_mut(id) {
            Some(record) => {
                record.status = status;
                record.updated_at = now_epoch();
                Ok(())
            }
            None => Err(DbError::NotFound(format!("agent '{}'", id))),
        }
    }
}

impl Default for InMemoryAgentRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Repository<AgentRecord> for InMemoryAgentRepo {
    async fn save(&self, item: &AgentRecord) -> Result<(), DbError> {
        let mut store = self.store.write().await;
        store.insert(item.id.clone(), item.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<AgentRecord>, DbError> {
        let store = self.store.read().await;
        Ok(store.get(id).cloned())
    }

    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<AgentRecord>, DbError> {
        let store = self.store.read().await;
        let mut items: Vec<AgentRecord> = store.values().cloned().collect();
        // Sort by created_at descending (newest first), matching Python behaviour.
        items.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let mut store = self.store.write().await;
        Ok(store.remove(id).is_some())
    }
}

// ---------------------------------------------------------------------------
// InMemorySessionRepo
// ---------------------------------------------------------------------------

/// In-memory implementation of `Repository<SessionRecord>`.
#[derive(Debug, Clone)]
pub struct InMemorySessionRepo {
    store: Arc<RwLock<HashMap<String, SessionRecord>>>,
}

impl InMemorySessionRepo {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// List sessions optionally filtered by agent_id, sorted by created_at
    /// descending.
    pub async fn list_by_agent(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, DbError> {
        let store = self.store.read().await;
        let mut items: Vec<SessionRecord> = store
            .values()
            .filter(|s| match agent_id {
                Some(aid) => s.agent_id == aid,
                None => true,
            })
            .cloned()
            .collect();
        items.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items.into_iter().take(limit).collect())
    }
}

impl Default for InMemorySessionRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Repository<SessionRecord> for InMemorySessionRepo {
    async fn save(&self, item: &SessionRecord) -> Result<(), DbError> {
        let mut store = self.store.write().await;
        store.insert(item.id.clone(), item.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<SessionRecord>, DbError> {
        let store = self.store.read().await;
        Ok(store.get(id).cloned())
    }

    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<SessionRecord>, DbError> {
        let store = self.store.read().await;
        let mut items: Vec<SessionRecord> = store.values().cloned().collect();
        items.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let mut store = self.store.write().await;
        Ok(store.remove(id).is_some())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::graph::{GraphDef, NodeDef};
    use crate::core::runner::{ExecutionResult, ExecutionStatus};
    use crate::core::state::SharedState;

    fn make_graph(id: &str) -> GraphDef {
        GraphDef {
            id: id.to_string(),
            name: format!("graph-{}", id),
            version: "1.0".to_string(),
            nodes: vec![NodeDef {
                id: "n1".into(),
                tool_type: "ai/llm_call".into(),
                version: "1.0.0".into(),
                config: HashMap::new(),
                position: Some((0.0, 0.0)),
            }],
            edges: vec![],
            metadata: HashMap::new(),
        }
    }

    fn make_agent(id: &str) -> AgentRecord {
        AgentRecord {
            id: id.to_string(),
            spec: AgentSpec {
                name: format!("agent-{}", id),
                description: String::new(),
                version: "v1".to_string(),
                agent_type: crate::core::agent_spec::AgentType::Managed,
                system_prompt: None,
                graph: Default::default(),
                triggers: vec![],
                config: Default::default(),
                resources: vec![],
                metadata: HashMap::new(),
            },
            status: AgentStatus::Disabled,
            created_at: now_epoch(),
            updated_at: now_epoch(),
        }
    }

    fn make_session(id: &str, agent_id: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            result: ExecutionResult {
                status: ExecutionStatus::Completed,
                state: SharedState::new(),
                trace: vec![],
                transcript: vec![],
                error: None,
                interrupt_node_id: None,
                interrupt_info: None,
            },
            created_at: now_epoch(),
            status: SessionStatus::Completed,
        }
    }

    // -- Graph repo --

    #[tokio::test]
    async fn graph_save_and_get() {
        let repo = InMemoryGraphRepo::new();
        let g = make_graph("g1");
        repo.save(&g).await.unwrap();

        let got = repo.get("g1").await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().name, "graph-g1");
    }

    #[tokio::test]
    async fn graph_get_missing() {
        let repo = InMemoryGraphRepo::new();
        let got = repo.get("nope").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn graph_list_with_pagination() {
        let repo = InMemoryGraphRepo::new();
        for i in 0..5 {
            repo.save(&make_graph(&format!("g{}", i))).await.unwrap();
        }
        let all = repo.list(10, 0).await.unwrap();
        assert_eq!(all.len(), 5);

        let page = repo.list(2, 2).await.unwrap();
        assert_eq!(page.len(), 2);
    }

    #[tokio::test]
    async fn graph_delete() {
        let repo = InMemoryGraphRepo::new();
        repo.save(&make_graph("g1")).await.unwrap();
        assert!(repo.delete("g1").await.unwrap());
        assert!(!repo.delete("g1").await.unwrap());
        assert!(repo.get("g1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn graph_save_upsert() {
        let repo = InMemoryGraphRepo::new();
        let mut g = make_graph("g1");
        repo.save(&g).await.unwrap();
        g.name = "updated".to_string();
        repo.save(&g).await.unwrap();

        let got = repo.get("g1").await.unwrap().unwrap();
        assert_eq!(got.name, "updated");
    }

    // -- Agent repo --

    #[tokio::test]
    async fn agent_save_and_get() {
        let repo = InMemoryAgentRepo::new();
        let a = make_agent("a1");
        repo.save(&a).await.unwrap();

        let got = repo.get("a1").await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().spec.name, "agent-a1");
    }

    #[tokio::test]
    async fn agent_update_status() {
        let repo = InMemoryAgentRepo::new();
        repo.save(&make_agent("a1")).await.unwrap();

        repo.update_status("a1", AgentStatus::Enabled).await.unwrap();
        let got = repo.get("a1").await.unwrap().unwrap();
        assert_eq!(got.status, AgentStatus::Enabled);
    }

    #[tokio::test]
    async fn agent_update_status_not_found() {
        let repo = InMemoryAgentRepo::new();
        let err = repo.update_status("nope", AgentStatus::Enabled).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn agent_delete() {
        let repo = InMemoryAgentRepo::new();
        repo.save(&make_agent("a1")).await.unwrap();
        assert!(repo.delete("a1").await.unwrap());
        assert!(repo.get("a1").await.unwrap().is_none());
    }

    // -- Session repo --

    #[tokio::test]
    async fn session_save_and_get() {
        let repo = InMemorySessionRepo::new();
        let s = make_session("s1", "a1");
        repo.save(&s).await.unwrap();

        let got = repo.get("s1").await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().agent_id, "a1");
    }

    #[tokio::test]
    async fn session_list_by_agent() {
        let repo = InMemorySessionRepo::new();
        repo.save(&make_session("s1", "a1")).await.unwrap();
        repo.save(&make_session("s2", "a1")).await.unwrap();
        repo.save(&make_session("s3", "a2")).await.unwrap();

        let a1_sessions = repo.list_by_agent(Some("a1"), 50).await.unwrap();
        assert_eq!(a1_sessions.len(), 2);

        let all = repo.list_by_agent(None, 50).await.unwrap();
        assert_eq!(all.len(), 3);

        let limited = repo.list_by_agent(None, 2).await.unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[tokio::test]
    async fn session_delete() {
        let repo = InMemorySessionRepo::new();
        repo.save(&make_session("s1", "a1")).await.unwrap();
        assert!(repo.delete("s1").await.unwrap());
        assert!(repo.get("s1").await.unwrap().is_none());
    }

    // -- DbError display --

    #[test]
    fn db_error_display() {
        let e = DbError::NotFound("x".into());
        assert_eq!(format!("{}", e), "not found: x");

        let e = DbError::DuplicateId("y".into());
        assert_eq!(format!("{}", e), "duplicate id: y");

        let e = DbError::SerializationError("bad json".into());
        assert_eq!(format!("{}", e), "serialization error: bad json");

        let e = DbError::ConnectionError("timeout".into());
        assert_eq!(format!("{}", e), "connection error: timeout");
    }

    // -- AgentStatus / SessionStatus display --

    #[test]
    fn status_display() {
        assert_eq!(AgentStatus::Enabled.to_string(), "enabled");
        assert_eq!(AgentStatus::Disabled.to_string(), "disabled");
        assert_eq!(SessionStatus::Running.to_string(), "running");
        assert_eq!(SessionStatus::Completed.to_string(), "completed");
        assert_eq!(SessionStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn agent_status_serde_roundtrip() {
        let s = AgentStatus::Enabled;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"enabled\"");
        let back: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AgentStatus::Enabled);
    }

    #[test]
    fn session_status_serde_roundtrip() {
        let s = SessionStatus::Failed;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"failed\"");
        let back: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SessionStatus::Failed);
    }
}
