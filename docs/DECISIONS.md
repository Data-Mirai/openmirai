# Architecture Decisions

This register records the decisions needed to understand and operate the
current system. It describes the implementation as it exists; it is not a list
of permanent constraints.

## D-001 — Rust engine and a small set of host binaries

**Status:** accepted.

The engine is implemented in Rust and exposed through the `mirai` CLI/server
host. This provides a single compiled engine artifact while still allowing
execution profiles to require external programs such as shells, browsers,
containers, MCP servers, or Claude Code.

**Consequence:** the core can be embedded, but the embedding host must supply
resources, stores, tools, hooks, and lifecycle management explicitly.

## D-002 — YAML is the canonical on-disk AgentSpec format

**Status:** accepted.

The CLI file loader reads YAML. The HTTP API can carry the same structure as
JSON because request bodies use Serde.

**Consequence:** SDK helpers and examples must not imply that CLI file loading
accepts JSON. See [AGENT_SPEC.md](AGENT_SPEC.md).

## D-003 — Hexagonal boundaries around engine ports

**Status:** accepted.

Domain execution depends on ports for LLMs, tools, memory, storage, hooks, and
other effects. Adapters connect those ports to providers and infrastructure.

**Consequence:** adding an adapter should not make the runner depend directly
on provider-specific HTTP or storage details.

## D-004 — Graph edges control traversal; `data_map` controls values

**Status:** accepted.

An edge determines which node may run next. Values do not automatically flow
between arbitrary node ports: the edge mapping defines the copy operation.

**Consequence:** an edge without the required mapping can produce a structurally
valid graph whose downstream node still lacks input.

## D-005 — Resource and host configuration are separate

**Status:** accepted.

AgentSpec declares resources and behavior. The process host resolves secrets,
constructs clients, registers built-in tools, opens persistence, and decides
which optional services are started.

**Consequence:** a declared provider, schedule, hook, or MCP credential does not
prove that the default host has wired every associated behavior.

## D-006 — Current deployment unit is one stateful process

**Status:** current constraint.

The standard server uses process memory plus local SQLite and local files. Live
scheduling and some orchestration state also depend on the process host.

**Consequence:** deploy one replica with persistent storage until leases,
distributed coordination, and shared state are implemented. See
[infra/CLOUD-DEPLOYMENT.md](infra/CLOUD-DEPLOYMENT.md).

## D-007 — Tools are the principal effect boundary

**Status:** accepted.

Network access, filesystem access, process execution, browser automation, and
other side effects occur through tools or adapters.

**Consequence:** tool registration is a security policy. Production hosts should
register only the tools required by their execution profile.

## D-008 — TLS is normally terminated outside the process

**Status:** accepted for the current server.

The built-in server speaks HTTP. A reverse proxy, ingress, or cloud load
balancer supplies TLS and external policy controls.

**Consequence:** non-loopback deployments require an authenticated, encrypted
edge and must not expose the server directly to the public Internet.

## D-009 — Partial features are documented by reachability

**Status:** accepted documentation policy.

Feature status is determined by an end-to-end path from an entry point, not by
module existence. The labels in [MAINTAINING_DOCS.md](MAINTAINING_DOCS.md) are
used throughout the documentation.

## D-010 — Initial cloud target is one GCP VM in `us-east1`

**Status:** accepted for the first cloud implementation.

Run one OpenMirai container on a private Linux Compute Engine VM with Docker
Compose, a Persistent Disk, HTTPS load balancing, IAP, Secret Manager, and
Cloud Operations. Default to `us-east1`; keep region and zone configurable.

**Consequence:** the deployment prioritizes a reproducible learning/production
baseline over immediate horizontal availability. See
[CLOUD-ARCHITECTURE-DECISION.md](infra/CLOUD-ARCHITECTURE-DECISION.md).

## D-011 — Initial cloud users and agents are trusted

**Status:** accepted with risk.

The initial image includes shell, filesystem, Git, MCP, browser, and
Claude/tmux capabilities for named internal users behind IAP. This is a
full-execution remote-code boundary, not an untrusted sandbox.

**Consequence:** use a dedicated private VM, least privilege, narrow mounts,
egress controls, audit, and no untrusted/mutually distrusting users. See
[EXECUTION-SECURITY-PROFILES.md](infra/EXECUTION-SECURITY-PROFILES.md).

## D-012 — Horizontal growth requires control-plane/worker separation

**Status:** target architecture.

Scale vertically on the initial VM. Before adding active replicas, externalize
definitions and run state, add a durable queue/leases/idempotency/cancellation,
and split restricted and full-execution worker profiles. GKE Standard is the
preferred later runtime after those application contracts exist.

**Consequence:** Kubernetes migration is gated by the protocol and acceptance
tests in
[CONTROL-PLANE-WORKER-PROTOCOL.md](infra/CONTROL-PLANE-WORKER-PROTOCOL.md),
not by traffic alone.

## D-013 — Initial reliability and retention defaults

**Status:** accepted for planning.

The initial targets are 99.5% monthly authenticated API availability, RPO 24
hours, RTO 4 hours, run retention 30 days, operational logs 14 days, and audit
events 90 days.

**Consequence:** daily verified backups, quarterly restore exercises, explicit
pruning/lifecycle work, and SLO monitoring are launch requirements.

## Recording a new decision

Add a numbered entry with context, decision, consequences, and status. Link it
from the affected architecture or operations document. If a decision is
replaced, keep the old entry and mark it superseded so upgrade reasoning remains
traceable.
