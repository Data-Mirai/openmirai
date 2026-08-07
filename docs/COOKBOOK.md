# OpenMirai Cookbook

This cookbook is a practical route through the engine. Read
[SYSTEM_LIFECYCLE.md](SYSTEM_LIFECYCLE.md) first when you need the full internal
model, and [AGENT_SPEC.md](AGENT_SPEC.md) while authoring a spec.

## 1. Verify the binary and a spec

```bash
mirai version
mirai validate examples/hello-world.yaml
mirai describe examples/hello-world.yaml
```

The CLI loader expects YAML. Validation checks the implemented structural rules;
it does not prove general graph acyclicity or that every declared host resource
will be available at runtime.

## 2. Execute once

```bash
mirai run examples/hello-world.yaml --input '{"question":"What is Rust?"}'
```

At runtime, traversal and data flow are separate. An edge selects a downstream
node, while `data_map` copies values:

```yaml
edges:
  - source: classify
    target: answer
    data_map:
      prompt: classify.result
```

If `answer` requires `prompt`, omitting this mapping does not automatically copy
`classify.result` into it.

## 3. Add bounded retry behavior

Retry is a node policy:

```yaml
nodes:
  - id: answer
    tool_type: ai/llm_call
    config:
      prompt: "Answer concisely"
      retry_policy:
        max_retries: 3
        initial_delay_secs: 0.25
        backoff: exponential
        on_failure: stop
```

Agent-level retry and timeout fields are declared in AgentSpec but are not
uniformly applied by the default CLI/server path. Prefer the documented
node-level policy for current workflows.

## 4. Start the HTTP host safely

```bash
export MIRAI_API_KEY='replace-with-a-secret'
mirai serve --host 127.0.0.1 --port 8080
curl http://127.0.0.1:8080/health
```

Bind to a non-loopback address only behind TLS and access controls. The exact
route and payload inventory is in [backend/API.md](backend/API.md), with a
machine-readable description in
[backend/openapi.yaml](backend/openapi.yaml).

For executable agents, create them from an AgentSpec. The generic `POST /agents`
route creates a record with an empty graph and is not a substitute for uploading
an executable spec.

## 5. Consume execution events

Use the streaming execution route when a client needs progress events. Treat
SSE event names and payloads as a separate contract from the final synchronous
response.

Clients must implement reconnect/error behavior and their own request timeout.
Disconnecting the client does not currently guarantee cancellation of the
underlying execution. See [backend/STREAMING.md](backend/STREAMING.md).

## 6. Add MCP tools

MCP servers belong under `config.mcp_servers` in AgentSpec. Choose `stdio` for a
locally spawned server or HTTP for a separately operated endpoint. Test startup,
tool discovery, argument/result conversion, timeout, and server shutdown.

`credential_ref` is currently descriptive in the standard manager and is not
automatically converted into HTTP authorization headers. Place remote MCP
behind a trusted authenticated boundary until host credential wiring is added.
See [backend/MCP.md](backend/MCP.md).

## 7. Decide whether a live agent is appropriate

The repository contains schedule, trigger, and live-agent components, but the
standard server constructs its scheduler without starting its run loop. Do not
base a production recurring workload on startup alone. Use an external scheduler
to call a tested execution entry point, or wire and test the live scheduler in a
custom host. See [backend/LIVE_AGENTS.md](backend/LIVE_AGENTS.md).

## 8. Preserve and inspect state

For a stateful deployment:

- mount the SQLite database and required file-storage paths on persistent
  volumes;
- back up the database with a SQLite-safe method;
- preserve encryption and provider credentials outside the image;
- record the binary version and AgentSpec revision for each rollout;
- verify trace and metrics export before sending production traffic.

Use [infra/OPERATIONS.md](infra/OPERATIONS.md) for backup, restore, incidents,
upgrade, and shutdown, and [backend/OBSERVABILITY.md](backend/OBSERVABILITY.md)
for telemetry.

## 9. Prepare for a cloud container

The current safe baseline is one replica, persistent state, ingress TLS,
authentication, explicit tool allowlisting, egress controls, and graceful
termination. A multi-replica stateless deployment is not yet supported by the
standard host.

The exact blockers and target container model are documented in
[infra/CLOUD-DEPLOYMENT.md](infra/CLOUD-DEPLOYMENT.md). Do not infer production
readiness merely from a successful local image build.
