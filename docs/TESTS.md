<!--
BLUEPRINT SEED — TESTS.md
Responsable: → blueprint/agents/05-JOURNEYS.md (via 00-WORKBOARD)

Estructura esperada:
- Agrupación por PRD, luego por CA (criterio de aceptación)
- Cada test con formato GWT: test_id, CA, Given, When, Then

Reglas:
- test_id global y único, nunca reutilizar
- Un test = un CA
- GWT siempre completo (Given, When, Then)
- Describe comportamiento, no implementación
- El agente 05-JOURNEYS agrega nuevos tests durante el diseño
-->

# Test Strategy

OpenMirai has a comprehensive suite across unit, integration, and system-level
tests. The exact count changes as the engine evolves; use `cargo test
--workspace --all-features -- --list` when an exact count is required. No
external services are required—the suite uses bundled SQLite, in-memory
adapters, mocks, and temporary directories.

## Running Tests

```bash
# All tests, all crates
cargo test --workspace --all-features

# Just the engine
cargo test -p openmirai-engine --lib --all-features

# Specific test module
cargo test --lib core::runner --all-features

# Integration tests (manual YAML-based scenarios)
./test/run_all.sh
```

## Test Architecture

The named tests below are a representative inventory. Use the command above
when the exact current suite size is needed.

### 1. Unit Tests (`#[test]`)

Synchronous tests for logic, parsing, schema validation, and pure functions. No async runtime required.

- **Graph validation** (`core/graph.rs`): node/edge validation, duplicate detection, self-loops, entry points.
- **Expression resolution** (`core/runner/tests.rs`): node reference resolution, template interpolation, missing fields.
- **Condition evaluation** (`core/runner/tests.rs`): `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte`, `In`, `Contains` operators.
- **Backoff calculation** (`core/runner/tests.rs`): `None`, `Linear`, `Exponential` retry strategies.
- **Agent spec** (`core/agent_spec.rs`): YAML parsing, memory modes, schedule validation, spec inheritance.
- **Cryptography & security** (`security.rs`): prompt injection detection, threat classification, sensitivity levels, scanner configuration.
- **Serialization** (`core/runner/tests.rs`): Serde round trips for `TraceEntry`, `ExecutionResult`, `RetryPolicy`.

### 2. Async Integration Tests (`#[tokio::test]`)

Async tests using Tokio runtime. Most tests run a full graph execution with stub executors and in-memory state.

#### Graph Execution (`core/runner/tests.rs`)

**Sequential Execution**

- `linear_graph_executes_all_nodes`: Linear chain A→B→C, all complete, trace ordered.
- `single_node_graph`: Minimal case, one node, immediate completion.
- `fanout_single_unconditional_edge_is_sequential`: A→B→C treated as sequential, no fanout transcript.

**Fan-out / Fan-in**

- `fanout_executes_parallel_nodes`: A→{B,C}→D, both B and C execute, D joins.
- `fanout_without_join_terminates_after_parallel`: A→{B,C} with no join node, all execute and terminate.
- `resolve_all_next_nodes_returns_multiple_unconditional`: Helper to detect fanout edges.

**Conditional Routing**

- `conditional_edge_routes_correctly`: Two conditional edges from start, only matching branch executes.
- `unconditional_fallback_when_no_condition_matches`: If conditional misses, fallback edge fires.

**Failure Modes**

- `failure_mode_stop_returns_failed`: Node fails, `FailureMode::Stop` halts, `ExecutionStatus::Failed`.
- `failure_mode_skip_continues`: Node fails, `FailureMode::Skip` skips node, execution continues to next.
- `failure_mode_route_to_error`: Node fails, `FailureMode::RouteToError` sets `__error__` field, conditional routing to error handler.

**Retry & Resilience**

- `retry_succeeds_on_second_attempt`: FailNExecutor fails once, retries, succeeds, trace shows 1 retry.
- `max_iterations_exceeded`: Graph with A→B→C→B cycle hits max iterations, returns `RunnerError::MaxIterationsExceeded`.

**State Management**

- `data_map_passes_values_between_nodes`: Edge data_map from A to B injects prior output, B receives injected value.
- `resume_continues_from_state`: Resume execution from node B with pre-existing state from A, B and C execute.

**Error Handling**

- `empty_graph_returns_graph_error`: Graph with no nodes/edges, returns `RunnerError::GraphError`.

