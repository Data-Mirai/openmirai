# Observability, Traces, Metrics, and Benchmarks

OpenMirai produces useful per-run diagnostic objects but does not yet ship a
complete telemetry export pipeline. This guide separates runner evidence,
HTTP aggregate views, OpenTelemetry-compatible JSON, and local benchmark logs.

## Data products

| Product | Scope | Storage/export |
|---|---|---|
| State | Final node output maps | Returned; server final runs persist to SQLite. |
| Trace | Per-node status, timing, retries, error | Returned and persisted for final HTTP runs. |
| Transcript | Human-oriented runner decisions | Returned and persisted. |
| Trace tree | Root plus flat child spans | Computed in process for CLI rendering. |
| Execution metrics | Counts/durations grouped by tool | Computed from traces. |
| `/api/v1/metrics` | Aggregate in-memory server/session view | Generated on request; not Prometheus format. |
| `/sessions/{id}/otel-trace` | OTLP-shaped JSON span export for one stored run | Generated on request; not pushed to a collector. |
| Benchmark JSONL | Cold-start/execution/LLM/tool/memory samples | Append-only local file when enabled. |

## Runner trace

Every normal node trace entry contains node/tool identity, status, duration,
retry count, start/end timestamps, and optional error. Static fan-out children
use a specialized execution path, so inspect actual traces rather than assuming
every declared graph node receives identical hook/checkpoint/event treatment.

`SharedState` answers what nodes returned. Trace answers what ran and how long
it took. Transcript answers how the runner routed and completed in
human-readable terms. None of these is automatically redacted.

## CLI

`mirai run agent.yaml --trace` renders the graph root and node spans as an
ASCII tree and prints aggregate timing/retry metrics. Normal result JSON still
contains state, trace, and transcript.

## HTTP metrics

`GET /api/v1/metrics` summarizes completed sessions currently known to the
server. Treat it as a diagnostic JSON endpoint, not a durable monitoring
backend. In-memory counts reset and the session hot cache is bounded.

Missing production metrics include request concurrency, queue depth, provider
latency/status, SQLite failures, streaming disconnects, active subprocesses,
scheduler lateness, persistence fallback, and resource saturation.

## OpenTelemetry-compatible session export

```http
GET /api/v1/sessions/{id}/otel-trace
X-API-Key: <server key>
```

The response creates a graph root span and one child per stored trace entry,
with trace/span IDs, nanosecond timestamps, tool type, retry count, error, and
status. Real timestamps are used for 0.7.0 traces; older zero-timestamp records
fall back to a synthetic serial timeline.

This endpoint does not:

- speak OTLP/gRPC or OTLP/HTTP to a collector;
- continuously export spans;
- propagate W3C trace context through incoming requests/provider calls;
- represent true nested subgraphs/tool/provider spans;
- provide sampling, batching, resource attributes, or exporter retries.

An operator must fetch/transform the JSON or add a native exporter.

## Benchmarking

Enable with `--benchmark`, `MIRAI_BENCHMARK=1`, and optionally
`MIRAI_BENCHMARK_FILE`. Records are append-only JSON lines with a timestamp,
metric type, milliseconds or bytes, and context.

Metric types are:

- `cold_start`;
- `execution`;
- `llm_latency`;
- `tool_latency`;
- `memory_usage` (Linux RSS via `/proc/self/status`; platform behavior differs).

Benchmark output is local process instrumentation, not a statistically sound
capacity result by itself. Record hardware, OS, build profile/SHA, provider,
model, warm/cold state, graph, concurrency, sample count, and percentile
methodology when publishing results.

## Logging and sensitive data

Use structured stdout collection and attach request/session/agent correlation
where available. State, trace, transcript, prompts, MCP payloads, URLs, file
paths, and errors may contain secrets or personal data. Apply redaction before
central export and define retention/deletion policies.

Never log API keys, authorization headers, complete environment variables, or
unfiltered agent configuration.

## Recommended production pipeline

```text
OpenMirai stdout + run IDs
 -> collector/sidecar
 -> central logs

run trace / future native spans
 -> OTLP collector
 -> tracing backend

service/provider/DB/scheduler metrics
 -> Prometheus-compatible exporter
 -> dashboard + alerts
```

Add resource attributes for service version, build SHA, deployment, replica,
provider, and model without placing user content in high-cardinality labels.

## Alerting minimum

- process restart/crash and readiness failure;
- authentication failures and rate anomalies;
- execute/stream error and timeout rate;
- provider latency/rate limits;
- SQLite open/save failure;
- disk/memory/PID/file-descriptor saturation;
- tool retry/error spikes;
- long-running streams and orphaned work;
- scheduler missed/failed cycles when live agents become wired.

## Current gaps

- no native exporter or trace-context propagation;
- no Prometheus endpoint;
- no request/session correlation middleware standard;
- no centralized redaction policy;
- no replay/debug UI in this repository;
- partial SSE event coverage;
- no published performance/capacity baseline.

## Related documentation

- [Streaming](STREAMING.md)
- [Operations](../infra/OPERATIONS.md)
- [Cloud deployment](../infra/CLOUD-DEPLOYMENT.md)
- [System lifecycle](../SYSTEM_LIFECYCLE.md)
