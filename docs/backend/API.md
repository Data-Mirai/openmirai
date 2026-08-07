# OpenMirai HTTP API v1

This is the human-readable contract for the Axum server in OpenMirai 0.7.0.
The route source is `engine/src/server/mod.rs`; request/response behavior lives
in `engine/src/server/handlers.rs` and `engine/src/server/orchestrator.rs`.

Machine-readable route inventory: [`openapi.yaml`](openapi.yaml).

Base URL: `http://127.0.0.1:3000`. The listener is plain HTTP. Use TLS at an
ingress or reverse proxy.

## Authentication

Set `MIRAI_API_KEY` and send it in `X-API-Key`. The server refuses a
non-loopback bind without a key. `/health` and `/version` are public; other API
routes are protected when a key is configured. Orchestrator EventSource also
accepts `?api_key=` because browser EventSource cannot set headers.

Authentication is one shared secret, not user identity, RBAC, tenancy, quotas,
or rate limiting. See [Security](../../SECURITY.md).

## Conventions

- JSON errors normally use `{"error":"message"}`.
- Lists are JSON arrays unless noted.
- Graphs, agents, hot sessions, memory, and schedules are process-local.
- Final execute/stream results are persisted to SQLite best-effort.
- `404` means a process-local or persisted record was not found.
- No rate limiting or idempotency-key contract exists.

## Public endpoints

### `GET /health`

Returns process liveness, version/build/SHA/timestamp, uptime, and in-memory
agent/session/tool counts. It does not prove SQLite or provider readiness.

### `GET /version`

Returns compiled version, build number, Git SHA, build timestamp, and engine
name.

## Graphs

### `POST /api/v1/graphs`

```json
{
  "name": "demo",
  "nodes": [
    {"id":"start","tool_type":"trigger/manual","config":{}},
    {"id":"out","tool_type":"output/response","config":{"message":"ok"}}
  ],
  "edges": [
    {"source":"start","target":"out"}
  ],
  "metadata": {}
}
```

Nodes deserialize as `NodeDef` (`id`, `tool_type`, optional `version`,
`config`, optional tuple `position`). Edges deserialize as `EdgeDef` (`id`,
`source`, `target`, optional `condition`, optional `data_map`). Invalid shapes
return 400. The handler stores the graph with a generated ID and version
`1.0.0`, but does not call full graph validation at creation time.

### `GET /api/v1/graphs`

Lists process-local graphs.

### `GET /api/v1/graphs/{id}`

Returns one graph or 404.

### `DELETE /api/v1/graphs/{id}`

Removes the process-local graph and returns 204 or 404. It does not cascade a
durable repository because this registry is in memory.

## Agents

### `POST /api/v1/agents`

Accepts `{name, description?, graph_id, triggers?}` and verifies that the graph
ID exists. It creates a lightweight process-local AgentSpec record containing
the graph ID in metadata, but does **not** copy the referenced graph nodes and
edges into `spec.graph`. Use `from-spec` for an executable agent in 0.7.0.

### `POST /api/v1/agents/from-spec`

Accepts the JSON representation of [AgentSpec v1](../AGENT_SPEC.md) and returns
`201 {id, name, status}`. Unlike file loading, HTTP naturally carries JSON.

Important: this handler deserializes and stores the spec but does not call
`AgentSpec::validate()` at registration. Graph validation occurs when a run is
started. Unknown fields are ignored by the current Serde model.

### `GET /api/v1/agents`

Lists IDs, names, descriptions, and graph IDs from the process-local map.

### `GET /api/v1/agents/{id}`

Returns the lightweight agent summary.

### `GET /api/v1/agents/{id}/spec`

Returns the stored full AgentSpec.

### `GET /api/v1/agents/{id}/schema`

Returns the agent name/version/description plus declared inputs and outputs.
Outputs remain descriptive in v1.

## Execution

### `POST /api/v1/agents/{id}/execute`

Request:

```json
{
  "trigger_data": {"question":"hello"},
  "entry_node_id": null
}
```

The handler validates declared inputs and applies defaults, rejects live
agents, converts/validates the graph, injects trigger/MCP/memory/prompt data,
executes with a 300-second server timeout, persists staged memory on success,
and records the final run best-effort.

Response:

```json
{
  "session_id": "generated-id",
  "agent_id": "agent-id",
  "agent_name": "demo",
  "status": "Completed",
  "trace": [],
  "transcript": [],
  "state": {},
  "error": null
}
```

`entry_node_id` is currently ignored. Input errors return 422; live agents
return 422; timeout returns 504 and is not recorded as a normal completed run.
Agent-level `timeout_ms`, `max_iterations`, retry, and hook declarations are
not applied by this default host path.

### `POST /api/v1/agents/{id}/stream`

Returns `text/event-stream`. It is not equivalent to execute: input validation,
agent-memory seed/flush, server timeout, cancellation, and event coverage
differ. See [Streaming](STREAMING.md) for the exact contract.

## Live agents