**Events & Observation**

- `event_emitter_receives_events`: SessionStarted, BlockStarted, BlockCompleted, SessionCompleted events emitted.
- `transcript_contains_all_event_types`: Transcript includes started, block_start, block_end, decision, completed.

#### Hooks & Checkpointing (`core/runner/tests.rs`)

- `hook_on_graph_start_aborts`: Hook returns `Abort`, execution halts before any node.
- `hook_pre_block_exec_skips`: Hook returns `Skip`, blocks bypass executor, no trace.
- `hook_pre_block_exec_modifies_inputs`: Hook returns `ModifiedInputs`, executor receives modified values.
- `hook_on_error_triggers_retry`: Hook returns `Retry`, node retries on error.
- `hook_timeout_returns_continue`: Hook timeout (30s) exceeds, defaults to `Continue`.
- `checkpoint_called_after_each_block`: Checkpoint saved after each successful node, count = node count.

#### Human Input & Interrupts (`core/runner/tests.rs`)

- `human_input_interrupts_execution`: `logic/human_input` node sets `ExecutionStatus::Interrupted` with `InterruptInfo`.
- `pause_request_interrupts`: `request_pause()` before run, execution pauses before first node.

#### API Tests (`server/tests.rs`)

- `health_returns_ok`: GET `/health` → 200 OK with `{"status": "ok"}`.
- `version_returns_engine_info`: GET `/version` → 200 OK with engine version.
- `graph_crud_lifecycle`: Create, list, get, delete graph, final get → 404.
- `agent_crud_lifecycle`: Create graph, create agent from graph, list, get agent.
- `execute_agent_from_spec`: POST `/api/v1/agents/from-spec`, execute via trigger/manual, verify status.
- `list_tools_returns_all_builtins`: GET `/api/v1/tools` → all 52 builtin tools.
- `create_agent_with_invalid_graph_returns_404`: Reference nonexistent graph → 404.
- `get_nonexistent_session_returns_404`: GET `/api/v1/sessions/ghost` → 404.
- `webhook_returns_received`: POST `/webhooks/my-hook` → 200 OK with `{"received": true}`.

### 3. Tool & Resource Tests

#### LLM Adapters

- **Claude** (`llm/claude.rs`): authentication, streaming, structured output parsing.
- **OpenAI compatible** (`llm/openai_compat.rs`): requests, streaming format, error handling.
- **Ollama** (`llm/ollama.rs` + `adapters/ollama_llm.rs`): local inference, model pull, fallback to default, think tag cleaning.
- **Gemini** (`llm/gemini.rs`): API key validation, streaming, content blocks.
- **OpenRouter** (`llm/openrouter.rs`): model selection and fallback.
- **NVIDIA NIM** (`llm/nvidia.rs`): endpoint availability.
- **Groq** (`llm/groq.rs`): fast completion.
- **Media handling** (`llm/media.rs`): image encoding, MIME types, base64 round trips, file references.
- **Mock LLM** (`adapters/mock_llm.rs`): response cycling, deterministic embeddings, token counting.

#### Built-in Tools

- **AI tools** (`tools/builtin/ai.rs`): LLM calls, structured output schema validation, retry logic, token counting.
- **Data tools** (`tools/builtin/data/`): JSON transformation, SQL queries, CSV parsing, vault operations.
- **Filesystem** (`tools/builtin/filesystem/`): file I/O, path traversal prevention, directory creation.
- **Logic** (`tools/builtin/logic.rs`): conditions, branching, boolean logic.
- **System** (`tools/builtin/system.rs`): process execution, environment variables, timeouts.
- **Agent** (`tools/builtin/agent.rs`): sub-agent invocation, circular reference detection, nesting depth.
- **Git** (`tools/builtin/git.rs`): clone, commit, push operations.
- **Trigger** (`tools/builtin/trigger.rs`): payload injection, HTTP triggers, webhook parsing.
- **Output** (`tools/builtin/output.rs`): response formatting and final payload assembly.
- **State** (`tools/builtin/state.rs`): memory read/write and persistence modes.
- **MCP** (`tools/builtin/mcp/`): tool discovery and resource invocation.

#### Memory & Persistence

- **SQLite backend** (`memory/sqlite_backend.rs`): schema creation, CRUD, transactions.
- **In-memory backend** (`memory/in_memory_backend.rs`): fallback when SQLite is unavailable.
- **Short-term memory** (`memory/short_term.rs`): session-scoped recall.
- **Long-term memory** (`memory/long_term.rs`): cross-session persistence and embeddings.

