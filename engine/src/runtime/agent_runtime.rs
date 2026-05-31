//! AgentRuntime — manages agent lifecycle (register, enable, disable,
//! execute) and tracks sessions.
//!
//! Mirrors the Python `runtime/agent_runtime.py`.  Runs fully in-memory;
//! persistence is handled by callers through the `db::repositories` layer.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::info;

use crate::utils::now_epoch;

use crate::core::agent_spec::AgentSpec;
use crate::core::events::EventEmitter;
use crate::core::runner::{ExecutionResult, ExecutionStatus};
use crate::core::state::SharedState;
use crate::utils::short_id;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),

    #[error("agent is disabled: {0}")]
    AgentDisabled(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

// ---------------------------------------------------------------------------
// RuntimeAgentStatus / RuntimeAgentRecord
// ---------------------------------------------------------------------------

/// Runtime lifecycle status of an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAgentStatus {
    Enabled,
    Disabled,
    /// PRD-008: Live agent actively cycling.
    Playing,
    Error(String),
}

impl std::fmt::Display for RuntimeAgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enabled => write!(f, "enabled"),
            Self::Disabled => write!(f, "disabled"),
            Self::Playing => write!(f, "playing"),
            Self::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// An agent registered in the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAgentRecord {
    pub id: String,
    pub spec: AgentSpec,
    pub status: RuntimeAgentStatus,
    pub registered_at: f64,
}

// ---------------------------------------------------------------------------
// CycleRecord (PRD-008)
// ---------------------------------------------------------------------------

/// Status of a single cycle execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleStatus {
    Running,
    Completed,
    Failed,
}

/// Record of a single cycle execution for a live agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleRecord {
    pub cycle_id: String,
    pub agent_id: String,
    pub cycle_number: u64,
    pub started_at: f64,
    pub completed_at: Option<f64>,
    pub status: CycleStatus,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// SessionRecord (runtime-local, not the DB one)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSessionRecord {
    pub id: String,
    pub agent_id: String,
    pub result: ExecutionResult,
    pub created_at: f64,
}

// ---------------------------------------------------------------------------
// AgentRuntime
// ---------------------------------------------------------------------------

/// Manages agent registration, lifecycle and in-memory execution.
pub struct AgentRuntime {
    agents: Arc<RwLock<HashMap<String, RuntimeAgentRecord>>>,
    sessions: Arc<RwLock<HashMap<String, RuntimeSessionRecord>>>,
    event_emitter: EventEmitter,
}

