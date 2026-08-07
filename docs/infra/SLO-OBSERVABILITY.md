# SLO and Observability Specification

This specification defines the first production objectives and the telemetry
needed to operate OpenMirai. It separates platform availability from external
LLM behavior and agent-defined failures.

## Service level objectives

Measured monthly for the private production endpoint:

| SLI | Target | Measurement |
|---|---:|---|
| authenticated API availability | 99.5% | successful synthetic authenticated request / valid attempts |
| health endpoint availability | 99.9% | load-balancer uptime probe |
| API admission latency | p95 < 1 s | request received to run accepted, excluding execution |
| simple non-LLM graph duration | p95 < 5 s | fixed synthetic graph |
| backup success | 100% daily | verified backup completed in 26-hour window |
| restore readiness | quarterly | full restore completes within 4 hours |

99.5% permits approximately 3 hours 39 minutes of unavailability in a
30.44-day month. Planned maintenance counts unless the business explicitly
changes this policy.

LLM execution latency does not have one global target because it depends on
model, prompt, provider, rate limits, and graph topology. Track it by provider,
model, agent, and node type; assign product-specific objectives later.

## Error classification

Every failed run/request must have one primary class:

- `platform`: process crash, disk, database, internal panic, unavailable VM;
- `configuration`: invalid AgentSpec, missing secret, missing executable;
- `provider`: upstream timeout, quota, rate limit, invalid provider response;
- `tool`: shell/Git/browser/MCP failure;
- `policy`: IAP/API key/egress/tool authorization denial;
- `user`: invalid request or intentional cancellation;
- `capacity`: queue/concurrency/resource limit exceeded.

Do not count expected 4xx validation and policy denials as platform
unavailability, but alert on sudden changes because they can indicate misuse or
configuration drift.

## Required telemetry

### Logs

Emit structured JSON with timestamp, severity, service/environment, image
digest or build SHA, request ID, run ID, agent ID, node ID, provider/model,
tool family, duration, status, and error class. Redact secrets, authorization
headers, full prompts, full tool payloads, and personal data by default.

Retention defaults:

- operational logs: 14 days;
- audit/security events: 90 days;
- run records: 30 days, subject to an implemented pruning job;
- incident evidence: retained according to incident policy.

### Metrics

Collect at minimum:

- HTTP requests, status, latency, and in-flight count;
- runs started/completed/failed/interrupted and duration;
- node/tool duration and failure by family;
- LLM calls, duration, errors, retries, input/output tokens, estimated cost;
- active Claude/tmux/browser processes and orphaned sessions;
- CPU, memory, process count, open files, disk bytes/inodes, disk latency;
- SQLite size, write failures, integrity failures, backup age/duration;
- IAP/API-key denials and egress policy denials;
- deployment version and last successful boot-manifest load.

Metrics must avoid unbounded labels such as raw prompt, command, URL, run ID,
or user-generated agent name.

### Traces

Use one trace per request/run with spans for graph, node, tool, provider HTTP,
SQLite, MCP, browser, and orchestrator operations. Propagate request/run IDs in
logs. The current OpenMirai trace facilities are not a complete managed
OpenTelemetry pipeline; exporting them is implementation work.

## Dashboard

One launch dashboard must show:

1. SLO/error-budget status;
2. traffic, errors, and admission latency;
3. run outcomes and duration percentiles;
4. provider latency/error/token/spend by model;
5. VM/container saturation;
6. disk/SQLite capacity and backup age;
7. active/orphaned full-profile processes;
8. current image digest and last deployment.

## Alerts

| Alert | Initial threshold | Priority |
|---|---|---|
| service unavailable | 3 consecutive 1-minute probes | page |
| API 5xx | >5% for 5 minutes with minimum traffic | page |
| disk usage | >80% warning; >90% critical | ticket/page |
| memory | >85% for 10 minutes or OOM | page |
| backup age | >26 hours | page |
| SQLite write/integrity error | any integrity error; repeated writes | page |
| provider error | >20% for 10 minutes by provider | ticket/page if all providers |
| abnormal spend | daily projected spend exceeds budget threshold | ticket |
| auth denials | anomalous spike or repeated principal | security ticket |
| orphaned process/session | above zero after reconciliation grace period | ticket |

Tune thresholds after the non-production soak test; do not remove an alert
without documenting which other control detects the same failure.

## Synthetic probes

- public load-balancer health probe to `/health`;
- authenticated readiness request through IAP;
- hourly deterministic no-LLM graph;
- low-frequency provider probe for each enabled provider using a cheap model;
- daily SQLite write/read and backup verification;
- daily execution-profile inventory for required binaries.

Synthetic prompts and outputs must contain no sensitive data.

## Error-budget policy

At 50% monthly budget consumption, review causes and pause risky changes. At
100%, freeze non-remediation production releases until reliability recovers or
the owner explicitly accepts the risk. Provider-specific incidents are tracked
separately but still require customer-visible communication when they make the
service unusable.
