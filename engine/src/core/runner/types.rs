//! Type definitions for the graph execution engine.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::core::graph::GraphError;
use crate::core::state::SharedState;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(transparent)]
    GraphError(#[from] GraphError),

    #[error("max iterations exceeded for node `{node_id}` ({visits} visits)")]
    MaxIterationsExceeded { node_id: String, visits: u32 },

    #[error("execution failed at node `{node_id}`")]
    ExecutionFailed {
        node_id: String,
        #[source]
        source: ToolError,
    },

    #[error("tool not found: `{tool_type}`")]
    ToolNotFound { tool_type: String },

    #[error("execution interrupted")]
    Interrupted,

    #[error("execution timed out")]
    Timeout,

    #[error("{context}: {message}")]
    Internal { context: String, message: String },
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool not found: `{tool_type}`")]
    NotFound { tool_type: String },

    #[error("tool `{tool_type}` execution failed: {message}")]
    ExecutionFailed { tool_type: String, message: String },
}

// ---------------------------------------------------------------------------
// HookHandler trait + HookResult
// ---------------------------------------------------------------------------

/// Result returned by hook methods to control execution flow.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Proceed normally.
    Continue,
    /// Skip this block (pre_block_exec only).
    Skip,
    /// Abort execution with a reason message.
    Abort(String),
    /// Retry the current block (on_error only).
    Retry,
    /// Replace the inputs for this block (pre_block_exec only).
    ModifiedInputs(HashMap<String, Value>),
}

// ---------------------------------------------------------------------------
// Checkpoint
// ---------------------------------------------------------------------------

/// Snapshot of execution state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session_id: String,
    pub step: u32,
    pub node_id: String,
    pub state_snapshot: HashMap<String, HashMap<String, Value>>,
    pub cursor_node_id: Option<String>,
    pub timestamp: f64,
}

// ---------------------------------------------------------------------------
// InterruptInfo
// ---------------------------------------------------------------------------

/// Information about a human_input interrupt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptInfo {
    pub node_id: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub timeout_minutes: Option<f64>,
}

// ---------------------------------------------------------------------------
// TranscriptEntry
// ---------------------------------------------------------------------------

/// Human-readable log entry generated during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub entry_type: String,
    pub message: String,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum BackoffStrategy {
    #[default]
    None,
    Linear,
    Exponential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum FailureMode {
    #[default]
    Stop,
    Skip,
    RouteToError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub backoff: BackoffStrategy,
    #[serde(default = "default_initial_delay")]
    pub initial_delay_secs: f64,
    #[serde(default)]
    pub on_failure: FailureMode,
}

fn default_initial_delay() -> f64 {
    1.0
}

/// Default: 3 retries with exponential backoff — industry standard for
/// transient failures.  Aligned with `AgentRetryConfig::default()`.
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: BackoffStrategy::Exponential,
            initial_delay_secs: default_initial_delay(),
            on_failure: FailureMode::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// ExecutionResult / TraceEntry / ExecutionStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Completed,
    Failed,
    Timeout,
    Interrupted,
}

/// Status of a single node execution within a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Ok,
    Error,
    Skipped,
}

impl std::fmt::Display for TraceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => write!(f, "ok"),
            Self::Error => write!(f, "error"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub node_id: String,
    pub tool_type: String,
    pub status: TraceStatus,
    pub duration_ms: u64,
    pub retries: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub state: SharedState,
    pub trace: Vec<TraceEntry>,
    pub transcript: Vec<TranscriptEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_info: Option<InterruptInfo>,
}