impl AgentRuntime {
    /// Create a new runtime with the given event emitter.
    pub fn new(event_emitter: EventEmitter) -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_emitter,
        }
    }

    /// Register an agent spec, returning the assigned ID.
    pub async fn register_agent(&self, spec: AgentSpec) -> Result<String, RuntimeError> {
        let id = short_id();

        let record = RuntimeAgentRecord {
            id: id.clone(),
            spec,
            status: RuntimeAgentStatus::Disabled,
            registered_at: now_epoch(),
        };

        let mut agents = self.agents.write().await;
        agents.insert(id.clone(), record);

        info!(agent_id = %id, "Agent registered");
        Ok(id)
    }

    /// Remove an agent from the runtime.
    pub async fn unregister_agent(&self, id: &str) -> Result<(), RuntimeError> {
        let mut agents = self.agents.write().await;
        if agents.remove(id).is_none() {
            return Err(RuntimeError::AgentNotFound(id.to_string()));
        }
        info!(agent_id = %id, "Agent unregistered");
        Ok(())
    }

    /// Enable an agent (allow execution).
    pub async fn enable_agent(&self, id: &str) -> Result<(), RuntimeError> {
        let mut agents = self.agents.write().await;
        match agents.get_mut(id) {
            Some(record) => {
                record.status = RuntimeAgentStatus::Enabled;
                info!(agent_id = %id, "Agent enabled");
                Ok(())
            }
            None => Err(RuntimeError::AgentNotFound(id.to_string())),
        }
    }

    /// Disable an agent (prevent execution).
    pub async fn disable_agent(&self, id: &str) -> Result<(), RuntimeError> {
        let mut agents = self.agents.write().await;
        match agents.get_mut(id) {
            Some(record) => {
                record.status = RuntimeAgentStatus::Disabled;
                info!(agent_id = %id, "Agent disabled");
                Ok(())
            }
            None => Err(RuntimeError::AgentNotFound(id.to_string())),
        }
    }

    /// Get a snapshot of a registered agent.
    pub async fn get_agent(&self, id: &str) -> Option<RuntimeAgentRecord> {
        let agents = self.agents.read().await;
        agents.get(id).cloned()
    }

    /// List all registered agents.
    pub async fn list_agents(&self) -> Vec<RuntimeAgentRecord> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    /// Execute an agent by ID.
    ///
    /// Constructs a `GraphDef` from the agent's spec, but since the
    /// `GraphRunner` requires a `ToolExecutor` that is set up externally,
    /// this method performs a *simulated* execution: it records the session
    /// with a `Completed` status and returns the result.  Full wiring with
    /// `GraphRunner` is done by the caller (e.g., the server layer).
    pub async fn execute_agent(
        &self,
        id: &str,
        trigger_payload: HashMap<String, Value>,
    ) -> Result<ExecutionResult, RuntimeError> {
        let agents = self.agents.read().await;
        let record = agents
            .get(id)
            .ok_or_else(|| RuntimeError::AgentNotFound(id.to_string()))?;

        if record.status == RuntimeAgentStatus::Disabled {
            return Err(RuntimeError::AgentDisabled(id.to_string()));
        }

        let _graph = record.spec.to_graph(None);

        // Build a minimal ExecutionResult.
        // In production the caller would wire a GraphRunner + ToolExecutor.
        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state: SharedState::new(),
            trace: vec![],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };

        let session_id = short_id();
        let session = RuntimeSessionRecord {
            id: session_id.clone(),
            agent_id: id.to_string(),
            result: result.clone(),
            created_at: now_epoch(),
        };

        // Emit event.
        let mut data = HashMap::new();
        data.insert("agent_id".to_string(), Value::String(id.to_string()));
        if !trigger_payload.is_empty() {
            data.insert(
                "trigger_payload".to_string(),
                serde_json::to_value(&trigger_payload).unwrap_or(Value::Null),
            );
        }
        self.event_emitter.emit(
            crate::core::events::EventType::SessionCompleted,
            session_id.clone(),
            None,
            data,
        );

        // Store session.
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, session);

        info!(agent_id = %id, "Agent executed");
        Ok(result)
    }

    /// Get a session by ID.
    pub async fn get_session(&self, id: &str) -> Option<RuntimeSessionRecord> {
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned()
    }

    /// List sessions, optionally filtered by agent_id.
    pub async fn list_sessions(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Vec<RuntimeSessionRecord> {
        let sessions = self.sessions.read().await;
        let mut items: Vec<RuntimeSessionRecord> = sessions
            .values()
            .filter(|s| match agent_id {
                Some(aid) => s.agent_id == aid,
                None => true,
            })
            .cloned()
            .collect();
        items.sort_by(|a, b| {
            b.created_at
                .partial_cmp(&a.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items.truncate(limit);
        items
    }

    /// Access the event emitter.
    pub fn event_emitter(&self) -> &EventEmitter {
        &self.event_emitter
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent_spec::{AgentConfig, AgentGraphSpec, AgentType};
    use crate::core::events::EventEmitter;

    fn make_spec(name: &str) -> AgentSpec {
        AgentSpec {
            name: name.to_string(),
            description: String::new(),
            version: "v1".to_string(),
            agent_type: AgentType::Managed,
            system_prompt: None,
            soul: None,
            inputs: None,
            outputs: None,
            graph: AgentGraphSpec::default(),
            schedule: None,
            triggers: vec![],
            config: AgentConfig::default(),
            resources: vec![],
            metadata: HashMap::new(),
        }
    }

    fn make_runtime() -> AgentRuntime {
        AgentRuntime::new(EventEmitter::new(16))
    }

    #[tokio::test]
    async fn register_and_get() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        let agent = rt.get_agent(&id).await;
        assert!(agent.is_some());
        assert_eq!(agent.unwrap().spec.name, "bot");
    }

    #[tokio::test]
    async fn unregister() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        rt.unregister_agent(&id).await.unwrap();
        assert!(rt.get_agent(&id).await.is_none());
    }

    #[tokio::test]
    async fn unregister_not_found() {
        let rt = make_runtime();
        let err = rt.unregister_agent("nope").await;
        assert!(matches!(err, Err(RuntimeError::AgentNotFound(_))));
    }

    #[tokio::test]
    async fn enable_disable() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();

        rt.enable_agent(&id).await.unwrap();
        let agent = rt.get_agent(&id).await.unwrap();
        assert_eq!(agent.status, RuntimeAgentStatus::Enabled);

        rt.disable_agent(&id).await.unwrap();
        let agent = rt.get_agent(&id).await.unwrap();
        assert_eq!(agent.status, RuntimeAgentStatus::Disabled);
    }

    #[tokio::test]
    async fn enable_not_found() {
        let rt = make_runtime();
        assert!(matches!(
            rt.enable_agent("ghost").await,
            Err(RuntimeError::AgentNotFound(_))
        ));
    }

    #[tokio::test]
    async fn list_agents() {
        let rt = make_runtime();
        rt.register_agent(make_spec("a")).await.unwrap();
        rt.register_agent(make_spec("b")).await.unwrap();
        assert_eq!(rt.list_agents().await.len(), 2);
    }

    #[tokio::test]
    async fn execute_disabled_agent_fails() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        let err = rt.execute_agent(&id, HashMap::new()).await;
        assert!(matches!(err, Err(RuntimeError::AgentDisabled(_))));
    }

    #[tokio::test]
    async fn execute_agent_success() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        rt.enable_agent(&id).await.unwrap();

        let result = rt.execute_agent(&id, HashMap::new()).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn execute_creates_session() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        rt.enable_agent(&id).await.unwrap();
        rt.execute_agent(&id, HashMap::new()).await.unwrap();

        let sessions = rt.list_sessions(Some(&id), 50).await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_id, id);
    }

    #[tokio::test]
    async fn list_sessions_filter() {
        let rt = make_runtime();
        let id1 = rt.register_agent(make_spec("a")).await.unwrap();
        let id2 = rt.register_agent(make_spec("b")).await.unwrap();
        rt.enable_agent(&id1).await.unwrap();
        rt.enable_agent(&id2).await.unwrap();
        rt.execute_agent(&id1, HashMap::new()).await.unwrap();
        rt.execute_agent(&id2, HashMap::new()).await.unwrap();
        rt.execute_agent(&id1, HashMap::new()).await.unwrap();

        assert_eq!(rt.list_sessions(None, 50).await.len(), 3);
        assert_eq!(rt.list_sessions(Some(&id1), 50).await.len(), 2);
        assert_eq!(rt.list_sessions(Some(&id2), 50).await.len(), 1);
    }

    #[tokio::test]
    async fn list_sessions_limit() {
        let rt = make_runtime();
        let id = rt.register_agent(make_spec("bot")).await.unwrap();
        rt.enable_agent(&id).await.unwrap();
        for _ in 0..5 {
            rt.execute_agent(&id, HashMap::new()).await.unwrap();
        }
        assert_eq!(rt.list_sessions(None, 3).await.len(), 3);
    }

    #[test]
    fn runtime_agent_status_display() {
        assert_eq!(RuntimeAgentStatus::Enabled.to_string(), "enabled");
        assert_eq!(RuntimeAgentStatus::Disabled.to_string(), "disabled");
        assert_eq!(
            RuntimeAgentStatus::Error("boom".into()).to_string(),
            "error: boom"
        );
    }

    #[test]
    fn runtime_error_display() {
        let e = RuntimeError::AgentNotFound("x".into());
        assert_eq!(format!("{}", e), "agent not found: x");
    }
}
