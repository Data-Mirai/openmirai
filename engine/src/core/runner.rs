use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::core::context::ExecutionContext;
use crate::core::events::{EventEmitter, EventType};
use crate::core::graph::{ComparisonOp, EdgeCondition, GraphDef, GraphError, NodeDef};
use crate::core::state::SharedState;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(transparent)]
    GraphError(#[from] GraphError),

    #[error("max iterations exceeded for node `{node_id}` ({visits} visits)")]
    MaxIterationsExceeded { node_id: String, visits: u32 },

    #[error("execution failed at node `{node_id}`: {message}")]
    ExecutionFailed { node_id: String, message: String },

    #[error("tool not found: `{tool_type}`")]
    ToolNotFound { tool_type: String },

    #[error("execution interrupted")]
    Interrupted,

    #[error("execution timed out")]
    Timeout,
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

/// Seven interception points during graph execution.
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn on_graph_start(
        &self,
        graph: &GraphDef,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn on_graph_end(
        &self,
        graph: &GraphDef,
        state: &SharedState,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn pre_block_exec(
        &self,
        node: &NodeDef,
        inputs: &mut HashMap<String, Value>,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn post_block_exec(
        &self,
        node: &NodeDef,
        output: &mut HashMap<String, Value>,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn pre_llm_call(
        &self,
        node: &NodeDef,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn post_llm_call(
        &self,
        node: &NodeDef,
        response: &mut Value,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;

    async fn on_error(
        &self,
        node: &NodeDef,
        error: &ToolError,
        ctx: &dyn ExecutionContext,
    ) -> HookResult;
}

// ---------------------------------------------------------------------------
// CheckpointCallback trait + Checkpoint
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

/// Callback invoked after each successful block to persist state.
#[async_trait]
pub trait CheckpointCallback: Send + Sync {
    async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError>;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackoffStrategy {
    None,
    Linear,
    Exponential,
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        BackoffStrategy::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureMode {
    Stop,
    Skip,
    RouteToError,
}

impl Default for FailureMode {
    fn default() -> Self {
        FailureMode::Stop
    }
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

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            backoff: BackoffStrategy::default(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub node_id: String,
    pub tool_type: String,
    /// One of: "ok", "error", "skipped"
    pub status: String,
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

// ---------------------------------------------------------------------------
// ToolExecutor trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single node.
    ///
    /// The runner provides the node definition, resolved inputs (from incoming
    /// edges / data_map) and the execution context.  The executor looks up the
    /// concrete tool implementation by `node.tool_type` and delegates to it.
    async fn execute(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}

// ---------------------------------------------------------------------------
// GraphRunner
// ---------------------------------------------------------------------------

/// Sequential cursor that traverses a validated DAG, executing nodes one at a
/// time.  Supports conditional branching, retry with backoff, template-based
/// input resolution, real-time event emission, hooks, checkpoints,
/// human_input interrupts, pause/resume, and transcript generation.
pub struct GraphRunner {
    executor: Box<dyn ToolExecutor>,
    event_emitter: Option<EventEmitter>,
    max_iterations: u32,
    default_retry_policy: RetryPolicy,
    hook_handler: Option<Box<dyn HookHandler>>,
    checkpoint_cb: Option<Box<dyn CheckpointCallback>>,
    pause_requested: Arc<AtomicBool>,
}

impl GraphRunner {
    /// Create a runner with the given tool executor.
    pub fn new(executor: Box<dyn ToolExecutor>) -> Self {
        Self {
            executor,
            event_emitter: None,
            max_iterations: 100,
            default_retry_policy: RetryPolicy::default(),
            hook_handler: None,
            checkpoint_cb: None,
            pause_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attach an event emitter (builder pattern).
    pub fn with_event_emitter(mut self, emitter: EventEmitter) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Override the default maximum visits per node (default: 100).
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    /// Override the default retry policy applied when a node has no explicit
    /// `retry_policy` in its config.
    pub fn with_default_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.default_retry_policy = policy;
        self
    }

    /// Attach a hook handler for execution interception.
    pub fn with_hook_handler(mut self, handler: Box<dyn HookHandler>) -> Self {
        self.hook_handler = Some(handler);
        self
    }

    /// Attach a checkpoint callback for state persistence.
    pub fn with_checkpoint_callback(mut self, cb: Box<dyn CheckpointCallback>) -> Self {
        self.checkpoint_cb = Some(cb);
        self
    }

    /// Request a pause at the next safe point (after current block finishes).
    pub fn request_pause(&self) {
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    /// Get a clone of the pause flag for external sharing.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.pause_requested.clone()
    }

    // -----------------------------------------------------------------------
    // Core execution loop
    // -----------------------------------------------------------------------

    /// Execute a graph to completion.
    ///
    /// # Algorithm
    ///
    /// 1. Validate graph structure.
    /// 2. Find entry node (first node with no incoming edges).
    /// 3. Walk the graph following edges, executing each node via the
    ///    `ToolExecutor`, until there are no more outgoing edges or an error
    ///    halts execution.
    pub async fn run(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
    ) -> Result<ExecutionResult, RunnerError> {
        // 1. Validate graph structure.
        graph.validate()?;

        // 2. Find entry node (no incoming edges).  Use first if multiple.
        let entry_nodes = graph.entry_nodes();
        let entry_node_id = entry_nodes
            .first()
            .expect("validated non-empty graph has at least one entry node")
            .id
            .clone();

        self.run_from(graph, context, SharedState::new(), &entry_node_id, 0)
            .await
    }

    /// Resume execution from a previously saved state.
    ///
    /// Starts the main loop from `resume_node_id` with the given state.
    pub async fn resume(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        resume_state: SharedState,
        resume_node_id: &str,
    ) -> Result<ExecutionResult, RunnerError> {
        graph.validate()?;
        self.run_from(graph, context, resume_state, resume_node_id, 0)
            .await
    }

    /// Internal: execute the main loop starting from a specific node/state.
    async fn run_from(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        state: SharedState,
        entry_node_id: &str,
        start_step: u32,
    ) -> Result<ExecutionResult, RunnerError> {
        let mut trace: Vec<TraceEntry> = Vec::new();
        let mut transcript: Vec<TranscriptEntry> = Vec::new();
        let mut visit_counts: HashMap<String, u32> = HashMap::new();
        let mut step = start_step;

        let mut current_idx: Option<usize> = node_index(graph, entry_node_id);

        let session_id = context.session_id().to_string();

        // Transcript: started
        transcript.push(TranscriptEntry {
            entry_type: "started".to_string(),
            message: format!("Ejecucion iniciada"),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });

        // Emit SessionStarted.
        self.emit_event(
            EventType::SessionStarted,
            &session_id,
            None,
            HashMap::new(),
        );

        info!(
            session_id = %session_id,
            graph_id = %graph.id,
            entry = %entry_node_id,
            "graph execution started"
        );

        // Hook: on_graph_start (with 30s timeout)
        if let Some(ref hook) = self.hook_handler {
            let hook_result = run_hook_with_timeout(
                hook.on_graph_start(graph, context),
            )
            .await;
            match hook_result {
                HookResult::Abort(reason) => {
                    return Ok(ExecutionResult {
                        status: ExecutionStatus::Failed,
                        state,
                        trace,
                        transcript,
                        error: Some(format!("Hook on_graph_start aborted: {}", reason)),
                        interrupt_node_id: None,
                        interrupt_info: None,
                    });
                }
                _ => {}
            }
        }

        // Main loop — walk the cursor until we run out of edges.
        while let Some(idx) = current_idx {
            let node = &graph.nodes[idx];
            let node_id = node.id.as_str();

            // Check for pause request.
            if self.pause_requested.swap(false, Ordering::SeqCst) {
                self.save_checkpoint(&session_id, step, node_id, &state, Some(node_id))
                    .await;
                self.emit_event(
                    EventType::SessionInterrupted,
                    &session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                return Ok(ExecutionResult {
                    status: ExecutionStatus::Interrupted,
                    state,
                    trace,
                    transcript,
                    error: None,
                    interrupt_node_id: Some(node_id.to_string()),
                    interrupt_info: None,
                });
            }

            // Guard: max iterations per node.
            let visits = visit_counts.entry(node.id.clone()).or_insert(0);
            *visits += 1;

            if *visits > self.max_iterations {
                error!(
                    node_id = %node_id,
                    visits = *visits,
                    "max iterations exceeded"
                );
                self.emit_event(
                    EventType::SessionFailed,
                    &session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                return Err(RunnerError::MaxIterationsExceeded {
                    node_id: node_id.to_string(),
                    visits: *visits,
                });
            }

            // Check if this is a human_input block — interrupt BEFORE execution.
            if node.tool_type == "logic/human_input" {
                let prompt = node
                    .config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Requiere decision")
                    .to_string();
                let options: Vec<String> = node
                    .config
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let timeout_minutes = node
                    .config
                    .get("timeout_minutes")
                    .and_then(|v| v.as_f64());

                let info = InterruptInfo {
                    node_id: node_id.to_string(),
                    prompt,
                    options,
                    timeout_minutes,
                };

                self.save_checkpoint(&session_id, step, node_id, &state, Some(node_id))
                    .await;
                self.emit_event(
                    EventType::InterruptCreated,
                    &session_id,
                    Some(node_id),
                    HashMap::new(),
                );

                return Ok(ExecutionResult {
                    status: ExecutionStatus::Interrupted,
                    state,
                    trace,
                    transcript,
                    error: None,
                    interrupt_node_id: Some(node_id.to_string()),
                    interrupt_info: Some(info),
                });
            }

            // Resolve inputs from incoming edges' data_map.
            let mut inputs = self.resolve_inputs(node_id, graph, &state);

            // Merge node.config as base layer — edge-resolved inputs take priority.
            for (key, val) in &node.config {
                inputs.entry(key.clone()).or_insert_with(|| val.clone());
            }

            debug!(
                node_id = %node_id,
                tool_type = %node.tool_type,
                input_keys = ?inputs.keys().collect::<Vec<_>>(),
                visit = *visits,
                "executing node"
            );

            // Hook: pre_block_exec (with 30s timeout)
            if let Some(ref hook) = self.hook_handler {
                let hook_result = run_hook_with_timeout(
                    hook.pre_block_exec(node, &mut inputs, context),
                )
                .await;
                match hook_result {
                    HookResult::Abort(reason) => {
                        return Ok(ExecutionResult {
                            status: ExecutionStatus::Failed,
                            state,
                            trace,
                            transcript,
                            error: Some(format!(
                                "Hook pre_block_exec aborted at '{}': {}",
                                node_id, reason
                            )),
                            interrupt_node_id: None,
                            interrupt_info: None,
                        });
                    }
                    HookResult::Skip => {
                        let empty: HashMap<String, Value> = HashMap::new();
                        let _ = state.set(node_id, empty, true);
                        current_idx = self
                            .resolve_next_node(node_id, &HashMap::new(), graph)
                            .as_deref()
                            .and_then(|nid| node_index(graph, nid));
                        step += 1;
                        continue;
                    }
                    HookResult::ModifiedInputs(new_inputs) => {
                        inputs = new_inputs;
                    }
                    _ => {}
                }
            }

            // Emit BlockStarted.
            self.emit_event(
                EventType::BlockStarted,
                &session_id,
                Some(node_id),
                HashMap::new(),
            );

            // Transcript: block starting
            transcript.push(TranscriptEntry {
                entry_type: "block_start".to_string(),
                message: format!("Ejecutando {}", node.tool_type),
                timestamp: now_ts(),
                node_id: Some(node_id.to_string()),
                metadata: HashMap::new(),
            });

            // Record start time.
            let start = Instant::now();

            // Hook: pre_llm_call for AI blocks
            if node.tool_type.starts_with("ai/") {
                if let Some(ref hook) = self.hook_handler {
                    let _ = run_hook_with_timeout(
                        hook.pre_llm_call(node, context),
                    )
                    .await;
                }
            }

            // Execute node with retry logic.
            let retry_policy = self.retry_policy_for(node);
            let exec_result = self
                .execute_with_retry(node, inputs, context, &retry_policy)
                .await;

            let elapsed_ms = start.elapsed().as_millis() as u64;

            // Branch on success / failure.
            current_idx = match exec_result {
                // ---- Success ----
                Ok((mut output, retries)) => {
                    // Hook: post_llm_call for AI blocks
                    if node.tool_type.starts_with("ai/") {
                        if let Some(ref hook) = self.hook_handler {
                            let mut response_val =
                                serde_json::to_value(&output).unwrap_or(Value::Null);
                            let _ = run_hook_with_timeout(
                                hook.post_llm_call(node, &mut response_val, context),
                            )
                            .await;
                        }
                    }

                    // Store output in shared state (overwrite for looping nodes).
                    if let Err(e) = state.set(node_id, output.clone(), true) {
                        warn!(node_id = %node_id, error = %e, "failed to set state");
                    }

                    trace.push(TraceEntry {
                        node_id: node_id.to_string(),
                        tool_type: node.tool_type.clone(),
                        status: "ok".to_string(),
                        duration_ms: elapsed_ms,
                        retries,
                        error: None,
                    });

                    // Hook: post_block_exec
                    if let Some(ref hook) = self.hook_handler {
                        let _ = run_hook_with_timeout(
                            hook.post_block_exec(node, &mut output, context),
                        )
                        .await;
                    }

                    // Emit BlockCompleted.
                    self.emit_event(
                        EventType::BlockCompleted,
                        &session_id,
                        Some(node_id),
                        HashMap::new(),
                    );

                    // Transcript: block completed
                    transcript.push(TranscriptEntry {
                        entry_type: "block_end".to_string(),
                        message: format!(
                            "Completado {} en {}ms",
                            node.tool_type, elapsed_ms
                        ),
                        timestamp: now_ts(),
                        node_id: Some(node_id.to_string()),
                        metadata: HashMap::new(),
                    });

                    // Checkpoint after successful execution (REGLA-12).
                    step += 1;
                    let next = self.resolve_next_node(node_id, &output, graph);

                    self.save_checkpoint(
                        &session_id,
                        step,
                        node_id,
                        &state,
                        next.as_deref(),
                    )
                    .await;

                    // Transcript: decision at conditional edges
                    let outgoing = graph.outgoing_edges(node_id);
                    let conditional: Vec<_> = outgoing
                        .iter()
                        .filter(|e| e.condition.is_some())
                        .collect();
                    if conditional.len() > 1 {
                        if let Some(ref next_id) = next {
                            // Find matching edge for metadata
                            let edge_id = outgoing
                                .iter()
                                .find(|e| e.target == *next_id)
                                .map(|e| e.id.as_str())
                                .unwrap_or("unknown");
                            let cond_desc = outgoing
                                .iter()
                                .find(|e| e.target == *next_id)
                                .and_then(|e| e.condition.as_ref())
                                .map(|c| {
                                    format!("{} {:?} {}", c.field, c.op, c.value)
                                })
                                .unwrap_or_default();

                            transcript.push(TranscriptEntry {
                                entry_type: "decision".to_string(),
                                message: format!(
                                    "Decision: siguiendo edge {} (condicion: {})",
                                    edge_id, cond_desc
                                ),
                                timestamp: now_ts(),
                                node_id: Some(node_id.to_string()),
                                metadata: HashMap::new(),
                            });
                        }
                    }

                    if let Some(ref nid) = next {
                        debug!(from = %node_id, to = %nid, "advancing to next node");
                    } else {
                        debug!(from = %node_id, "no outgoing edge — end of graph");
                    }
                    next.as_deref().and_then(|nid| node_index(graph, nid))
                }

                // ---- Failure ----
                Err((tool_err, retries)) => {
                    let err_msg = tool_err.to_string();

                    // Hook: on_error
                    let hook_action = if let Some(ref hook) = self.hook_handler {
                        run_hook_with_timeout(
                            hook.on_error(node, &tool_err, context),
                        )
                        .await
                    } else {
                        HookResult::Continue
                    };

                    // If hook says Retry, re-execute this node.
                    if matches!(hook_action, HookResult::Retry) {
                        // Transcript: error noted but retrying via hook
                        transcript.push(TranscriptEntry {
                            entry_type: "error".to_string(),
                            message: format!(
                                "Error en {}: {} (retrying via hook)",
                                node.tool_type, err_msg
                            ),
                            timestamp: now_ts(),
                            node_id: Some(node_id.to_string()),
                            metadata: HashMap::new(),
                        });
                        // Do NOT advance current_idx — re-enter the loop for this node.
                        current_idx = Some(idx);
                        continue;
                    }

                    // Transcript: error
                    transcript.push(TranscriptEntry {
                        entry_type: "error".to_string(),
                        message: format!("Error en {}: {}", node.tool_type, err_msg),
                        timestamp: now_ts(),
                        node_id: Some(node_id.to_string()),
                        metadata: HashMap::new(),
                    });

                    match retry_policy.on_failure {
                        FailureMode::Stop => {
                            trace.push(TraceEntry {
                                node_id: node_id.to_string(),
                                tool_type: node.tool_type.clone(),
                                status: "error".to_string(),
                                duration_ms: elapsed_ms,
                                retries,
                                error: Some(err_msg.clone()),
                            });

                            self.emit_event(
                                EventType::BlockError,
                                &session_id,
                                Some(node_id),
                                HashMap::new(),
                            );
                            self.emit_event(
                                EventType::SessionFailed,
                                &session_id,
                                None,
                                HashMap::new(),
                            );

                            error!(
                                node_id = %node_id,
                                error = %err_msg,
                                "node execution failed — stopping"
                            );

                            return Ok(ExecutionResult {
                                status: ExecutionStatus::Failed,
                                state,
                                trace,
                                transcript,
                                error: Some(err_msg),
                                interrupt_node_id: None,
                                interrupt_info: None,
                            });
                        }

                        FailureMode::Skip => {
                            warn!(
                                node_id = %node_id,
                                error = %err_msg,
                                "node execution failed — skipping"
                            );

                            let empty_output: HashMap<String, Value> = HashMap::new();
                            let _ = state.set(node_id, empty_output.clone(), true);

                            trace.push(TraceEntry {
                                node_id: node_id.to_string(),
                                tool_type: node.tool_type.clone(),
                                status: "skipped".to_string(),
                                duration_ms: elapsed_ms,
                                retries,
                                error: Some(err_msg),
                            });

                            self.emit_event(
                                EventType::BlockCompleted,
                                &session_id,
                                Some(node_id),
                                HashMap::new(),
                            );

                            // Continue with normal edge routing (empty output).
                            self.resolve_next_node(node_id, &empty_output, graph)
                                .as_deref()
                                .and_then(|nid| node_index(graph, nid))
                        }

                        FailureMode::RouteToError => {
                            warn!(
                                node_id = %node_id,
                                error = %err_msg,
                                "node execution failed — routing to error path"
                            );

                            let mut error_output: HashMap<String, Value> = HashMap::new();
                            error_output.insert(
                                "__error__".to_string(),
                                Value::String(err_msg.clone()),
                            );
                            let _ = state.set(node_id, error_output.clone(), true);

                            trace.push(TraceEntry {
                                node_id: node_id.to_string(),
                                tool_type: node.tool_type.clone(),
                                status: "error".to_string(),
                                duration_ms: elapsed_ms,
                                retries,
                                error: Some(err_msg),
                            });

                            self.emit_event(
                                EventType::BlockError,
                                &session_id,
                                Some(node_id),
                                HashMap::new(),
                            );

                            // Follow edges — conditional edges can match on __error__.
                            self.resolve_next_node(node_id, &error_output, graph)
                                .as_deref()
                                .and_then(|nid| node_index(graph, nid))
                        }
                    }
                }
            };
        }

        // Hook: on_graph_end
        if let Some(ref hook) = self.hook_handler {
            let _ = run_hook_with_timeout(
                hook.on_graph_end(graph, &state, context),
            )
            .await;
        }

        // Transcript: completed
        transcript.push(TranscriptEntry {
            entry_type: "completed".to_string(),
            message: format!("Ejecucion completada ({} bloques)", trace.len()),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });

        // Emit SessionCompleted.
        self.emit_event(
            EventType::SessionCompleted,
            &session_id,
            None,
            HashMap::new(),
        );

        info!(
            session_id = %session_id,
            nodes_executed = trace.len(),
            "graph execution completed"
        );

        // Return final result.
        Ok(ExecutionResult {
            status: ExecutionStatus::Completed,
            state,
            trace,
            transcript,
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        })
    }

    /// Internal: save checkpoint if callback is configured.
    async fn save_checkpoint(
        &self,
        session_id: &str,
        step: u32,
        node_id: &str,
        state: &SharedState,
        cursor_node_id: Option<&str>,
    ) {
        if let Some(ref cb) = self.checkpoint_cb {
            let checkpoint = Checkpoint {
                session_id: session_id.to_string(),
                step,
                node_id: node_id.to_string(),
                state_snapshot: state.snapshot(),
                cursor_node_id: cursor_node_id.map(String::from),
                timestamp: now_ts(),
            };
            match cb.save_checkpoint(checkpoint).await {
                Ok(checkpoint_id) => {
                    self.emit_event(
                        EventType::CheckpointCreated,
                        session_id,
                        Some(node_id),
                        {
                            let mut data = HashMap::new();
                            data.insert(
                                "checkpoint_id".to_string(),
                                Value::String(checkpoint_id),
                            );
                            data.insert(
                                "step".to_string(),
                                Value::Number(serde_json::Number::from(step)),
                            );
                            data
                        },
                    );
                }
                Err(e) => {
                    warn!(error = %e, "failed to save checkpoint");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Input resolution
    // -----------------------------------------------------------------------

    /// Resolve inputs for a target node by inspecting all incoming edges and
    /// their `data_map` definitions.
    ///
    /// Two expression forms are supported:
    ///
    /// * **Direct ref** — `"source_node.field"`: returns the raw `Value` from
    ///   the source node's output in state.
    /// * **Template** — a string containing `${source_node.field}` markers:
    ///   each marker is replaced with the stringified value and the whole
    ///   expression is returned as `Value::String`.
    fn resolve_inputs(
        &self,
        node_id: &str,
        graph: &GraphDef,
        state: &SharedState,
    ) -> HashMap<String, Value> {
        let mut inputs = HashMap::new();

        // Collect incoming edges for this node.
        let incoming: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.target == node_id)
            .collect();

        for edge in incoming {
            if let Some(ref data_map) = edge.data_map {
                for (target_param, source_expr) in data_map {
                    if let Some(val) = self.resolve_expression(source_expr, state) {
                        inputs.insert(target_param.clone(), val);
                    }
                }
            }
        }

        inputs
    }

    /// Resolve a single data_map expression against the current shared state.
    fn resolve_expression(&self, expr: &str, state: &SharedState) -> Option<Value> {
        // Regex for template markers: ${node_id.field}
        let template_re = Regex::new(r"\$\{([^.}]+)\.([^}]+)\}").expect("valid regex");

        if template_re.is_match(expr) {
            // Template mode: replace all ${node.field} occurrences.
            let mut result = expr.to_string();
            // Iterate over all captures (we re-find because replace needs owned data).
            for caps in template_re.captures_iter(expr) {
                let full_match = caps.get(0).unwrap().as_str();
                let src_node = &caps[1];
                let src_field = &caps[2];
                if let Some(val) = state.get_field(src_node, src_field) {
                    let replacement = match &val {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    result = result.replace(full_match, &replacement);
                }
            }
            Some(Value::String(result))
        } else if let Some((src_node, src_field)) = expr.split_once('.') {
            // Direct ref: "node_id.field"
            state.get_field(src_node, src_field)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Edge / condition evaluation
    // -----------------------------------------------------------------------

    /// Determine the next node by evaluating outgoing edges from `node_id`.
    ///
    /// 1. First conditional edge whose condition matches wins.
    /// 2. Fallback: first unconditional edge.
    /// 3. `None` if no outgoing edges (end of graph).
    fn resolve_next_node(
        &self,
        node_id: &str,
        output: &HashMap<String, Value>,
        graph: &GraphDef,
    ) -> Option<String> {
        let outgoing = graph.outgoing_edges(node_id);

        if outgoing.is_empty() {
            return None;
        }

        // First pass: conditional edges.
        for edge in &outgoing {
            if let Some(ref condition) = edge.condition {
                if Self::evaluate_condition(output, condition) {
                    return Some(edge.target.clone());
                }
            }
        }

        // Second pass: first unconditional edge.
        for edge in &outgoing {
            if edge.condition.is_none() {
                return Some(edge.target.clone());
            }
        }

        // All edges are conditional and none matched.
        None
    }

    /// Evaluate a single edge condition against a node's output map.
    fn evaluate_condition(output: &HashMap<String, Value>, condition: &EdgeCondition) -> bool {
        let actual = match output.get(&condition.field) {
            Some(v) => v,
            None => return false,
        };
        let expected = &condition.value;

        match condition.op {
            ComparisonOp::Eq => actual == expected,
            ComparisonOp::Neq => actual != expected,
            ComparisonOp::Gt => compare_numbers(actual, expected, |a, b| a > b),
            ComparisonOp::Lt => compare_numbers(actual, expected, |a, b| a < b),
            ComparisonOp::Gte => compare_numbers(actual, expected, |a, b| a >= b),
            ComparisonOp::Lte => compare_numbers(actual, expected, |a, b| a <= b),
            ComparisonOp::In => {
                // `actual` is IN `expected` (expected must be an array).
                match expected {
                    Value::Array(arr) => arr.contains(actual),
                    _ => false,
                }
            }
            ComparisonOp::Contains => {
                // `actual` string/array contains `expected`.
                match actual {
                    Value::String(s) => match expected {
                        Value::String(sub) => s.contains(sub.as_str()),
                        _ => false,
                    },
                    Value::Array(arr) => arr.contains(expected),
                    _ => false,
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Retry logic
    // -----------------------------------------------------------------------

    /// Execute a node, retrying according to the given policy.
    ///
    /// Returns `Ok((output, retries_used))` on success, or
    /// `Err((last_error, retries_used))` after all attempts are exhausted.
    async fn execute_with_retry(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
        policy: &RetryPolicy,
    ) -> Result<(HashMap<String, Value>, u32), (ToolError, u32)> {
        let total_attempts = 1 + policy.max_retries;
        let mut last_error: Option<ToolError> = None;

        for attempt in 0..total_attempts {
            if attempt > 0 {
                let delay = Self::calculate_backoff(attempt, policy);
                if !delay.is_zero() {
                    debug!(
                        node_id = %node.id,
                        attempt = attempt,
                        delay_ms = delay.as_millis() as u64,
                        "retrying after backoff"
                    );
                    tokio::time::sleep(delay).await;
                }
            }

            match self.executor.execute(node, inputs.clone(), context).await {
                Ok(output) => return Ok((output, attempt)),
                Err(e) => {
                    warn!(
                        node_id = %node.id,
                        attempt = attempt + 1,
                        total = total_attempts,
                        error = %e,
                        "tool execution attempt failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err((
            last_error.expect("at least one attempt must have been made"),
            policy.max_retries,
        ))
    }

    /// Calculate the backoff duration for a given retry attempt number.
    ///
    /// * `None` — always zero.
    /// * `Linear` — `initial_delay * attempt`.
    /// * `Exponential` — `initial_delay * 2^(attempt-1)`.
    fn calculate_backoff(retry: u32, policy: &RetryPolicy) -> Duration {
        let secs = match policy.backoff {
            BackoffStrategy::None => 0.0,
            BackoffStrategy::Linear => policy.initial_delay_secs * f64::from(retry),
            BackoffStrategy::Exponential => {
                policy.initial_delay_secs * 2.0_f64.powi(retry.saturating_sub(1) as i32)
            }
        };
        Duration::from_secs_f64(secs)
    }

    /// Look up a node's retry policy from its config map. Falls back to the
    /// runner-level default when not present or unparseable.
    fn retry_policy_for(&self, node: &NodeDef) -> RetryPolicy {
        node.config
            .get("retry_policy")
            .and_then(|v| serde_json::from_value::<RetryPolicy>(v.clone()).ok())
            .unwrap_or_else(|| self.default_retry_policy.clone())
    }

    // -----------------------------------------------------------------------
    // Event helper
    // -----------------------------------------------------------------------

    /// Emit an event if the emitter is attached; no-op otherwise.
    fn emit_event(
        &self,
        event_type: EventType,
        session_id: &str,
        node_id: Option<&str>,
        data: HashMap<String, Value>,
    ) {
        if let Some(ref emitter) = self.event_emitter {
            emitter.emit(
                event_type,
                session_id.to_string(),
                node_id.map(String::from),
                data,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Find the index of a node by id inside `graph.nodes`.
fn node_index(graph: &GraphDef, node_id: &str) -> Option<usize> {
    graph.nodes.iter().position(|n| n.id == node_id)
}

/// Try to extract f64 from two JSON values and apply a comparator.
fn compare_numbers(a: &Value, b: &Value, cmp: fn(f64, f64) -> bool) -> bool {
    match (as_f64(a), as_f64(b)) {
        (Some(x), Some(y)) => cmp(x, y),
        _ => false,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// Get current unix timestamp as f64.
fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Run a hook future with a 30-second timeout. On timeout, return Continue.
async fn run_hook_with_timeout<F>(fut: F) -> HookResult
where
    F: std::future::Future<Output = HookResult>,
{
    match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Ok(result) => result,
        Err(_) => {
            warn!("hook timed out after 30s — continuing");
            HookResult::Continue
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::{AuthContext, ExecutionContext, LLMResource, Role};
    use crate::core::graph::{EdgeCondition, EdgeDef, GraphDef, NodeDef};
    use serde_json::json;

    // -- Test helpers -------------------------------------------------------

    fn make_node(id: &str, tool_type: &str) -> NodeDef {
        NodeDef {
            id: id.to_string(),
            tool_type: tool_type.to_string(),
            version: "1.0.0".to_string(),
            config: HashMap::new(),
            position: None,
        }
    }

    fn make_edge(id: &str, source: &str, target: &str) -> EdgeDef {
        EdgeDef {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            condition: None,
            data_map: None,
        }
    }

    fn make_conditional_edge(
        id: &str,
        source: &str,
        target: &str,
        field: &str,
        op: ComparisonOp,
        value: Value,
    ) -> EdgeDef {
        EdgeDef {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            condition: Some(EdgeCondition {
                field: field.to_string(),
                op,
                value,
            }),
            data_map: None,
        }
    }

    // -- Stub executor: echoes node id + inputs back as output. -------------

    struct EchoExecutor;

    #[async_trait]
    impl ToolExecutor for EchoExecutor {
        async fn execute(
            &self,
            node: &NodeDef,
            inputs: HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> Result<HashMap<String, Value>, ToolError> {
            let mut output = inputs;
            output.insert("_node_id".to_string(), Value::String(node.id.clone()));
            Ok(output)
        }
    }

    // -- Stub executor: always fails. ---------------------------------------

    struct FailExecutor;

    #[async_trait]
    impl ToolExecutor for FailExecutor {
        async fn execute(
            &self,
            node: &NodeDef,
            _inputs: HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> Result<HashMap<String, Value>, ToolError> {
            Err(ToolError::ExecutionFailed {
                tool_type: node.tool_type.clone(),
                message: "always fails".to_string(),
            })
        }
    }

    // -- Stub executor: fails N times then succeeds. ------------------------

    struct FailNExecutor {
        fail_count: std::sync::atomic::AtomicU32,
        fail_limit: u32,
    }

    impl FailNExecutor {
        fn new(fail_limit: u32) -> Self {
            Self {
                fail_count: std::sync::atomic::AtomicU32::new(0),
                fail_limit,
            }
        }
    }

    #[async_trait]
    impl ToolExecutor for FailNExecutor {
        async fn execute(
            &self,
            node: &NodeDef,
            inputs: HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> Result<HashMap<String, Value>, ToolError> {
            let n = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_limit {
                Err(ToolError::ExecutionFailed {
                    tool_type: node.tool_type.clone(),
                    message: format!("fail #{}", n + 1),
                })
            } else {
                let mut output = inputs;
                output.insert("_node_id".to_string(), Value::String(node.id.clone()));
                Ok(output)
            }
        }
    }

    // -- Minimal LLM stub (required by ExecutionContext). --------------------

    struct StubLLM;

    #[async_trait]
    impl LLMResource for StubLLM {
        async fn call(
            &self,
            _model: &str,
            _prompt: &str,
            _context: &[serde_json::Value],
            _temperature: f64,
            _max_tokens: u32,
        ) -> Result<crate::core::context::LLMResponse, crate::core::context::ResourceError> {
            unimplemented!("stub")
        }

        async fn embed(
            &self,
            _text: &str,
            _model: &str,
        ) -> Result<Vec<f64>, crate::core::context::ResourceError> {
            unimplemented!("stub")
        }
    }

    // -- Minimal execution context for tests. -------------------------------

    struct TestContext {
        session_id: String,
        auth: AuthContext,
        llm: StubLLM,
    }

    impl TestContext {
        fn new() -> Self {
            Self {
                session_id: "test-session".to_string(),
                auth: AuthContext {
                    user_id: "test-user".into(),
                    role: Role::Owner,
                    universe_id: None,
                    environment_id: None,
                },
                llm: StubLLM,
            }
        }
    }

    impl ExecutionContext for TestContext {
        fn db(&self) -> Option<&dyn crate::core::context::DBResource> {
            None
        }
        fn llm(&self) -> &dyn LLMResource {
            &self.llm
        }
        fn storage(&self) -> Option<&dyn crate::core::context::StorageResource> {
            None
        }
        fn vector(&self) -> Option<&dyn crate::core::context::VectorResource> {
            None
        }
        fn auth(&self) -> &AuthContext {
            &self.auth
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn node_id(&self) -> Option<&str> {
            None
        }
        fn system_prompt(&self) -> Option<&str> {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn linear_graph_executes_all_nodes() {
        let graph = GraphDef {
            id: "g1".into(),
            name: "linear".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace.len(), 3);
        assert_eq!(result.trace[0].node_id, "a");
        assert_eq!(result.trace[1].node_id, "b");
        assert_eq!(result.trace[2].node_id, "c");
        assert!(result.trace.iter().all(|t| t.status == "ok"));
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn single_node_graph() {
        let graph = GraphDef {
            id: "g".into(),
            name: "single".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("only", "tool/echo")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].node_id, "only");
    }

    #[tokio::test]
    async fn conditional_edge_routes_correctly() {
        let graph = GraphDef {
            id: "g2".into(),
            name: "branching".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("start", "tool/echo"),
                make_node("yes_path", "tool/echo"),
                make_node("no_path", "tool/echo"),
            ],
            edges: vec![
                make_conditional_edge(
                    "e_yes",
                    "start",
                    "yes_path",
                    "_node_id",
                    ComparisonOp::Eq,
                    json!("start"),
                ),
                make_conditional_edge(
                    "e_no",
                    "start",
                    "no_path",
                    "_node_id",
                    ComparisonOp::Neq,
                    json!("start"),
                ),
            ],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace.len(), 2);
        // EchoExecutor puts _node_id = "start" → Eq matches → routes to yes_path
        assert_eq!(result.trace[1].node_id, "yes_path");
    }

    #[tokio::test]
    async fn unconditional_fallback_when_no_condition_matches() {
        let graph = GraphDef {
            id: "g".into(),
            name: "fallback".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("cond_target", "tool/echo"),
                make_node("fallback_target", "tool/echo"),
            ],
            edges: vec![
                // Conditional that won't match (EchoExecutor doesn't produce "status" field).
                make_conditional_edge(
                    "e_cond",
                    "a",
                    "cond_target",
                    "status",
                    ComparisonOp::Eq,
                    json!("special"),
                ),
                // Unconditional fallback.
                make_edge("e_fallback", "a", "fallback_target"),
            ],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.trace.len(), 2);
        assert_eq!(result.trace[1].node_id, "fallback_target");
    }

    #[tokio::test]
    async fn failure_mode_stop_returns_failed() {
        let graph = GraphDef {
            id: "g3".into(),
            name: "fail".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/fail")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(FailExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Failed);
        assert!(result.error.is_some());
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].status, "error");
    }

    #[tokio::test]
    async fn failure_mode_skip_continues() {
        let mut node_a = make_node("a", "tool/fail");
        node_a.config.insert(
            "retry_policy".to_string(),
            serde_json::to_value(RetryPolicy {
                max_retries: 0,
                backoff: BackoffStrategy::None,
                initial_delay_secs: 0.0,
                on_failure: FailureMode::Skip,
            })
            .unwrap(),
        );

        let graph = GraphDef {
            id: "g4".into(),
            name: "skip".into(),
            version: "1.0.0".into(),
            nodes: vec![node_a, make_node("b", "tool/fail")],
            edges: vec![make_edge("e1", "a", "b")],
            metadata: HashMap::new(),
        };

        // FailExecutor fails on both nodes.  Node "a" has Skip policy so runner
        // continues to "b".  Node "b" uses default (Stop) so it returns Failed.
        let runner = GraphRunner::new(Box::new(FailExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.trace[0].node_id, "a");
        assert_eq!(result.trace[0].status, "skipped");
        assert_eq!(result.trace.len(), 2);
        assert_eq!(result.trace[1].node_id, "b");
    }

    #[tokio::test]
    async fn failure_mode_route_to_error() {
        let mut node_a = make_node("a", "tool/fail");
        node_a.config.insert(
            "retry_policy".to_string(),
            serde_json::to_value(RetryPolicy {
                max_retries: 0,
                backoff: BackoffStrategy::None,
                initial_delay_secs: 0.0,
                on_failure: FailureMode::RouteToError,
            })
            .unwrap(),
        );

        let graph = GraphDef {
            id: "g".into(),
            name: "route_error".into(),
            version: "1.0.0".into(),
            nodes: vec![
                node_a,
                make_node("happy", "tool/echo"),
                make_node("error_handler", "tool/echo"),
            ],
            edges: vec![
                // Conditional edge: if __error__ field exists, route to error handler.
                EdgeDef {
                    id: "e_err".into(),
                    source: "a".into(),
                    target: "error_handler".into(),
                    condition: Some(EdgeCondition {
                        field: "__error__".to_string(),
                        op: ComparisonOp::Neq,
                        value: json!(null),
                    }),
                    data_map: None,
                },
                // Unconditional fallback to happy path.
                make_edge("e_happy", "a", "happy"),
            ],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(FailExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        // "a" fails with RouteToError → sets __error__ → conditional edge to error_handler matches.
        // error_handler also fails (FailExecutor) with default Stop policy.
        assert_eq!(result.trace[0].node_id, "a");
        assert_eq!(result.trace[0].status, "error");
        assert_eq!(result.trace[1].node_id, "error_handler");
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        let mut node = make_node("a", "tool/flaky");
        node.config.insert(
            "retry_policy".to_string(),
            serde_json::to_value(RetryPolicy {
                max_retries: 2,
                backoff: BackoffStrategy::None,
                initial_delay_secs: 0.0,
                on_failure: FailureMode::Stop,
            })
            .unwrap(),
        );

        let graph = GraphDef {
            id: "g".into(),
            name: "retry".into(),
            version: "1.0.0".into(),
            nodes: vec![node],
            edges: vec![],
            metadata: HashMap::new(),
        };

        // Fail once, then succeed.
        let runner = GraphRunner::new(Box::new(FailNExecutor::new(1)));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace[0].retries, 1); // succeeded on attempt index 1
        assert_eq!(result.trace[0].status, "ok");
    }

    #[tokio::test]
    async fn max_iterations_exceeded() {
        // 3-node graph: "entry" -> "b" -> "c" -> "b" (cycle between b and c).
        // "entry" has no incoming edges so it is the entry point.
        let graph = GraphDef {
            id: "g5".into(),
            name: "loop".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("entry", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![
                make_edge("e1", "entry", "b"),
                make_edge("e2", "b", "c"),
                make_edge("e3", "c", "b"),
            ],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor)).with_max_iterations(3);
        let ctx = TestContext::new();
        let err = runner.run(&graph, &ctx).await.unwrap_err();

        assert!(matches!(err, RunnerError::MaxIterationsExceeded { .. }));
    }

    #[tokio::test]
    async fn empty_graph_returns_graph_error() {
        let graph = GraphDef {
            id: "g".into(),
            name: "empty".into(),
            version: "1.0.0".into(),
            nodes: vec![],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let err = runner.run(&graph, &ctx).await.unwrap_err();

        assert!(matches!(err, RunnerError::GraphError(_)));
    }

    #[tokio::test]
    async fn data_map_passes_values_between_nodes() {
        let graph = GraphDef {
            id: "g".into(),
            name: "data_map".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo"), make_node("b", "tool/echo")],
            edges: vec![EdgeDef {
                id: "e1".into(),
                source: "a".into(),
                target: "b".into(),
                condition: None,
                data_map: Some({
                    let mut m = HashMap::new();
                    m.insert("prev_id".to_string(), "a._node_id".to_string());
                    m
                }),
            }],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        // Node "b" should have received prev_id = "a" from data_map.
        let b_output = result.state.get_field("b", "prev_id");
        assert_eq!(b_output, Some(json!("a")));
    }

    #[tokio::test]
    async fn event_emitter_receives_events() {
        let emitter = EventEmitter::new(64);
        let mut rx = emitter.subscribe();

        let graph = GraphDef {
            id: "g".into(),
            name: "events".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor)).with_event_emitter(emitter);
        let ctx = TestContext::new();
        runner.run(&graph, &ctx).await.unwrap();

        // Collect all emitted events.
        let mut types = Vec::new();
        while let Ok(evt) = rx.try_recv() {
            types.push(evt.event_type);
        }

        assert!(types.contains(&EventType::SessionStarted));
        assert!(types.contains(&EventType::BlockStarted));
        assert!(types.contains(&EventType::BlockCompleted));
        assert!(types.contains(&EventType::SessionCompleted));
    }

    // -----------------------------------------------------------------------
    // Unit tests — condition evaluation
    // -----------------------------------------------------------------------

    #[test]
    fn evaluate_condition_eq() {
        let mut output = HashMap::new();
        output.insert("status".to_string(), json!("done"));

        let cond = EdgeCondition {
            field: "status".to_string(),
            op: ComparisonOp::Eq,
            value: json!("done"),
        };
        assert!(GraphRunner::evaluate_condition(&output, &cond));
    }

    #[test]
    fn evaluate_condition_neq() {
        let mut output = HashMap::new();
        output.insert("status".to_string(), json!("pending"));

        let cond = EdgeCondition {
            field: "status".to_string(),
            op: ComparisonOp::Neq,
            value: json!("done"),
        };
        assert!(GraphRunner::evaluate_condition(&output, &cond));
    }

    #[test]
    fn evaluate_condition_gt() {
        let mut output = HashMap::new();
        output.insert("score".to_string(), json!(85));

        assert!(GraphRunner::evaluate_condition(
            &output,
            &EdgeCondition {
                field: "score".to_string(),
                op: ComparisonOp::Gt,
                value: json!(80),
            }
        ));
        assert!(!GraphRunner::evaluate_condition(
            &output,
            &EdgeCondition {
                field: "score".to_string(),
                op: ComparisonOp::Gt,
                value: json!(90),
            }
        ));
    }

    #[test]
    fn evaluate_condition_lt() {
        let mut output = HashMap::new();
        output.insert("temp".to_string(), json!(36.5));

        assert!(GraphRunner::evaluate_condition(
            &output,
            &EdgeCondition {
                field: "temp".to_string(),
                op: ComparisonOp::Lt,
                value: json!(37.0),
            }
        ));
    }

    #[test]
    fn evaluate_condition_gte_lte() {
        let mut output = HashMap::new();
        output.insert("v".to_string(), json!(10));

        assert!(GraphRunner::evaluate_condition(
            &output,
            &EdgeCondition {
                field: "v".to_string(),
                op: ComparisonOp::Gte,
                value: json!(10),
            }
        ));
        assert!(GraphRunner::evaluate_condition(
            &output,
            &EdgeCondition {
                field: "v".to_string(),
                op: ComparisonOp::Lte,
                value: json!(10),
            }
        ));
    }

    #[test]
    fn evaluate_condition_in() {
        let mut output = HashMap::new();
        output.insert("status".to_string(), json!("active"));

        let cond = EdgeCondition {
            field: "status".to_string(),
            op: ComparisonOp::In,
            value: json!(["active", "pending"]),
        };
        assert!(GraphRunner::evaluate_condition(&output, &cond));

        let cond_miss = EdgeCondition {
            field: "status".to_string(),
            op: ComparisonOp::In,
            value: json!(["closed"]),
        };
        assert!(!GraphRunner::evaluate_condition(&output, &cond_miss));
    }

    #[test]
    fn evaluate_condition_contains_string() {
        let mut output = HashMap::new();
        output.insert("text".to_string(), json!("hello world"));

        let cond = EdgeCondition {
            field: "text".to_string(),
            op: ComparisonOp::Contains,
            value: json!("world"),
        };
        assert!(GraphRunner::evaluate_condition(&output, &cond));
    }

    #[test]
    fn evaluate_condition_contains_array() {
        let mut output = HashMap::new();
        output.insert("tags".to_string(), json!(["alpha", "beta"]));

        let cond = EdgeCondition {
            field: "tags".to_string(),
            op: ComparisonOp::Contains,
            value: json!("beta"),
        };
        assert!(GraphRunner::evaluate_condition(&output, &cond));
    }

    #[test]
    fn evaluate_condition_missing_field_is_false() {
        let output = HashMap::new();
        let cond = EdgeCondition {
            field: "missing".to_string(),
            op: ComparisonOp::Eq,
            value: json!(true),
        };
        assert!(!GraphRunner::evaluate_condition(&output, &cond));
    }

    // -----------------------------------------------------------------------
    // Unit tests — backoff calculation
    // -----------------------------------------------------------------------

    #[test]
    fn calculate_backoff_none() {
        let policy = RetryPolicy {
            backoff: BackoffStrategy::None,
            initial_delay_secs: 1.0,
            ..RetryPolicy::default()
        };
        assert_eq!(GraphRunner::calculate_backoff(1, &policy), Duration::ZERO);
        assert_eq!(GraphRunner::calculate_backoff(5, &policy), Duration::ZERO);
    }

    #[test]
    fn calculate_backoff_linear() {
        let policy = RetryPolicy {
            backoff: BackoffStrategy::Linear,
            initial_delay_secs: 1.0,
            ..RetryPolicy::default()
        };
        assert_eq!(
            GraphRunner::calculate_backoff(1, &policy),
            Duration::from_secs(1)
        );
        assert_eq!(
            GraphRunner::calculate_backoff(3, &policy),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn calculate_backoff_exponential() {
        let policy = RetryPolicy {
            backoff: BackoffStrategy::Exponential,
            initial_delay_secs: 1.0,
            ..RetryPolicy::default()
        };
        // attempt 1 → 2^0 = 1s
        assert_eq!(
            GraphRunner::calculate_backoff(1, &policy),
            Duration::from_secs(1)
        );
        // attempt 2 → 2^1 = 2s
        assert_eq!(
            GraphRunner::calculate_backoff(2, &policy),
            Duration::from_secs(2)
        );
        // attempt 3 → 2^2 = 4s
        assert_eq!(
            GraphRunner::calculate_backoff(3, &policy),
            Duration::from_secs(4)
        );
    }

    // -----------------------------------------------------------------------
    // Unit tests — expression resolution
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_expression_direct_ref() {
        let state = SharedState::new();
        let mut out = HashMap::new();
        out.insert("text".to_string(), json!("hello"));
        state.set("node_a", out, false).unwrap();

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("node_a.text", &state);
        assert_eq!(val, Some(json!("hello")));
    }

    #[test]
    fn resolve_expression_template() {
        let state = SharedState::new();
        let mut out = HashMap::new();
        out.insert("name".to_string(), json!("world"));
        state.set("greeter", out, false).unwrap();

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("Hello, ${greeter.name}!", &state);
        assert_eq!(val, Some(json!("Hello, world!")));
    }

    #[test]
    fn resolve_expression_template_numeric() {
        let state = SharedState::new();
        let mut out = HashMap::new();
        out.insert("count".to_string(), json!(42));
        state.set("counter", out, false).unwrap();

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("Total: ${counter.count}", &state);
        assert_eq!(val, Some(json!("Total: 42")));
    }

    #[test]
    fn resolve_expression_multiple_templates() {
        let state = SharedState::new();
        let mut out_a = HashMap::new();
        out_a.insert("first".to_string(), json!("John"));
        state.set("a", out_a, false).unwrap();

        let mut out_b = HashMap::new();
        out_b.insert("last".to_string(), json!("Doe"));
        state.set("b", out_b, false).unwrap();

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("${a.first} ${b.last}", &state);
        assert_eq!(val, Some(json!("John Doe")));
    }

    #[test]
    fn resolve_expression_missing_returns_none() {
        let state = SharedState::new();
        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("ghost.field", &state);
        assert_eq!(val, None);
    }

    #[test]
    fn resolve_expression_no_dot_returns_none() {
        let state = SharedState::new();
        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let val = runner.resolve_expression("justanid", &state);
        assert_eq!(val, None);
    }

    // -----------------------------------------------------------------------
    // Serde roundtrips
    // -----------------------------------------------------------------------

    #[test]
    fn serde_roundtrip_trace_entry() {
        let entry = TraceEntry {
            node_id: "n1".to_string(),
            tool_type: "ai/llm_call".to_string(),
            status: "ok".to_string(),
            duration_ms: 150,
            retries: 0,
            error: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: TraceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.node_id, "n1");
        assert_eq!(back.duration_ms, 150);
    }

    #[test]
    fn serde_roundtrip_execution_result() {
        let result = ExecutionResult {
            status: ExecutionStatus::Completed,
            state: SharedState::new(),
            trace: vec![],
            transcript: vec![],
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, ExecutionStatus::Completed);
    }

    #[test]
    fn serde_roundtrip_retry_policy() {
        let policy = RetryPolicy {
            max_retries: 3,
            backoff: BackoffStrategy::Exponential,
            initial_delay_secs: 0.5,
            on_failure: FailureMode::RouteToError,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_retries, 3);
        assert!(matches!(back.backoff, BackoffStrategy::Exponential));
        assert!(matches!(back.on_failure, FailureMode::RouteToError));
    }

    // ===================================================================
    // Mock HookHandler for new feature tests
    // ===================================================================

    use std::sync::Mutex;

    /// Records which hooks were called and returns configurable results.
    struct MockHookHandler {
        calls: Arc<Mutex<Vec<String>>>,
        on_graph_start_result: Mutex<HookResult>,
        pre_block_exec_result: Mutex<HookResult>,
        post_block_exec_result: Mutex<HookResult>,
        on_error_result: Mutex<HookResult>,
    }

    impl MockHookHandler {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                on_graph_start_result: Mutex::new(HookResult::Continue),
                pre_block_exec_result: Mutex::new(HookResult::Continue),
                post_block_exec_result: Mutex::new(HookResult::Continue),
                on_error_result: Mutex::new(HookResult::Continue),
            }
        }

        fn with_on_graph_start(self, result: HookResult) -> Self {
            *self.on_graph_start_result.lock().unwrap() = result;
            self
        }

        fn with_pre_block_exec(self, result: HookResult) -> Self {
            *self.pre_block_exec_result.lock().unwrap() = result;
            self
        }

        fn with_on_error(self, result: HookResult) -> Self {
            *self.on_error_result.lock().unwrap() = result;
            self
        }

        fn call_log(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl HookHandler for MockHookHandler {
        async fn on_graph_start(
            &self,
            _graph: &GraphDef,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("on_graph_start".into());
            self.on_graph_start_result.lock().unwrap().clone()
        }

        async fn on_graph_end(
            &self,
            _graph: &GraphDef,
            _state: &SharedState,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("on_graph_end".into());
            HookResult::Continue
        }

        async fn pre_block_exec(
            &self,
            _node: &NodeDef,
            _inputs: &mut HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("pre_block_exec".into());
            self.pre_block_exec_result.lock().unwrap().clone()
        }

        async fn post_block_exec(
            &self,
            _node: &NodeDef,
            _output: &mut HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("post_block_exec".into());
            self.post_block_exec_result.lock().unwrap().clone()
        }

        async fn pre_llm_call(
            &self,
            _node: &NodeDef,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("pre_llm_call".into());
            HookResult::Continue
        }

        async fn post_llm_call(
            &self,
            _node: &NodeDef,
            _response: &mut Value,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("post_llm_call".into());
            HookResult::Continue
        }

        async fn on_error(
            &self,
            _node: &NodeDef,
            _error: &ToolError,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            self.calls.lock().unwrap().push("on_error".into());
            self.on_error_result.lock().unwrap().clone()
        }
    }

    // ===================================================================
    // MockCheckpointCallback
    // ===================================================================

    struct MockCheckpointCallback {
        checkpoints: Arc<Mutex<Vec<Checkpoint>>>,
    }

    impl MockCheckpointCallback {
        fn new() -> Self {
            Self {
                checkpoints: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn saved_checkpoints(&self) -> Vec<Checkpoint> {
            self.checkpoints.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CheckpointCallback for MockCheckpointCallback {
        async fn save_checkpoint(
            &self,
            checkpoint: Checkpoint,
        ) -> Result<String, RunnerError> {
            let id = format!("cp-{}", checkpoint.step);
            self.checkpoints.lock().unwrap().push(checkpoint);
            Ok(id)
        }
    }

    // ===================================================================
    // SlowHookHandler (for timeout test)
    // ===================================================================

    struct SlowHookHandler;

    #[async_trait]
    impl HookHandler for SlowHookHandler {
        async fn on_graph_start(
            &self,
            _graph: &GraphDef,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            // Sleep longer than the 30s timeout
            tokio::time::sleep(Duration::from_secs(35)).await;
            HookResult::Abort("should never reach this".to_string())
        }

        async fn on_graph_end(
            &self,
            _graph: &GraphDef,
            _state: &SharedState,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }

        async fn pre_block_exec(
            &self,
            _node: &NodeDef,
            _inputs: &mut HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }

        async fn post_block_exec(
            &self,
            _node: &NodeDef,
            _output: &mut HashMap<String, Value>,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }

        async fn pre_llm_call(
            &self,
            _node: &NodeDef,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }

        async fn post_llm_call(
            &self,
            _node: &NodeDef,
            _response: &mut Value,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }

        async fn on_error(
            &self,
            _node: &NodeDef,
            _error: &ToolError,
            _ctx: &dyn ExecutionContext,
        ) -> HookResult {
            HookResult::Continue
        }
    }

    // ===================================================================
    // Test 1: Hook on_graph_start aborts execution
    // ===================================================================

    #[tokio::test]
    async fn hook_on_graph_start_aborts() {
        let graph = GraphDef {
            id: "g".into(),
            name: "hook_abort".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let hook = MockHookHandler::new()
            .with_on_graph_start(HookResult::Abort("test abort".to_string()));

        let runner = GraphRunner::new(Box::new(EchoExecutor))
            .with_hook_handler(Box::new(hook));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Failed);
        assert!(result.error.unwrap().contains("test abort"));
        // No blocks should have executed
        assert_eq!(result.trace.len(), 0);
    }

    // ===================================================================
    // Test 2: Hook pre_block_exec skips a block
    // ===================================================================

    #[tokio::test]
    async fn hook_pre_block_exec_skips() {
        let graph = GraphDef {
            id: "g".into(),
            name: "hook_skip".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo"), make_node("b", "tool/echo")],
            edges: vec![make_edge("e1", "a", "b")],
            metadata: HashMap::new(),
        };

        let hook = MockHookHandler::new().with_pre_block_exec(HookResult::Skip);

        let runner = GraphRunner::new(Box::new(EchoExecutor))
            .with_hook_handler(Box::new(hook));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        // Both blocks are skipped, no trace entries from executor
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace.len(), 0);
    }

    // ===================================================================
    // Test 3: Hook pre_block_exec modifies inputs
    // ===================================================================

    #[tokio::test]
    async fn hook_pre_block_exec_modifies_inputs() {
        let graph = GraphDef {
            id: "g".into(),
            name: "hook_modify".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let mut injected = HashMap::new();
        injected.insert("injected_key".to_string(), json!("injected_value"));

        let hook =
            MockHookHandler::new().with_pre_block_exec(HookResult::ModifiedInputs(injected));

        let runner = GraphRunner::new(Box::new(EchoExecutor))
            .with_hook_handler(Box::new(hook));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        // EchoExecutor echoes inputs back, so "a" should have the injected key
        let val = result.state.get_field("a", "injected_key");
        assert_eq!(val, Some(json!("injected_value")));
    }

    // ===================================================================
    // Test 4: Hook on_error triggers retry
    // ===================================================================

    #[tokio::test]
    async fn hook_on_error_triggers_retry() {
        let graph = GraphDef {
            id: "g".into(),
            name: "hook_retry".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/flaky")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        // FailNExecutor fails once then succeeds. The hook tells the runner
        // to retry on error.
        let hook = MockHookHandler::new().with_on_error(HookResult::Retry);

        let runner = GraphRunner::new(Box::new(FailNExecutor::new(1)))
            .with_hook_handler(Box::new(hook));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        // The hook-triggered retry should have made it succeed on the second visit
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert!(result.trace.iter().any(|t| t.status == "ok"));
    }

    // ===================================================================
    // Test 5: Hook timeout (30s) — use tokio::time::pause
    // ===================================================================

    #[tokio::test]
    async fn hook_timeout_returns_continue() {
        // Use tokio::time::pause for instant-advancing time.
        tokio::time::pause();

        let graph = GraphDef {
            id: "g".into(),
            name: "hook_timeout".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo")],
            edges: vec![],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor))
            .with_hook_handler(Box::new(SlowHookHandler));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        // SlowHookHandler sleeps 35s in on_graph_start.
        // Timeout fires at 30s → HookResult::Continue → execution proceeds.
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.trace.len(), 1);
    }

    // ===================================================================
    // Test 6: Checkpoint callback is called after each block
    // ===================================================================

    #[tokio::test]
    async fn checkpoint_called_after_each_block() {
        let graph = GraphDef {
            id: "g".into(),
            name: "checkpoint".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
            metadata: HashMap::new(),
        };

        let cp_cb = Arc::new(MockCheckpointCallback::new());
        // We need an Arc-shareable wrapper because we want to inspect after run().
        // Wrap in a struct that forwards:
        struct ArcCb(Arc<MockCheckpointCallback>);

        #[async_trait]
        impl CheckpointCallback for ArcCb {
            async fn save_checkpoint(
                &self,
                checkpoint: Checkpoint,
            ) -> Result<String, RunnerError> {
                self.0.save_checkpoint(checkpoint).await
            }
        }

        let runner = GraphRunner::new(Box::new(EchoExecutor))
            .with_checkpoint_callback(Box::new(ArcCb(cp_cb.clone())));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        let saved = cp_cb.saved_checkpoints();
        // One checkpoint per successfully executed block (3 blocks = 3 checkpoints)
        assert_eq!(saved.len(), 3);
        assert_eq!(saved[0].node_id, "a");
        assert_eq!(saved[1].node_id, "b");
        assert_eq!(saved[2].node_id, "c");
    }

    // ===================================================================
    // Test 7: Human input interrupt returns correct status + info
    // ===================================================================

    #[tokio::test]
    async fn human_input_interrupts_execution() {
        let mut hi_node = make_node("hi", "logic/human_input");
        hi_node
            .config
            .insert("prompt".to_string(), json!("Pick one"));
        hi_node
            .config
            .insert("options".to_string(), json!(["yes", "no"]));
        hi_node
            .config
            .insert("timeout_minutes".to_string(), json!(5.0));

        let graph = GraphDef {
            id: "g".into(),
            name: "human_input".into(),
            version: "1.0.0".into(),
            nodes: vec![make_node("a", "tool/echo"), hi_node, make_node("c", "tool/echo")],
            edges: vec![make_edge("e1", "a", "hi"), make_edge("e2", "hi", "c")],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Interrupted);
        assert_eq!(result.interrupt_node_id, Some("hi".to_string()));

        let info = result.interrupt_info.unwrap();
        assert_eq!(info.node_id, "hi");
        assert_eq!(info.prompt, "Pick one");
        assert_eq!(info.options, vec!["yes", "no"]);
        assert_eq!(info.timeout_minutes, Some(5.0));
        // Only "a" should have been executed (before the interrupt)
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].node_id, "a");
    }

    // ===================================================================
    // Test 8: Pause request interrupts execution
    // ===================================================================

    #[tokio::test]
    async fn pause_request_interrupts() {
        let graph = GraphDef {
            id: "g".into(),
            name: "pause".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        // Request pause before run() — will trigger at the first node
        runner.request_pause();
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Interrupted);
        assert_eq!(result.interrupt_node_id, Some("a".to_string()));
        // No blocks executed because pause fires before the first execution
        assert_eq!(result.trace.len(), 0);
    }

    // ===================================================================
    // Test 9: Resume continues from checkpoint state
    // ===================================================================

    #[tokio::test]
    async fn resume_continues_from_state() {
        let graph = GraphDef {
            id: "g".into(),
            name: "resume".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
            metadata: HashMap::new(),
        };

        // Simulate: "a" already completed, resume from "b"
        let resume_state = SharedState::new();
        let mut a_output = HashMap::new();
        a_output.insert("_node_id".to_string(), json!("a"));
        resume_state.set("a", a_output, false).unwrap();

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner
            .resume(&graph, &ctx, resume_state, "b")
            .await
            .unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);
        // Only "b" and "c" should have been executed in this run
        assert_eq!(result.trace.len(), 2);
        assert_eq!(result.trace[0].node_id, "b");
        assert_eq!(result.trace[1].node_id, "c");
        // State should contain all three nodes
        assert!(result.state.get("a").is_some());
        assert!(result.state.get("b").is_some());
        assert!(result.state.get("c").is_some());
    }

    // ===================================================================
    // Test 10: Transcript contains all event types
    // ===================================================================

    #[tokio::test]
    async fn transcript_contains_all_event_types() {
        // Build a graph with a conditional branch to get a "decision" entry
        let graph = GraphDef {
            id: "g".into(),
            name: "transcript".into(),
            version: "1.0.0".into(),
            nodes: vec![
                make_node("a", "tool/echo"),
                make_node("b", "tool/echo"),
                make_node("c", "tool/echo"),
            ],
            edges: vec![
                // Two conditional edges from "a" to force a decision transcript
                make_conditional_edge(
                    "e_yes",
                    "a",
                    "b",
                    "_node_id",
                    ComparisonOp::Eq,
                    json!("a"),
                ),
                make_conditional_edge(
                    "e_no",
                    "a",
                    "c",
                    "_node_id",
                    ComparisonOp::Neq,
                    json!("a"),
                ),
            ],
            metadata: HashMap::new(),
        };

        let runner = GraphRunner::new(Box::new(EchoExecutor));
        let ctx = TestContext::new();
        let result = runner.run(&graph, &ctx).await.unwrap();

        assert_eq!(result.status, ExecutionStatus::Completed);

        let types: Vec<&str> = result
            .transcript
            .iter()
            .map(|t| t.entry_type.as_str())
            .collect();

        assert!(types.contains(&"started"), "missing 'started'");
        assert!(types.contains(&"block_start"), "missing 'block_start'");
        assert!(types.contains(&"block_end"), "missing 'block_end'");
        assert!(types.contains(&"decision"), "missing 'decision'");
        assert!(types.contains(&"completed"), "missing 'completed'");
    }
}
