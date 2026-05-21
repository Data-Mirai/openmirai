use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_node_id: Option<String>,
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
/// input resolution, and real-time event emission.
pub struct GraphRunner {
    executor: Box<dyn ToolExecutor>,
    event_emitter: Option<EventEmitter>,
    max_iterations: u32,
    default_retry_policy: RetryPolicy,
}

impl GraphRunner {
    /// Create a runner with the given tool executor.
    pub fn new(executor: Box<dyn ToolExecutor>) -> Self {
        Self {
            executor,
            event_emitter: None,
            max_iterations: 100,
            default_retry_policy: RetryPolicy::default(),
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
            .as_str();

        // 3. Initialize working state.
        let state = SharedState::new();
        let mut trace: Vec<TraceEntry> = Vec::new();
        let mut visit_counts: HashMap<String, u32> = HashMap::new();

        // We keep the current position as an index into graph.nodes so the
        // borrow checker is happy (no lifetime tangles with owned Strings).
        let mut current_idx: Option<usize> = node_index(graph, entry_node_id);

        let session_id = context.session_id().to_string();

        // 4. Emit SessionStarted.
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

        // 5. Main loop — walk the cursor until we run out of edges.
        while let Some(idx) = current_idx {
            let node = &graph.nodes[idx];
            let node_id = node.id.as_str();

            // 5a. Guard: max iterations per node.
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

            // 5c. Resolve inputs from incoming edges' data_map.
            let inputs = self.resolve_inputs(node_id, graph, &state);

            debug!(
                node_id = %node_id,
                tool_type = %node.tool_type,
                input_keys = ?inputs.keys().collect::<Vec<_>>(),
                visit = *visits,
                "executing node"
            );

            // 5d. Emit BlockStarted.
            self.emit_event(
                EventType::BlockStarted,
                &session_id,
                Some(node_id),
                HashMap::new(),
            );

            // 5e. Record start time.
            let start = Instant::now();

            // 5f. Execute node with retry logic.
            let retry_policy = self.retry_policy_for(node);
            let exec_result = self
                .execute_with_retry(node, inputs, context, &retry_policy)
                .await;

            let elapsed_ms = start.elapsed().as_millis() as u64;

            // Branch on success / failure.
            current_idx = match exec_result {
                // ---- Success ----
                Ok((output, retries)) => {
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

                    // 5g. Emit BlockCompleted.
                    self.emit_event(
                        EventType::BlockCompleted,
                        &session_id,
                        Some(node_id),
                        HashMap::new(),
                    );

                    // 5h. Resolve next node.
                    let next = self.resolve_next_node(node_id, &output, graph);
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
                                error: Some(err_msg),
                                interrupt_node_id: None,
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

        // 6. Emit SessionCompleted.
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

        // 7. Return final result.
        Ok(ExecutionResult {
            status: ExecutionStatus::Completed,
            state,
            trace,
            error: None,
            interrupt_node_id: None,
        })
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
            error: None,
            interrupt_node_id: None,
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
}
