#![allow(dead_code)]
#![allow(unused_imports)]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::core::context::{AuthContext, ExecutionContext, LLMResource, Role};
use crate::core::events::{EventEmitter, EventType};
use crate::core::graph::{ComparisonOp, EdgeCondition, EdgeDef, GraphDef, NodeDef};
use crate::core::state::SharedState;
use crate::core::well_known as wk;

use super::{
    BackoffStrategy, Checkpoint, CheckpointCallback, ExecutionResult, ExecutionStatus, FailureMode,
    GraphRunner, HookHandler, HookResult, InterruptInfo, RetryPolicy, RunnerError, ToolError,
    ToolExecutor, TraceEntry, TraceStatus, TranscriptEntry,
};

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
        _max_tokens: Option<u32>,
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
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(result.trace.len(), 3);
    assert_eq!(result.trace[0].node_id, "a");
    assert_eq!(result.trace[1].node_id, "b");
    assert_eq!(result.trace[2].node_id, "c");
    assert!(result.trace.iter().all(|t| t.status == TraceStatus::Ok));
    assert!(result.error.is_none());
}

#[tokio::test]
async fn trace_entries_carry_real_timestamps() {
    // 0.7.0: cada TraceEntry lleva started_at/finished_at reales (unix epoch),
    // la base de la trazabilidad persistente y del export OTel fiel.
    let graph = GraphDef {
        id: "g-ts".into(),
        name: "timestamps".into(),
        version: "1.0.0".into(),
        nodes: vec![make_node("a", "tool/echo"), make_node("b", "tool/echo")],
        edges: vec![make_edge("e1", "a", "b")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    for entry in &result.trace {
        assert!(
            entry.started_at >= before - 1.0 && entry.finished_at <= after + 1.0,
            "timestamps del nodo '{}' fuera de la ventana de ejecución: {} / {}",
            entry.node_id,
            entry.started_at,
            entry.finished_at
        );
        assert!(
            entry.finished_at >= entry.started_at,
            "finished_at < started_at en '{}'",
            entry.node_id
        );
    }
    // Orden temporal: b no puede terminar antes de que a empiece.
    assert!(result.trace[1].finished_at >= result.trace[0].started_at);
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
        strict_completion: false,
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
        strict_completion: false,
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
        strict_completion: false,
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
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(FailExecutor)).with_default_retry_policy(RetryPolicy {
        max_retries: 0,
        backoff: BackoffStrategy::None,
        initial_delay_secs: 0.0,
        on_failure: FailureMode::Stop,
    });
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    assert!(result.error.is_some());
    assert_eq!(result.trace.len(), 1);
    assert_eq!(result.trace[0].status, TraceStatus::Error);
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
        strict_completion: false,
    };

    // FailExecutor fails on both nodes.  Node "a" has Skip policy so runner
    // continues to "b".  Node "b" uses default (Stop) so it returns Failed.
    let runner = GraphRunner::new(Box::new(FailExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.trace[0].node_id, "a");
    assert_eq!(result.trace[0].status, TraceStatus::Skipped);
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
                    field: wk::ERROR_FIELD.to_string(),
                    op: ComparisonOp::Neq,
                    value: json!(null),
                }),
                data_map: None,
            },
            // Unconditional fallback to happy path.
            make_edge("e_happy", "a", "happy"),
        ],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(FailExecutor)).with_default_retry_policy(RetryPolicy {
        max_retries: 0,
        backoff: BackoffStrategy::None,
        initial_delay_secs: 0.0,
        on_failure: FailureMode::Stop,
    });
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    // "a" fails with RouteToError → sets __error__ → conditional edge to error_handler matches.
    // error_handler also fails (FailExecutor) with default Stop policy (set via runner default).
    assert_eq!(result.trace[0].node_id, "a");
    assert_eq!(result.trace[0].status, TraceStatus::Error);
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
        strict_completion: false,
    };

    // Fail once, then succeed.
    let runner = GraphRunner::new(Box::new(FailNExecutor::new(1)));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(result.trace[0].retries, 1); // succeeded on attempt index 1
    assert_eq!(result.trace[0].status, TraceStatus::Ok);
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
        strict_completion: false,
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
        strict_completion: false,
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
        strict_completion: false,
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
        strict_completion: false,
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
        status: TraceStatus::Ok,
        duration_ms: 150,
        retries: 0,
        started_at: 0.0,
        finished_at: 0.0,
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
    async fn on_graph_start(&self, _graph: &GraphDef, _ctx: &dyn ExecutionContext) -> HookResult {
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

    async fn pre_llm_call(&self, _node: &NodeDef, _ctx: &dyn ExecutionContext) -> HookResult {
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
    async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError> {
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
    async fn on_graph_start(&self, _graph: &GraphDef, _ctx: &dyn ExecutionContext) -> HookResult {
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

    async fn pre_llm_call(&self, _node: &NodeDef, _ctx: &dyn ExecutionContext) -> HookResult {
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
        strict_completion: false,
    };

    let hook =
        MockHookHandler::new().with_on_graph_start(HookResult::Abort("test abort".to_string()));

    let runner = GraphRunner::new(Box::new(EchoExecutor)).with_hook_handler(Box::new(hook));
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
        strict_completion: false,
    };

    let hook = MockHookHandler::new().with_pre_block_exec(HookResult::Skip);

    let runner = GraphRunner::new(Box::new(EchoExecutor)).with_hook_handler(Box::new(hook));
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
        strict_completion: false,
    };

    let mut injected = HashMap::new();
    injected.insert("injected_key".to_string(), json!("injected_value"));

    let hook = MockHookHandler::new().with_pre_block_exec(HookResult::ModifiedInputs(injected));

    let runner = GraphRunner::new(Box::new(EchoExecutor)).with_hook_handler(Box::new(hook));
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
        strict_completion: false,
    };

    // FailNExecutor fails once then succeeds. The hook tells the runner
    // to retry on error.
    let hook = MockHookHandler::new().with_on_error(HookResult::Retry);

    let runner =
        GraphRunner::new(Box::new(FailNExecutor::new(1))).with_hook_handler(Box::new(hook));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    // The hook-triggered retry should have made it succeed on the second visit
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert!(result.trace.iter().any(|t| t.status == TraceStatus::Ok));
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
        strict_completion: false,
    };

    let runner =
        GraphRunner::new(Box::new(EchoExecutor)).with_hook_handler(Box::new(SlowHookHandler));
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
        strict_completion: false,
    };

    let cp_cb = Arc::new(MockCheckpointCallback::new());
    // We need an Arc-shareable wrapper because we want to inspect after run().
    // Wrap in a struct that forwards:
    struct ArcCb(Arc<MockCheckpointCallback>);

    #[async_trait]
    impl CheckpointCallback for ArcCb {
        async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError> {
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

    // PRD-021-A: cada checkpoint lleva QUÉ nodos ya corrieron — sin eso,
    // reanudar los vuelve a ejecutar y repite sus efectos.
    assert_eq!(saved[0].executed_nodes, vec!["a"]);
    assert_eq!(saved[1].executed_nodes, vec!["a", "b"]);
    assert_eq!(saved[2].executed_nodes, vec!["a", "b", "c"]);
    // Y dónde retomar: tras 'a' sigue 'b'; tras el último no queda nada.
    assert_eq!(saved[0].cursor_node_id.as_deref(), Some("b"));
    assert_eq!(saved[2].cursor_node_id, None);
}

// ===================================================================
// Test 6b: el nodo que FALLÓ no cuenta como ejecutado (PRD-021-A)
// ===================================================================

/// Falla solo en un nodo concreto; el resto pasa.
struct FailOneNodeExecutor {
    node_id: &'static str,
}

#[async_trait]
impl ToolExecutor for FailOneNodeExecutor {
    async fn execute(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        _ctx: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        if node.id == self.node_id {
            return Err(ToolError::ExecutionFailed {
                tool_type: node.tool_type.clone(),
                message: "boom".to_string(),
            });
        }
        let mut output = inputs;
        output.insert("_node_id".to_string(), Value::String(node.id.clone()));
        Ok(output)
    }
}

#[tokio::test]
async fn checkpoint_no_marca_como_ejecutado_el_nodo_que_fallo() {
    // Reanudar tiene que arrancar EN el nodo que falló: si el fallido contara
    // como ejecutado, la reanudación se lo saltaría y ese trabajo nunca se
    // haría. (Los que se saltan a propósito — Skipped — SÍ cuentan.)
    let graph = GraphDef {
        id: "g".into(),
        name: "route-error".into(),
        version: "1.0.0".into(),
        nodes: vec![
            make_node("a", "tool/echo"),
            make_node("b", "tool/echo"),
            make_node("c", "tool/echo"),
        ],
        edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let cp_cb = Arc::new(MockCheckpointCallback::new());
    struct ArcCb(Arc<MockCheckpointCallback>);
    #[async_trait]
    impl CheckpointCallback for ArcCb {
        async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError> {
            self.0.save_checkpoint(checkpoint).await
        }
    }

    let runner = GraphRunner::new(Box::new(FailOneNodeExecutor { node_id: "b" }))
        .with_default_retry_policy(RetryPolicy {
            max_retries: 0,
            backoff: BackoffStrategy::None,
            initial_delay_secs: 0.0,
            on_failure: FailureMode::RouteToError,
        })
        .with_checkpoint_callback(Box::new(ArcCb(cp_cb.clone())));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    // 'b' quedó en la traza como Error; 'a' y 'c' como Ok.
    let b_trace = result.trace.iter().find(|t| t.node_id == "b").unwrap();
    assert_eq!(b_trace.status, TraceStatus::Error);

    let saved = cp_cb.saved_checkpoints();
    let ultimo = saved.last().expect("al menos un checkpoint");
    assert_eq!(
        ultimo.executed_nodes,
        vec!["a", "c"],
        "el nodo en error NO puede contar como ejecutado"
    );
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
        nodes: vec![
            make_node("a", "tool/echo"),
            hi_node,
            make_node("c", "tool/echo"),
        ],
        edges: vec![make_edge("e1", "a", "hi"), make_edge("e2", "hi", "c")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    // PRD-021-C: `Paused`, no `Interrupted`. Hasta 0.7.0 el run se marcaba
    // "interrumpido" y ahí moría; ahora está PAUSADO esperando al humano, que
    // es un estado reanudable (`POST /sessions/{id}/resume`).
    assert_eq!(result.status, ExecutionStatus::Paused);
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
        strict_completion: false,
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
        strict_completion: false,
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
            make_conditional_edge("e_yes", "a", "b", "_node_id", ComparisonOp::Eq, json!("a")),
            make_conditional_edge("e_no", "a", "c", "_node_id", ComparisonOp::Neq, json!("a")),
        ],
        metadata: HashMap::new(),
        strict_completion: false,
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

// -----------------------------------------------------------------------
// Fan-out / Fan-in tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn fanout_executes_parallel_nodes() {
    // Graph: A → [B, C] → D (fan-out from A, fan-in at D)
    let graph = GraphDef {
        id: "g-fo".into(),
        name: "fanout".into(),
        version: "1.0.0".into(),
        nodes: vec![
            make_node("a", "tool/echo"),
            make_node("b", "tool/echo"),
            make_node("c", "tool/echo"),
            make_node("d", "tool/echo"),
        ],
        edges: vec![
            // A fans out to B and C (two unconditional edges)
            make_edge("e1", "a", "b"),
            make_edge("e2", "a", "c"),
            // B and C converge at D (fan-in)
            make_edge("e3", "b", "d"),
            make_edge("e4", "c", "d"),
        ],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);

    // A executes first, then B and C in parallel (via fanout), then D
    let traced_ids: Vec<&str> = result.trace.iter().map(|t| t.node_id.as_str()).collect();
    assert_eq!(traced_ids[0], "a");
    // B and C should both appear (order may vary)
    assert!(traced_ids.contains(&"b"), "B should be in trace");
    assert!(traced_ids.contains(&"c"), "C should be in trace");
    // D is the last one (join node)
    assert_eq!(traced_ids.last().unwrap(), &"d");

    // All should succeed
    assert!(result.trace.iter().all(|t| t.status == TraceStatus::Ok));

    // Fan-out transcript entries should exist
    let types: Vec<&str> = result
        .transcript
        .iter()
        .map(|t| t.entry_type.as_str())
        .collect();
    assert!(types.contains(&"fanout_start"), "missing fanout_start");
    assert!(types.contains(&"fanout_end"), "missing fanout_end");
}

#[tokio::test]
async fn fanout_single_unconditional_edge_is_sequential() {
    // Graph: A → B → C (single edges, no fan-out)
    let graph = GraphDef {
        id: "g-seq".into(),
        name: "sequential".into(),
        version: "1.0.0".into(),
        nodes: vec![
            make_node("a", "tool/echo"),
            make_node("b", "tool/echo"),
            make_node("c", "tool/echo"),
        ],
        edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "c")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(result.trace.len(), 3);
    // Should NOT contain fanout entries
    let types: Vec<&str> = result
        .transcript
        .iter()
        .map(|t| t.entry_type.as_str())
        .collect();
    assert!(
        !types.contains(&"fanout_start"),
        "sequential graph should not fan-out"
    );
}

#[tokio::test]
async fn fanout_without_join_terminates_after_parallel() {
    // Graph: A → [B, C] (no join node)
    let graph = GraphDef {
        id: "g-nj".into(),
        name: "no-join".into(),
        version: "1.0.0".into(),
        nodes: vec![
            make_node("a", "tool/echo"),
            make_node("b", "tool/echo"),
            make_node("c", "tool/echo"),
        ],
        edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "a", "c")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);

    // A + B + C should all execute
    let traced_ids: Vec<&str> = result.trace.iter().map(|t| t.node_id.as_str()).collect();
    assert!(traced_ids.contains(&"a"));
    assert!(traced_ids.contains(&"b"));
    assert!(traced_ids.contains(&"c"));
}

#[tokio::test]
async fn resolve_all_next_nodes_returns_multiple_unconditional() {
    let graph = GraphDef {
        id: "g".into(),
        name: "t".into(),
        version: "1.0.0".into(),
        nodes: vec![
            make_node("a", "tool/echo"),
            make_node("b", "tool/echo"),
            make_node("c", "tool/echo"),
        ],
        edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "a", "c")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let output = HashMap::new();
    let next = runner.resolve_all_next_nodes("a", &output, &graph);
    assert_eq!(next.len(), 2);
    assert!(next.contains(&"b".to_string()));
    assert!(next.contains(&"c".to_string()));
}

// ===================================================================
// PRD-021-B/C — cancelación cooperativa y reanudar sin repetir efectos
// ===================================================================

/// Ejecutor que registra **cuántas veces corrió cada nodo**. Es lo único que
/// demuestra que un nodo no se re-ejecutó: el estado final es idéntico tanto
/// si corrió una vez como si corrió dos.
#[derive(Default)]
struct Bitacora {
    corridas: std::sync::Mutex<Vec<String>>,
    /// Levanta esta bandera apenas TERMINA el nodo indicado — simula el
    /// `POST /cancel` llegando con el run a mitad de camino.
    cancelar_tras: Option<(String, Arc<AtomicBool>)>,
}

impl Bitacora {
    fn corridas(&self) -> Vec<String> {
        self.corridas.lock().unwrap().clone()
    }
}

/// Envoltorio para poder pasar la MISMA `Bitacora` al runner (que la consume
/// en un `Box`) y seguir leyéndola desde el test.
struct Contador(Arc<Bitacora>);

#[async_trait]
impl ToolExecutor for Contador {
    async fn execute(
        &self,
        node: &NodeDef,
        _inputs: HashMap<String, Value>,
        _ctx: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        self.0.corridas.lock().unwrap().push(node.id.clone());
        if let Some((disparador, flag)) = &self.0.cancelar_tras {
            if *disparador == node.id {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let mut out = HashMap::new();
        out.insert("_node_id".to_string(), Value::String(node.id.clone()));
        Ok(out)
    }
}

fn grafo_lineal(ids: &[&str]) -> GraphDef {
    GraphDef {
        id: "g".into(),
        name: "lineal".into(),
        version: "1.0.0".into(),
        nodes: ids.iter().map(|id| make_node(id, "tool/echo")).collect(),
        edges: ids
            .windows(2)
            .enumerate()
            .map(|(i, par)| make_edge(&format!("e{i}"), par[0], par[1]))
            .collect(),
        metadata: HashMap::new(),
        strict_completion: false,
    }
}

#[tokio::test]
async fn cancelar_a_mitad_de_run_para_en_la_frontera_del_siguiente_nodo() {
    // Cooperativo: el nodo en curso TERMINA (no se lo mata) y el bucle se
    // detiene antes de arrancar el siguiente, dejando dicho dónde quedó.
    let flag = Arc::new(AtomicBool::new(false));
    let bitacora = Arc::new(Bitacora {
        corridas: Default::default(),
        cancelar_tras: Some(("b".to_string(), flag.clone())),
    });

    let graph = grafo_lineal(&["a", "b", "c", "d"]);
    let runner =
        GraphRunner::new(Box::new(Contador(bitacora.clone()))).with_cancel_flag(flag.clone());
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Cancelled);
    assert_eq!(
        result.interrupt_node_id,
        Some("c".to_string()),
        "queda registrado el nodo en el que iba"
    );
    assert_eq!(
        bitacora.corridas(),
        vec!["a", "b"],
        "'b' termina (no se lo mata) y 'c'/'d' nunca arrancan"
    );
}

#[tokio::test]
async fn request_cancel_antes_de_arrancar_no_ejecuta_nada() {
    let graph = grafo_lineal(&["a", "b"]);
    let runner = GraphRunner::new(Box::new(FailExecutor));
    runner.request_cancel();
    let ctx = TestContext::new();
    let result = runner.run(&graph, &ctx).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Cancelled);
    assert_eq!(result.interrupt_node_id, Some("a".to_string()));
    assert!(result.trace.is_empty(), "no corrió un solo nodo");
}

#[tokio::test]
async fn la_bandera_de_cancelacion_es_por_run_no_global() {
    // `GraphRunner` es Clone y comparte sus campos por `Arc`: si la bandera se
    // heredara de la plantilla, cancelar UN run cancelaría todos los del
    // proceso. Por eso el server ata una bandera nueva por run.
    let plantilla = GraphRunner::new(Box::new(EchoExecutor));
    let run_a = plantilla
        .clone()
        .with_cancel_flag(Arc::new(AtomicBool::new(false)));
    let run_b = plantilla
        .clone()
        .with_cancel_flag(Arc::new(AtomicBool::new(false)));
    run_a.request_cancel();

    let graph = grafo_lineal(&["a", "b"]);
    let ctx = TestContext::new();
    assert_eq!(
        run_b.run(&graph, &ctx).await.unwrap().status,
        ExecutionStatus::Completed,
        "cancelar un run no puede tumbar al de al lado"
    );
    assert_eq!(
        run_a.run(&graph, &ctx).await.unwrap().status,
        ExecutionStatus::Cancelled
    );
}

#[tokio::test]
async fn resume_skipping_no_vuelve_a_ejecutar_los_nodos_ya_corridos() {
    // El riesgo del PRD: si 'a' mandó un correo, reanudar no puede mandarlo
    // otra vez. Se cuenta CUÁNTAS VECES corrió cada nodo.
    let bitacora = Arc::new(Bitacora::default());
    let graph = grafo_lineal(&["a", "b", "c"]);
    let runner = GraphRunner::new(Box::new(Contador(bitacora.clone())));
    let ctx = TestContext::new();

    // Estado tal como quedó tras correr 'a' y 'b'.
    let estado = SharedState::new();
    for id in ["a", "b"] {
        let mut out = HashMap::new();
        out.insert("_node_id".to_string(), Value::String(id.to_string()));
        estado.set(id, out, true).unwrap();
    }

    // Se apunta a 'a' A PROPÓSITO — el peor caso: un cursor mal calculado o un
    // grafo que vuelve atrás. El guard tiene que saltarlo igual.
    let result = runner
        .resume_skipping(
            &graph,
            &ctx,
            estado,
            "a",
            &["a".to_string(), "b".to_string()],
            2,
        )
        .await
        .unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        bitacora.corridas(),
        vec!["c"],
        "solo el nodo pendiente corre: 'a' y 'b' no repiten su efecto"
    );
}

