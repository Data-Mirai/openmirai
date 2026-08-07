# Streaming Execution and SSE Contract

OpenMirai exposes graph progress through
`POST /api/v1/agents/{id}/stream`. The response is a Server-Sent Events stream,
not a token-only LLM stream and not a behaviorally identical encoding of the
synchronous execute endpoint.

## Request

```http
POST /api/v1/agents/{id}/stream
Content-Type: application/json
Accept: text/event-stream
X-API-Key: <server key when configured>

{"trigger_data":{"question":"hello"}}
```

The request type also contains `entry_node_id`, but the current handler ignores
it. The graph runner selects the first entry node.

## Wire format

Each frame is:

```text
event: graph.started
data: {"event":"graph.started","data":{"graph_name":"demo","node_count":3}}

```

The JSON in `data:` repeats the event name because `StreamEvent` uses an
internally tagged Serde representation. Clients should use the SSE `event:`
line for dispatch and retain tolerance for additional JSON fields.

Response headers are `Content-Type: text/event-stream`, `Cache-Control:
no-cache`, and `Connection: keep-alive`. No SSE `id`, retry directive,
heartbeat, or replay cursor is currently emitted.

## Event catalog

| Event | Payload fields | Emitted by current live runner? |
|---|---|---:|
| `graph.started` | `graph_name`, `node_count` | Yes |
| `node.started` | `node_id`, `tool_type` | Yes |
| `node.token` | `node_id`, `token` | No |
| `node.completed` | `node_id`, `tool_type`, `duration_ms`, `output_keys`, `output` | Yes |
| `node.error` | `node_id`, `tool_type`, `error` | Not by the normal runner path |
| `fanout.started` | `source_node`, `parallel_nodes` | No |
| `fanout.completed` | `succeeded`, `failed` | No |
| `graph.completed` | `status`, `total_duration_ms`, `nodes_executed`, `output` | Yes |
| `graph.error` | `error` | Yes for pre-run validation/runner errors in the streaming helper |

`graph.completed` is the normal terminal event. A stream can also simply end
after `graph.error` or transport failure. A session ID is not included in the
event contract even though the server records the completed result under a
new internal session ID.

## Ordering

Sequential nodes normally produce:

```text
graph.started
node.started
node.completed
...
graph.completed
```

Do not infer a complete partial order for fan-out from the current events.
Immediate fan-out children use a specialized path and the declared fan-out
events are not emitted. Future versions may interleave concurrent node events.

## Differences from synchronous execute

| Behavior | `/execute` | `/stream` |
|---|---|---|
| Input contract validation/defaults | Yes | No |
| Live-agent rejection | Yes | Handler currently follows its own path |
| Agent KV memory seed/persist | Yes | No |
| Server timeout wrapper | 300 seconds | None |
| Result body | JSON response | SSE terminal event |
| Final run persistence | Best effort | Best effort after helper returns a result |
| Disconnect cancellation | Request future ends/cancels at handler boundary | Delivery ends, execution task is not explicitly cancelled |

Clients must not switch between the endpoints assuming identical semantics.

## Backpressure and disconnects

The handler uses bounded channels of 256 events/frames. If the client stops
reading, delivery can back up. When the byte-stream receiver closes, the drain
task stops, but the separately spawned execution task is not aborted. Tool and
provider side effects may therefore continue after a client disconnects.

There is no public cancel endpoint, durable run ownership, resume token, or
idempotency key. Production clients should avoid automatically starting a
replacement run until they understand whether the first run performed side
effects.

## Client requirements

- parse frames incrementally across arbitrary network chunks;
- support multi-line SSE data if added later;
- dispatch by `event:` and parse `data:` as JSON;
- ignore unknown events/fields;
- treat `graph.completed` or `graph.error` as terminal when received;
- implement client time limits without assuming server cancellation;
- log the agent/request correlation externally because no stream session ID is
  currently provided;
- send `X-API-Key` on authenticated servers.

## Replay helper versus live streaming

`trace_to_events()` can convert a completed `ExecutionResult` into synthetic
events. It produces node error events from stored trace entries and reconstructs
outputs from final state. That helper is not an HTTP replay endpoint and its
event sequence is not proof that the live runner emitted the same events.

## Production gaps

- input and memory parity with `/execute`;
- explicit cancellation and ownership;
- timeout and maximum stream duration;
- session ID in the initial event;
- SSE heartbeat and proxy timeout guidance;
- event IDs/replay or resume behavior;
- real LLM token events;
- node error and fan-out events from the live runner;
- bounded output/event payload sizes.

## Related documentation

- [HTTP API](API.md)
- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [SDK behavior](../SDK.md)
- [Observability](OBSERVABILITY.md)
