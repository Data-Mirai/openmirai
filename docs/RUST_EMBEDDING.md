# Embedding `openmirai-engine` in Rust

Use the engine crate when an application needs to own resource wiring, tool
registration, hooks, checkpoints, authorization context, or persistence rather
than invoking the `mirai` CLI/server.

## Minimal graph execution

```rust
use std::sync::Arc;

use openmirai_engine::{
    AgentSpec, DefaultExecutionContext, GraphRunner, RegistryExecutor,
    ToolRegistry,
};
use openmirai_engine::tools::builtin::register_all_builtin_tools;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = AgentSpec::from_file("agent.yaml")?;
    let graph = spec.to_graph(None);

    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let executor = RegistryExecutor::new(Arc::new(registry));
    let runner = GraphRunner::new(Box::new(executor));

    // Mock LLM + in-memory DB/storage: development/testing only.
    let context = DefaultExecutionContext::default_dev();
    let result = runner.run(&graph, &context).await?;

    println!("{:?}", result.status);
    Ok(())
}
```

`AgentSpec::from_file()` already parses and validates. If a host constructs or
mutates a `GraphDef`, call `prepare()`/`validate()` before execution.

## Production context

`DefaultExecutionContext::builder()` requires an `LLMResource` and optionally
accepts DB, storage, vector, auth, session ID, node ID, system prompt, and
scratch directory resources. Tools can only use resources that the context
provides—except documented direct-host tools such as filesystem/system/git.

```text
host adapter / provider client
 -> LLMResource, DBResource, StorageResource, VectorResource
 -> DefaultExecutionContext
 -> Tool
```

An embedding host owns connection pooling, credentials, tenancy, retries,
timeouts, secret handling, transaction boundaries, and resource cleanup.

## Runner configuration

Builder methods can attach:

- maximum visits per node;
- default retry policy;
- `HookHandler`;
- `CheckpointCallback`;
- generic `EventEmitter`;
- SSE-style stream sender.

This is how an embedding application can activate library features that the
default CLI/server leave unwired. Keep callbacks non-blocking or explicitly
offload their I/O, and define behavior when checkpoint/event persistence fails.

## Custom tools

Implement `Tool`, expose a `ToolFactory` with a `ToolSpec`, and register it in a
host-owned `ToolRegistry`. `RegistryExecutor` validates declared inputs/config,
creates a fresh tool, catches panics in the tool future, and converts failures
to `ToolError`.

Use `ExecutionContext` ports for DB/storage/vector/model operations so the host
can enforce scoping. Adding direct filesystem/network/process behavior expands
the host security boundary and requires explicit review.

## Persistence

Runner `SharedState` is per execution. A `CheckpointCallback` receives the
current node/cursor/state and enables a host to persist it; `resume()` requires
the host to restore state and context. The library does not choose a checkpoint
database or expose distributed ownership automatically.

The server's SQLite run repository is a host adapter, not the same DB resource
tools see in a default run.

## Feature flags

The engine defaults include `server` and `builtin-tools`. An embedding build
can disable default features and select only the desired surface, but the host
must test the exact feature combination. The CLI depends on default engine
features.

## Testing pattern

- use `DefaultExecutionContext::default_dev()` or explicit mock resources;
- keep tests independent of external providers/services;
- use temporary directories for filesystem tools;
- validate graph and tool schemas;
- test failure, retry, pause, checkpoint, and cancellation semantics;
- never treat `AgentRuntime::execute_agent()` as the GraphRunner production path—it is currently simulated.

## Host responsibilities checklist

- provider/resource factories and secret injection;
- AgentSpec boundary validation and runtime injection;
- permitted tool registry/profile;
- timeouts, cancellation, idempotency, and concurrency;
- state/checkpoint/result persistence;
- authentication/tenant context;
- logs, metrics, traces, and redaction;
- graceful shutdown and resource cleanup;
- migration/compatibility policy.

## Related documentation

- [Architecture](ARCHITECTURE.md)
- [AgentSpec](AGENT_SPEC.md)
- [Primitives](backend/PRIMITIVES.md)
- [Built-in tools](backend/BUILTIN_TOOLS.md)
- [System lifecycle](SYSTEM_LIFECYCLE.md)