#[tokio::test]
async fn el_salto_es_de_una_sola_vez_para_no_romper_los_bucles() {
    // Saltar SIEMPRE rompería un grafo con ciclo legítimo: la segunda vuelta
    // por el mismo nodo sí tiene que ejecutarse.
    let bitacora = Arc::new(Bitacora::default());
    // a → b, y 'b' vuelve a 'a' (ciclo).
    let graph = GraphDef {
        id: "g".into(),
        name: "ciclo".into(),
        version: "1.0.0".into(),
        nodes: vec![make_node("a", "tool/echo"), make_node("b", "tool/echo")],
        edges: vec![make_edge("e1", "a", "b"), make_edge("e2", "b", "a")],
        metadata: HashMap::new(),
        strict_completion: false,
    };

    let runner = GraphRunner::new(Box::new(Contador(bitacora.clone()))).with_max_iterations(2);
    let ctx = TestContext::new();
    let estado = SharedState::new();
    let mut out = HashMap::new();
    out.insert("_node_id".to_string(), Value::String("a".to_string()));
    estado.set("a", out, true).unwrap();

    let _ = runner
        .resume_skipping(&graph, &ctx, estado, "a", &["a".to_string()], 1)
        .await;

    let corridas = bitacora.corridas();
    assert_eq!(
        corridas.first().map(String::as_str),
        Some("b"),
        "la primera pasada saltó 'a' (ya había corrido): {corridas:?}"
    );
    assert!(
        corridas.iter().any(|n| n == "a"),
        "en la vuelta del ciclo 'a' SÍ ejecuta — el salto valió una sola vez: {corridas:?}"
    );
}

