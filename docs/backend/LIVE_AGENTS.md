# Live Agents, Scheduling, and Triggers

This guide separates the declared live-agent model from what OpenMirai 0.7.0
actually executes. Live agents are not background versions of `mirai run`;
they depend on the HTTP server scheduler and process-local memory.

## AgentSpec contract

```yaml
name: monitor
agent_type: live
schedule:
  interval_seconds: 60
  max_cycles: 100
  on_cycle_error: continue
graph:
  memory:
    persist: cycle
    keys:
      last_seen: null
  nodes: []
  edges: []
```

Rules:

- `live` requires `schedule`;
- `managed` forbids `schedule`;
- exactly one of `interval_seconds` and `cron` is required;
- intervals must be at least one second;
- cron is represented but rejected in v1;
- `on_cycle_error` is `continue` or `stop`;
- direct `mirai run` rejects live agents.

## HTTP lifecycle

| Endpoint | Purpose |
|---|---|
| `POST /api/v1/agents/{id}/play` | Intended to start scheduled cycles. |
| `POST /api/v1/agents/{id}/stop` | Stop a registered schedule and reset cycle-scoped memory. |
| `GET /api/v1/agents/{id}/cycles` | Return bounded in-memory cycle history. |
| `GET /api/v1/agents/{id}/memory` | Return current process-local agent KV memory. |
| `DELETE /api/v1/agents/{id}/memory` | Reset memory to declared initial values. |

Agents must first be registered in the same server process. Registration,
play state, memory, and history are not restored from SQLite after restart.

## Intended cycle flow

```text
play
 -> clear cycle memory
 -> register interval task
 -> wait interval
 -> inject cycle_number + triggered_by=scheduler
 -> seed SharedState from agent memory
 -> execute graph
 -> persist staged state/memory output
 -> append cycle result
 -> continue, stop on error policy, or stop at max_cycles
```

Each cycle builds a new execution context and run state. Memory is explicitly
seeded and flushed around it; ordinary node outputs do not automatically
become cross-cycle memory.

## Critical current wiring limitation

`Scheduler::schedule_agent()` executes its loop only while the scheduler's
atomic `running` flag is true. `AppState::new()` constructs the scheduler, but
the current `serve()` startup path does not call `Scheduler::start()`.

Consequently, `play` can return an intended playing state while scheduled
cycles do not progress. Treat live execution as incomplete until startup is
wired and an end-to-end elapsed-time test proves cycles occur.

## Memory modes

| Mode | Intended scope | Current durability |
|---|---|---|
| `none` | Initial values every cycle/run | Process memory only |
| `cycle` | Carry during one play session; reset stop→play | Process memory only |
| `execution` | Carry across play/stop and managed executions | Process memory only |

`state/memory` stages writes in node output. The host copies them to
`AgentMemoryStore` only after a successful supported execution path.
Streaming does not currently seed or flush this memory.

## Triggers

AgentSpec trigger declarations support fields for manual, webhook, schedule,
heartbeat, and event-style metadata. The built-in `trigger/*` tools are graph
entry nodes; they do not register external listeners themselves.

Current host status:

- manual HTTP/CLI payload injection: wired;
- interval live scheduling: implemented but server startup is incomplete;
- cron: rejected;
- dynamic webhook receipt: endpoint exists but only acknowledges payload;
- automatic trigger-to-agent dispatch: not implemented;
- event bus and heartbeat delivery: require host-side integration.

## Failure and stop behavior

`continue` retains scheduling after a failed cycle; `stop` is intended to end
the live task. `max_cycles` bounds successful/attempted scheduler iterations
according to the scheduler implementation. There is no distributed lease or
leader election, so starting the same live agent on multiple future replicas
would duplicate work.

Stop is process-local and not a durable cancellation record. Process death
ends tasks and loses their in-flight state.

## Production acceptance criteria

Before enabling live agents:

1. call `Scheduler::start()` during server boot and stop/drain it on shutdown;
2. prove cycles execute with an integration test using paused Tokio time;
3. persist desired schedules and reload them explicitly;
4. define missed-run, overlap, drift, and clock-change semantics;
5. add distributed ownership before replicas;
6. make cycle execution idempotent or side-effect aware;
7. persist memory/history when the contract says `execution`;
8. expose metrics for scheduled, running, succeeded, failed, skipped, and late cycles;
9. align streaming and synchronous memory behavior;
10. document retention and operator recovery.

## Related documentation

- [AgentSpec](../AGENT_SPEC.md)
- [Memory](MEMORY.md)
- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [Operations](../infra/OPERATIONS.md)
