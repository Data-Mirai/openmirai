# Operations Runbook

This runbook covers the current single-process OpenMirai server. It does not
assume the future distributed control-plane/worker architecture. Read
[Cloud Deployment Readiness](CLOUD-DEPLOYMENT.md) and
[Security](../../SECURITY.md) before exposing the service.

## 1. Supported operating shape

Operate one active `mirai serve` process per state domain. Use a durable local
block volume for SQLite, trusted AgentSpecs, an authenticated ingress, and
container/VM isolation. Multiple interchangeable replicas are not supported
because agents, graphs, memory, schedules, and in-flight runs are process-local.

## 2. Required configuration

```text
MIRAI_API_KEY=<high-entropy server secret>
MIRAI_LLM_PROVIDER=<explicit provider>
MIRAI_LLM_MODEL=<pinned model>
<PROVIDER>_API_KEY=<provider credential when required>
MIRAI_DB_PATH=/data/openmirai/engine.db
```

Start the server:

```bash
mirai serve --host 0.0.0.0 --port 3000 --db-path /data/openmirai/engine.db
```

Do not use the shared `--api-key` flag for a provider credential on `serve`;
prefer provider-specific variables and reserve `MIRAI_API_KEY` for server auth.

## 3. Preflight

Before every deployment verify:

- binary architecture and glibc compatibility (`file`, and `ldd` on Linux);
- `/data/openmirai` exists and is writable by the non-root runtime user;
- provider DNS/TLS/egress and credentials;
- required external programs for the allowed tool profile;
- sufficient disk, inode, memory, PID, and file-descriptor limits;
- ingress TLS and `X-API-Key` injection policy;
- no host root, Docker socket, or unrelated secret mounts;
- one active replica.

## 4. Liveness and readiness

`GET /health` is public and is suitable only for process liveness. It reports
version/build metadata, uptime, and in-memory counts. It does not verify
SQLite writes, disk capacity, provider/model reachability, tmux/Claude, or a
successful graph execution.

A production readiness check should independently verify:

1. the expected `/version` build/SHA;
2. SQLite path exists and has write/free-space headroom;
3. required provider endpoint is reachable with a bounded non-generation check;
4. profile-specific dependencies exist;
5. persistence has not fallen back to memory-only mode.

The code does not currently expose that composite readiness endpoint. Keep a
deployment unready when durability is required and SQLite failed to open.

## 5. Smoke test

```bash
curl -fsS http://127.0.0.1:3000/health
curl -fsS http://127.0.0.1:3000/version
curl -fsS -H "X-API-Key: $MIRAI_API_KEY" \
  http://127.0.0.1:3000/api/v1/tools
```

Then register and execute a trusted no-side-effect example, retrieve its
session, and confirm it remains available after a controlled restart.

## 6. Logs and failure signals

Capture structured stdout/stderr and correlate by session/agent where fields
exist. Alert on:

- SQLite open or save warnings;
- repeated authentication failures;
- HTTP 5xx/504 responses;
- provider timeouts and rate limits;
- tool panics/errors and excessive retries;
- resource saturation or disk pressure;
- unexpected process/tmux creation;
- streaming executions that continue after clients disconnect.

A successful execute response does not guarantee the final SQLite write
succeeded; persistence is best-effort and failure is logged.

## 7. SQLite backup

The safest current backup is a controlled stop:

1. stop ingress traffic;
2. wait for in-flight requests or the chosen termination grace period;
3. send SIGTERM and wait for `mirai` to exit;
4. copy `engine.db` from the persistent volume to versioned backup storage;
5. record binary version, build SHA, database checksum, and timestamp;
6. restart and run the smoke test.

If an external SQLite tool is available, its online backup API may be used,
but test that procedure against WAL mode. Do not copy only the main file during
active writes while ignoring `-wal`/`-shm` state.

Also back up `~/.openmirai/config.toml` and
`orchestrator_sessions.json` only when those features are intentionally used.
They have different security and recovery properties from run history.

## 8. Restore

1. stop the process and preserve the failed/current volume for forensics;
2. verify backup checksum and the compatible OpenMirai version;
3. restore the database as the runtime UID/GID;
4. start exactly one process and watch migration/open logs;
5. retrieve known sessions and execute a harmless test agent;
6. re-register agents/graphs because the default server does not restore them;
7. verify agent memory and schedules explicitly—they are not restored.

Practice restore regularly. A backup without a tested restore is not a
recovery plan.

## 9. Upgrade and rollback

Follow [Compatibility](../COMPATIBILITY.md). Back up first, canary the exact
binary/image, validate representative agents, and verify persistence. Database
downgrades are not supported automatically; rollback may require restoring the
pre-upgrade database together with the previous binary.

## 10. Graceful shutdown

The server handles SIGINT/SIGTERM through Axum graceful shutdown. Configure the
platform to stop routing new traffic, send SIGTERM, wait, then SIGKILL only
after the termination grace period. There is no durable handoff for in-flight
runs, and long tools or streams may outlive a short grace period.

The server has no explicit scheduler/tmux drain transaction. Inspect external
orchestrator sessions separately before host replacement.

## 11. Incident playbooks

### SQLite unavailable

- treat the instance as unready when durability is required;
- inspect permissions, parent directory, disk, filesystem, and lock errors;
- do not accept sustained production traffic in silent memory-only mode;
- recover or replace the volume, then verify stored sessions.

### Provider unavailable

- confirm DNS, TLS, egress, credential, base URL, model, and provider status;
- stop retry storms at ingress/client layers;
- remember `/health` can remain green;
- do not automatically switch models when reproducibility matters.

### Compromised agent or credential

- disable ingress and revoke/rotate server and provider credentials;
- isolate the worker/volume, preserve logs and state, and inspect side effects;
- rotate any secret visible to the process or mounted workspace;
- rebuild from a trusted image rather than cleaning the live container;
- review agent YAML, trigger data, MCP servers, egress, and executed tools.

### Run stuck or client disconnected

- streaming disconnect does not explicitly cancel execution;
- synchronous HTTP has a 300-second handler timeout, but tool/provider behavior
  still needs observation;
- restart loses in-flight state because durable checkpoints are not wired;
- avoid duplicate retries until idempotency/side effects are understood.

## 12. Capacity and retention

No official capacity envelope is published. Load-test the exact workload and
model. Track database growth, session payload/trace size, memory hot-cache
growth, concurrent streams, tool subprocesses, provider latency, and filesystem
usage. Long-term memory has no automatic TTL; define external retention and
privacy deletion procedures before production use.

## 13. Current operational gaps

- no repository Dockerfile/Compose/Kubernetes artifact;
- no composite readiness endpoint;
- no automatic backup/restore command;
- no durable agent/graph bootstrap;
- no distributed coordination, cancellation, or idempotency key;
- no native telemetry exporter, RBAC, rate limit, or tenant isolation;
- no tool allowlist in default server startup.

These are deployment blockers to resolve, not settings hidden elsewhere.

## Related documentation

- [Cloud deployment readiness](CLOUD-DEPLOYMENT.md)
- [Accepted GCP architecture](CLOUD-ARCHITECTURE-DECISION.md)
- [State, bootstrap, and recovery](STATE-BOOTSTRAP-RECOVERY.md)
- [SLO and observability](SLO-OBSERVABILITY.md)
- [Capacity and cost](CAPACITY-AND-COST-MODEL.md)
- [Disaster recovery](DISASTER-RECOVERY.md)
- [Security](../../SECURITY.md)
- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [Database schema](../database/SCHEMA.md)
- [Observability](../backend/OBSERVABILITY.md)