// ===========================================================================
// strict_completion — layer 2 (PRD-022)
// ===========================================================================

/// Emits an output whose only field is null: the shape of a node that ran and
/// produced nothing — `logic/merge` with no data, the `db_write` sentinel.
struct EmptyOutputExecutor;

#[async_trait]
impl ToolExecutor for EmptyOutputExecutor {
    async fn execute(
        &self,
        _node: &NodeDef,
        _inputs: HashMap<String, Value>,
        _ctx: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let mut out = HashMap::new();
        out.insert("data".to_string(), Value::Null);
        Ok(out)
    }
}

fn strict_linear_graph(nodes: Vec<NodeDef>, edges: Vec<EdgeDef>) -> GraphDef {
    GraphDef {
        id: "strict".into(),
        name: "strict".into(),
        version: "1.0.0".into(),
        nodes,
        edges,
        metadata: HashMap::new(),
        strict_completion: true,
    }
}

fn node_with_failure_mode(id: &str, on_failure: FailureMode) -> NodeDef {
    let mut node = make_node(id, "tool/fail");
    node.config.insert(
        "retry_policy".to_string(),
        serde_json::to_value(RetryPolicy {
            max_retries: 0,
            backoff: BackoffStrategy::None,
            initial_delay_secs: 0.0,
            on_failure,
        })
        .unwrap(),
    );
    node
}