| Method and path | Behavior |
|---|---|
| `POST /api/v1/agents/{id}/play` | Intended to schedule a live agent; 409 if already scheduled. |
| `POST /api/v1/agents/{id}/stop` | Unschedule and return completed cycle count. |
| `GET /api/v1/agents/{id}/cycles?limit=N` | Process-local cycle history. |
| `GET /api/v1/agents/{id}/memory` | Process-local agent KV memory. |
| `DELETE /api/v1/agents/{id}/memory` | Reset to initial values; 409 while scheduled. |

The current server constructs but does not start the Scheduler, so a successful
play response is not evidence that cycles advance. See [Live Agents](LIVE_AGENTS.md).

## Tools and templates

### `GET /api/v1/tools`

Returns the 52 registered tool descriptions, categories, input fields, and
output fields. Aliases are not listed.

### `GET /api/v1/templates`

Returns built-in template metadata (`id`, `name`, `category`, `description`,
required providers, tags), not the complete generated AgentSpec body.

## Sessions and run history

### `GET /api/v1/sessions?agent_id=...&limit=50`

Reads SQLite first when available, with optional agent filter and limit, then
falls back to the in-memory hot cache on repository failure/unavailability.

### `GET /api/v1/sessions/{id}`

Reads hot cache then SQLite. Hot-cache responses contain ID, status, trace, and
error. Persisted responses add agent identity, transcript, state, timing, and
duration. Clients must tolerate the additive shape difference.

### `GET /api/v1/sessions/{id}/otel-trace`

Returns an OpenTelemetry-compatible JSON resource/scope/span structure derived
from the stored run. It is a pull conversion, not an OTLP exporter. See
[Observability](OBSERVABILITY.md).

## Specialized endpoints

### `GET /api/v1/metrics`

Returns in-memory aggregate session/node counts and duration plus tool/agent
counts. It is JSON diagnostics, not Prometheus.

### `POST /api/v1/rag/search`

Accepts query/documents/chunk/search options, embeds documents/query through
the configured LLM resource, and returns ranked chunks. It is stateless per
request; see [RAG](RAG.md).

### `POST /api/v1/eval`

Accepts `input`, `output`, optional `context`, `duration_ms`, optional
`judge_model`, and required `eval_types`. Returns `{results, eval_count}`.
Programmatic and LLM-judge behavior is described in
[Advanced Subsystems](ADVANCED_SUBSYSTEMS.md).

### `POST /api/v1/universe/message`

Builds a request-scoped router from `message`, `agents`, optional `name`, and
strategy. It executes the selected agent only if its ID is already registered
in this process. `llm_classify` currently falls back to keyword matching in the
synchronous router.

### `POST /api/v1/universe/groupchat`

Accepts `topic`, at least two `{name, personality}` participants, and optional
`max_rounds`. It performs direct sequential LLM calls; participants are not
AgentSpec graphs.

## Orchestrator

The following privileged API manages tmux/Claude Code sessions rather than
graphs:

| Method and path | Purpose |
|---|---|
| `POST, GET /api/v1/orchestrator/sessions` | Spawn/list sessions. |
| `GET, DELETE /api/v1/orchestrator/sessions/{id}` | Detail/unregister external node. |
| `POST /api/v1/orchestrator/sessions/{id}/send` | Send instruction. |
| `GET /api/v1/orchestrator/sessions/{id}/output` | Capture pane output. |
| `POST /api/v1/orchestrator/sessions/{id}/stop` | Stop tmux session. |
| `POST /api/v1/orchestrator/sessions/{id}/restart` | Restart with optional mode/model/effort. |
| `POST /api/v1/orchestrator/sessions/register` | Register external node. |
| `POST /api/v1/orchestrator/sessions/{id}/status` | Update external status. |
| `POST /api/v1/orchestrator/sessions/{id}/unregister` | Stop external node. |
| `POST, GET /api/v1/orchestrator/sessions/{id}/activity` | Record/read activity. |
| `GET /api/v1/orchestrator/events` | SSE events. |
| `GET /api/v1/orchestrator/projects` | Project picker data. |
| `POST /api/v1/orchestrator/pick-folder` | Native host folder dialog. |

Full bodies, events, dependencies, and security implications are in
[Orchestrator](ORCHESTRATOR.md).

## Webhooks

### `POST /webhooks/{path}`

Accepts any JSON and returns `{received, path, body}`. It is a placeholder: it
does not match trigger declarations or execute an agent. When the server API
key is configured, normal auth middleware applies. Do not expose it as a
production webhook product without sender authentication, replay protection,
payload limits, and real routing.

## Error/status summary

| Status | Typical meaning |
|---:|---|
| 200 | Successful read/action. |
| 201 | Created. |
| 202 | Accepted async orchestrator send/activity. |
| 204 | Graph deleted. |
| 400 | Invalid JSON/shape/request. |
| 401 | Missing or invalid API key. |
| 404 | Resource not found. |
| 409 | Conflicting state. |
| 422 | Agent input/type/lifecycle validation. |
| 500 | Internal/backend/provider error depending on handler. |
| 501 | Native folder picker unsupported. |
| 504 | Synchronous execution timeout. |

## Related documentation

- [AgentSpec](../AGENT_SPEC.md)
- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [Streaming](STREAMING.md)
- [Observability](OBSERVABILITY.md)
- [Orchestrator](ORCHESTRATOR.md)
- [Security](../../SECURITY.md)