#### Storage Adapters

- **Local filesystem** (`adapters/local_storage.rs`): put/get/delete, path traversal prevention, subdirectory creation.
- **In-memory storage** (`adapters/in_memory_storage.rs`): key-value operations, prefix listing, deletion.
- **SQLite DB** (`adapters/sqlite_db.rs`): execute, fetch, transaction isolation.
- **In-memory DB** (`adapters/in_memory_db.rs`): table queries, upsert, row deletion.

### 4. System & Cross-Cutting Tests

- **Auth & RBAC** (`core/auth.rs`): role validation, universe scoping, permission checks.
- **Schema validation** (`core/schema.rs`): field type matching, required fields, nested objects.
- **State management** (`core/state.rs`): node state isolation, shared state, field injection.
- **Value types** (`core/value_type.rs`): type coercion and JSON serialization.
- **Event system** (`core/events.rs`): broadcast channels and event filtering.
- **Observability** (`observability.rs`): logging, tracing, metrics.
- **Benchmarking** (`benchmark.rs`): execution timing, metric collection, JSONL output.
- **Template rendering** (`templates.rs`): variable substitution and Markdown formatting.
- **Search & RAG** (`search/`, `rag/`, `mcp/`): vector search, semantic retrieval, embedding management, hybrid search, MCP client/manager.
- **Sandboxing** (`sandbox.rs`): resource limits, unsafe code isolation, Bash/Python execution.

### 5. MockLLMResource (Unit Tests Only)

The `MockLLMResource` is **never used in the HTTP server, CLI, or runtime**. It exists solely for unit test convenience:

```rust
// In unit tests only
#[test]
fn my_test() {
    let mock_llm = MockLLMResource::new()
        .with_response("expected output");
    // ... run test
}
```

**Server tests use a test factory**:

```rust
fn test_llm_factory() -> LLMFactory {
    Arc::new(|| Box::new(MockLLMResource::new()))  // Server starts but doesn't call LLM
}
```

**Production uses real LLM adapters** (Claude, OpenAI, Ollama, etc.) configured via environment variables.

---

## Representative Test Scenarios (Given-When-Then)

### Scenario 1: Linear Pipeline Execution

**TEST-001: Linear graph executes all nodes in sequence**

- **CA**: Graph execution completes successfully when nodes form a linear chain.
- **Given**: A graph with nodes A→B→C connected by unconditional edges.
- **When**: `runner.run(&graph, &context)` is called.
- **Then**: All three nodes execute in order, `trace` contains 3 entries with status `Ok`, final `ExecutionStatus::Completed`.

### Scenario 2: Conditional Routing

**TEST-002: Conditional edge selects correct branch based on field value**

- **CA**: Graph routing respects `ComparisonOp` on output fields.
- **Given**: Start node outputs `_node_id = "start"`, two edges with conditions `Eq("start")` and `Neq("start")`.
- **When**: Runner evaluates edges after start executes.
- **Then**: Only the matching condition's target executes; trace length is 2, second entry is the `Eq` target.

### Scenario 3: Failure with Retry

**TEST-003: Flaky node succeeds on second attempt when retry policy allows it**

- **CA**: Node failure triggers retry up to `max_retries` limit.
- **Given**: FailNExecutor(1) that fails once then succeeds, node config sets `max_retries: 2`, `backoff: Linear, initial_delay_secs: 0.1`.
- **When**: Runner executes the node and encounters first failure.
- **Then**: Runner retries, second attempt succeeds, `trace[0].retries == 1`, `trace[0].status == Ok`.

### Scenario 4: Error Handler Routing

**TEST-004: Failed node routes to error handler via `__error__` field**

- **CA**: `FailureMode::RouteToError` injects error state, conditional edge detects it.
- **Given**: Node A with `FailureMode::RouteToError` that always fails, conditional edge checking `__error__ != null` to error_handler.
- **When**: Node A executes and fails.
- **Then**: `__error__` field populated, conditional evaluates true, error_handler is next target, `trace[1].node_id == "error_handler"`.

### Scenario 5: Fan-out and Join

**TEST-005: Multiple parallel branches execute, then rejoin at convergence node**