/// R1 — the walk ended on a node that produced no value.
#[tokio::test]
async fn strict_r1_fails_when_the_run_ends_without_a_value() {
    let graph = strict_linear_graph(
        vec![make_node("a", "logic/merge"), make_node("b", "logic/merge")],
        vec![make_edge("e1", "a", "b")],
    );

    let runner = GraphRunner::new(Box::new(EmptyOutputExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    let error = result.error.expect("strict violation must carry a message");
    assert!(error.starts_with(wk::STRICT_PREFIX), "got: {error}");
    assert!(error.contains("'b'"), "must name the node: {error}");
    assert!(error.contains("no value"), "got: {error}");
}

/// TEST-213 / zero false positives — the same shape with a real value passes.
#[tokio::test]
async fn strict_lets_a_healthy_run_complete() {
    let graph = strict_linear_graph(
        vec![
            make_node("a", "logic/merge"),
            make_node("b", "output/response"),
        ],
        vec![make_edge("e1", "a", "b")],
    );

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert!(result.error.is_none());
}

/// Same graph, flag off: the value-less ending stays `Completed`, byte for
/// byte what the engine did before PRD-022.
#[tokio::test]
async fn strict_off_keeps_the_silent_completion() {
    let mut graph = strict_linear_graph(
        vec![make_node("a", "logic/merge"), make_node("b", "logic/merge")],
        vec![make_edge("e1", "a", "b")],
    );
    graph.strict_completion = false;

    let runner = GraphRunner::new(Box::new(EmptyOutputExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert!(result.error.is_none());
}

/// R2 — `on_failure: skip` swallowed the error; the run must carry the cause.
#[tokio::test]
async fn strict_r2_reports_the_error_a_skip_swallowed() {
    let graph = strict_linear_graph(
        vec![node_with_failure_mode("save", FailureMode::Skip)],
        vec![],
    );

    let runner = GraphRunner::new(Box::new(FailExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    let error = result.error.expect("strict violation must carry a message");
    assert!(error.starts_with(wk::STRICT_PREFIX), "got: {error}");
    assert!(error.contains("'save'"), "must name the node: {error}");
    assert!(error.contains("skipped"), "got: {error}");
    assert!(
        error.contains("always fails"),
        "must carry the original cause: {error}"
    );
}

/// R2 — `route_to_error` with no error edge: the error stayed in the state.
#[tokio::test]
async fn strict_r2_reports_route_to_error_that_found_no_edge() {
    let graph = strict_linear_graph(
        vec![node_with_failure_mode("save", FailureMode::RouteToError)],
        vec![],
    );

    let runner = GraphRunner::new(Box::new(FailExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    let error = result.error.expect("strict violation must carry a message");
    assert!(error.starts_with(wk::STRICT_PREFIX), "got: {error}");
    assert!(
        error.contains("routed to error"),
        "must distinguish it from a skip: {error}"
    );
    assert!(
        error.contains("always fails"),
        "must carry the original cause: {error}"
    );
}

/// R3 — defense in depth: a runtime dead end that layer 1 never saw, because
/// the graph was handed straight to the runner with the flag set after load.
#[tokio::test]
async fn strict_r3_fails_on_a_runtime_dead_end() {
    let mut graph = strict_linear_graph(
        vec![
            make_node("route", "logic/switch"),
            make_node("yes", "output/response"),
        ],
        vec![make_conditional_edge(
            "e1",
            "route",
            "yes",
            "missing_field",
            ComparisonOp::Eq,
            json!(true),
        )],
    );
    // Layer 1 would reject this shape (S1); skip it to exercise layer 2 alone.
    graph.strict_completion = false;
    graph.validate().unwrap();
    graph.strict_completion = true;

    let runner = GraphRunner::new(Box::new(EchoExecutor));
    let result = runner
        .run_from(
            &graph,
            &TestContext::new(),
            SharedState::new(),
            "route",
            0,
            std::collections::HashSet::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    let error = result.error.expect("strict violation must carry a message");
    assert!(error.starts_with(wk::STRICT_PREFIX), "got: {error}");
    assert!(error.contains("'route'"), "must name the node: {error}");
    assert!(error.contains("outgoing edges matched"), "got: {error}");
}

/// TEST-214 — a paused run never reaches the completion check: strict only
/// ever turns a `Completed` into a `Failed`.
#[tokio::test]
async fn strict_leaves_a_paused_run_alone() {
    let graph = strict_linear_graph(
        vec![
            make_node("ask", wk::HUMAN_INPUT_TOOL),
            make_node("done", "output/response"),
        ],
        vec![make_edge("e1", "ask", "done")],
    );

    let runner = GraphRunner::new(Box::new(EmptyOutputExecutor));
    let result = runner.run(&graph, &TestContext::new()).await.unwrap();

    assert_eq!(result.status, ExecutionStatus::Paused);
    assert!(result.error.is_none());
}
