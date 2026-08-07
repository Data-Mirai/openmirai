# Future Control-Plane and Worker Protocol

This is the application contract required before horizontal scaling. It is a
target design, not current OpenMirai behavior.

## Responsibilities

| Control plane | Worker |
|---|---|
| authenticate and authorize users | claim jobs matching capabilities |
| validate/version AgentSpecs | construct execution context |
| persist definitions and run metadata | execute graph and tools |
| create/cancel jobs | heartbeat and honor cancellation/deadline |
| enforce quotas and budgets | upload events/results/artifacts |
| expose API/SSE | clean workspace and terminate child processes |

## Job envelope

```json
{
  "protocol_version": 1,
  "job_id": "uuid",
  "run_id": "uuid",
  "tenant_id": "tenant",
  "principal_id": "user",
  "agent_revision": "immutable-revision",
  "input_ref": "object-or-inline-reference",
  "execution_profile": "restricted|full-trusted",
  "required_capabilities": ["llm", "git"],
  "deadline": "RFC3339",
  "attempt": 1,
  "idempotency_key": "opaque",
  "trace_context": "W3C traceparent"
}
```

Large inputs, outputs, and artifacts belong in object storage with scoped,
short-lived references, not in the queue message.

## State machine

```text
accepted -> queued -> leased -> running -> succeeded
                              |-> failed
                              |-> cancelled
                              |-> timed_out
                   lease loss -> queued (new attempt) or dead_letter
```

State transitions use compare-and-set on the current state/version. A worker
cannot complete a job after its lease has expired or been superseded.

## Delivery semantics

Assume at-least-once delivery. Workers must tolerate duplicate messages.
Claiming uses a lease with worker ID, expiry, and periodic heartbeat. Completion
uses a unique result commit keyed by run/attempt. The control plane accepts only
the active attempt and preserves late results for audit without publishing them
as canonical.

Tools with external side effects require their own idempotency key or must be
marked non-retriable. Automatic retry of shell, Git push, email, webhook, or
other external mutations is unsafe without effect-level semantics.

## Events and streaming

Workers append ordered run events with run ID, attempt, monotonically
increasing sequence number, type, timestamp, and redacted payload. The control
plane streams from the durable event log. Reconnecting clients provide the last
sequence received. Duplicate event delivery is permitted; reordering within an
attempt is not.

## Cancellation and deadlines

Cancellation is durable state, not a transient signal. Workers poll/subscribe,
stop scheduling new nodes, terminate child processes, emit terminal status, and
release the lease. A hard deadline is enforced independently by the worker
runtime. The control plane marks a lost worker only after heartbeat/lease grace.

## Capability routing

Workers publish image version, protocol versions, execution profile, tool
families, provider adapters, browser/MCP/orchestrator support, and current
capacity. Jobs are leased only to a compatible pool.

Full-execution workers use separate nodes, identities, network policies,
secrets, and persistent-workspace rules. They must not share a pod/container
security boundary with the control plane.

## Version compatibility

Protocol and AgentSpec revisions are immutable in a job. During rolling
upgrades, the queue routes only compatible jobs. Support at least the current
and previous protocol revision or drain the old revision before promotion.
Unknown required fields/capabilities fail safely rather than being ignored.

## GCP target mapping

A likely later mapping is GKE Standard for control plane/workers, Cloud SQL for
metadata, Pub/Sub or another durable queue with explicit lease semantics, and
Cloud Storage for artifacts. Select actual products only after validating that
their ordering, lease, retry, size, and residency behavior satisfies this
protocol.

## Migration gates

Do not activate a second worker until duplicate delivery, worker crash,
heartbeat loss, cancellation, deadline, poison job/DLQ, event reconnect,
capability mismatch, rolling-version compatibility, and external-side-effect
tests all pass.