- **CA**: Fan-out edges from one node execute in parallel; fan-in waits for all.
- **Given**: Graph with A→{B, C}→D (two unconditional edges from A to B and C, two from B and C to D).
- **When**: Runner executes the graph.
- **Then**: A executes first, transcript shows `fanout_start`, both B and C appear in trace, D executes last, `trace` is [A, B, C, D] or [A, C, B, D].

### Scenario 6: Data Mapping Between Nodes

**TEST-006: Output field from one node is renamed and passed to next via data_map**

- **CA**: `data_map` on edges transforms output keys.
- **Given**: Node A outputs `_node_id = "a"`, edge A→B with `data_map: {"prev_id": "a._node_id"}`.
- **When**: Runner processes data_map before executing B.
- **Then**: Node B's inputs include `prev_id = "a"`, B's EchoExecutor echoes it back, `result.state.get_field("b", "prev_id") == json!("a")`.

### Scenario 7: Structured Output Validation

**TEST-007: LLM call with output_schema validates response and retries on mismatch**

- **CA**: AI tool enforces JSON schema, retries if response doesn't match.
- **Given**: LLM tool with `output_schema: '{"type": "object", "properties": {"answer": {"type": "string"}}}'`, `max_retries: 2`, LLM returns invalid JSON.
- **When**: Tool executes and gets non-conformant response.
- **Then**: Tool retries up to 2 times, if all fail and `output_schema_strict: true`, execution fails with schema validation error.

### Scenario 8: Sub-agent Invocation

**TEST-008: Parent graph calls child agent, inherits state, child state flows back**

- **CA**: `agent/run_agent` tool invokes sub-agent, enforces nesting depth, detects circular references.
- **Given**: Agent A executes agent/run_agent with `agent_id: "agent-b"`, `__agent_call_chain__: ["agent-a"]`.
- **When**: Tool checks for circular references.
- **Then**: Tool returns error containing "Circular agent reference" if "agent-a" is also in the chain.

### Scenario 9: Hook Abort Before Execution

**TEST-009: on_graph_start hook returning Abort halts execution before any node runs**

- **CA**: Hooks can abort before graph begins.
- **Given**: Graph with node A, hook returning `Abort("not authorized")`.
- **When**: Runner calls `on_graph_start` hook.
- **Then**: Hook returns abort, execution stops, `ExecutionStatus::Failed`, `trace.len() == 0`, error message contains "not authorized".

### Scenario 10: Checkpoint Persistence

**TEST-010: After each successful node, checkpoint is saved with step and state**

- **CA**: Checkpoint callback fires after node completion, enabling resumption.
- **Given**: Linear graph A→B→C, checkpoint callback registered.
- **When**: All nodes execute successfully.
- **Then**: Checkpoint callback called 3 times (once per node), saved checkpoints contain node_id and step counter, can resume from any checkpoint.

---

## Test Execution Summary

| Category | Count | Type | Runs | Isolation |
|----------|-------|------|------|-----------|
| Unit (graph, expr, backoff) | ~80 | Sync | All PRs | No async, no I/O |
| Async (runner, API) | ~65 | Async | All PRs | Tokio runtime, in-memory |
| Tools & LLM adapters | ~200 | Mixed | All PRs | In-memory or mock |
| Memory & persistence | ~45 | Sync+Async | All PRs | SQLite bundled, no external |
| System & cross-cutting | ~300 | Sync+Async | All PRs | SQLite bundled, no external |
| **Total** | **Current workspace suite** | — | — | **No external services** |

## CI Pipeline

`.github/workflows/ci.yml` runs on every push/PR:

1. **rustfmt**: Code formatting check
2. **clippy**: Linter (treat warnings as errors)
3. **test**: Full `cargo test --workspace --all-features` on Ubuntu & macOS

The complete workspace suite must pass before merge.

## Notes

- **No external services**: All tests use bundled SQLite, in-memory adapters, or stubs.
- **MockLLMResource is test-only**: Never shipped to server/CLI/runtime.
- **Deterministic**: No flaky tests, no timing assumptions, tokio::time::pause for timeout tests.
- **Coverage**: Graph logic, tool execution, LLM integration, error handling, state management, API surface, hooks, interrupts.
- **52 builtin tools**: Registered tools include 7 logic, 6 AI, 11 data, 12 filesystem, 3 system, 4 git, 1 output, 1 agent, 1 MCP, 5 trigger, and 1 state tool.
