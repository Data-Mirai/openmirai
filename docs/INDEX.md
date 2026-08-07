# OpenMirai — Documentation Index

## 1. Overview

Open-source agent execution engine compiled in Rust. Runs agentic workflows defined as YAML graphs. One binary, any LLM, 52 built-in tools, and 7 LLM providers. Alternative to LangGraph, CrewAI, and Google ADK. Version: v0.7.0.

## 2. Stack

Rust engine + Axum HTTP server + SQLite + 7 LLM providers. Full details → [ARCHITECTURE.md](ARCHITECTURE.md)

## 3. Documentation Map

| Document | Description | Path |
|---|---|---|
| SYSTEM_LIFECYCLE.md | Canonical end-to-end build, startup, execution, data-flow, persistence, streaming, and shutdown lifecycle | [SYSTEM_LIFECYCLE.md](SYSTEM_LIFECYCLE.md) |
| ARCHITECTURE.md | Stack, modules, conventions | [ARCHITECTURE.md](ARCHITECTURE.md) |
| AGENT_SPEC.md | Formal AgentSpec v1 fields, defaults, validation, and host wiring | [AGENT_SPEC.md](AGENT_SPEC.md) |
| agent-spec.schema.json | Machine-readable AgentSpec authoring schema | [../schemas/agent-spec.schema.json](../schemas/agent-spec.schema.json) |
| USAGE.md | User guide, graph patterns, and tool catalog | [../USAGE.md](../USAGE.md) |
| CLI.md | `mirai` command reference, interactive mode, sessions | [CLI.md](CLI.md) |
| CLI.es.md | Spanish translation of CLI.md | [CLI.es.md](CLI.es.md) |
| DOMINIO.md | Domain glossary, roles, capabilities | [producto/DOMINIO.md](producto/DOMINIO.md) |
| FLUJOS.md | Execution flows, state machines, business rules | [producto/FLUJOS.md](producto/FLUJOS.md) |
| API.md | HTTP API endpoints and contracts | [backend/API.md](backend/API.md) |
| openapi.yaml | Machine-readable HTTP route inventory | [backend/openapi.yaml](backend/openapi.yaml) |
| STREAMING.md | SSE events, ordering, parity, backpressure, and cancellation | [backend/STREAMING.md](backend/STREAMING.md) |
| LIVE_AGENTS.md | Live scheduling, triggers, memory, and current startup limitation | [backend/LIVE_AGENTS.md](backend/LIVE_AGENTS.md) |
| ORCHESTRATOR.md | tmux/Claude Code process orchestration API and operations | [backend/ORCHESTRATOR.md](backend/ORCHESTRATOR.md) |
| MCP.md | MCP transports, lifecycle, tool contract, limitations, and security | [backend/MCP.md](backend/MCP.md) |
| OBSERVABILITY.md | Trace, metrics, OpenTelemetry-compatible export, logs, and benchmarks | [backend/OBSERVABILITY.md](backend/OBSERVABILITY.md) |
| ADVANCED_SUBSYSTEMS.md | Eval, universe, intelligence, energy, render, benchmark, catalog, and status | [backend/ADVANCED_SUBSYSTEMS.md](backend/ADVANCED_SUBSYSTEMS.md) |
| BUILTIN_TOOLS.md | Implementer contracts and security boundaries for all built-in tools | [backend/BUILTIN_TOOLS.md](backend/BUILTIN_TOOLS.md) |
| MEMORY.md | Memory subsystem (short/long-term, agent KV, CLI sessions) | [backend/MEMORY.md](backend/MEMORY.md) |
| MEMORY.es.md | MEMORY.md — Spanish translation | [backend/MEMORY.es.md](backend/MEMORY.es.md) |
| PRIMITIVES.md | Reusable engine code patterns | [backend/PRIMITIVES.md](backend/PRIMITIVES.md) |
| RAG.md | RAG/search subsystem (providers, hybrid, embeddings) | [backend/RAG.md](backend/RAG.md) |
| RAG.es.md | RAG.md — Spanish translation | [backend/RAG.es.md](backend/RAG.es.md) |
| SCHEMA.md | SQLite schema (tables, stores) | [database/SCHEMA.md](database/SCHEMA.md) |
| STORAGE.md | Storage backends, vault, FileRef | [database/STORAGE.md](database/STORAGE.md) |
| INFRA.md | Build, CI/CD, release, deploy, env vars | [infra/INFRA.md](infra/INFRA.md) |
| CLOUD-DEPLOYMENT.md | Current Docker/cloud readiness, state boundaries, hardening, scaling blockers, and deployment roadmap | [infra/CLOUD-DEPLOYMENT.md](infra/CLOUD-DEPLOYMENT.md) |
| CLOUD-ARCHITECTURE-DECISION.md | Accepted GCP region, topology, trust boundary, and scale-out decision | [infra/CLOUD-ARCHITECTURE-DECISION.md](infra/CLOUD-ARCHITECTURE-DECISION.md) |
| CLOUD-IMPLEMENTATION-PLAN.md | Ordered implementation phases and cloud definition of done | [infra/CLOUD-IMPLEMENTATION-PLAN.md](infra/CLOUD-IMPLEMENTATION-PLAN.md) |
| CONTAINER-RUNTIME-CONTRACT.md | Image, process, filesystem, secret, network, health, startup, and shutdown contract | [infra/CONTAINER-RUNTIME-CONTRACT.md](infra/CONTAINER-RUNTIME-CONTRACT.md) |
| EXECUTION-SECURITY-PROFILES.md | Restricted, full-trusted, and unsupported untrusted execution boundaries | [infra/EXECUTION-SECURITY-PROFILES.md](infra/EXECUTION-SECURITY-PROFILES.md) |
| STATE-BOOTSTRAP-RECOVERY.md | State durability inventory, boot manifest, backup, and restore semantics | [infra/STATE-BOOTSTRAP-RECOVERY.md](infra/STATE-BOOTSTRAP-RECOVERY.md) |
| SLO-OBSERVABILITY.md | Initial SLOs, telemetry, dashboards, alerts, and error-budget policy | [infra/SLO-OBSERVABILITY.md](infra/SLO-OBSERVABILITY.md) |
| CAPACITY-AND-COST-MODEL.md | Workload sizing, regional validation, cost components, and scaling triggers | [infra/CAPACITY-AND-COST-MODEL.md](infra/CAPACITY-AND-COST-MODEL.md) |
| RELEASE-SUPPLY-CHAIN.md | WIF-based CI/CD, immutable images, SBOM/provenance, promotion, and rollback | [infra/RELEASE-SUPPLY-CHAIN.md](infra/RELEASE-SUPPLY-CHAIN.md) |
| CLOUD-ACCEPTANCE-TESTS.md | End-to-end release gates for GCP, recovery, tools, identity, and telemetry | [infra/CLOUD-ACCEPTANCE-TESTS.md](infra/CLOUD-ACCEPTANCE-TESTS.md) |
| CONTROL-PLANE-WORKER-PROTOCOL.md | Future distributed job, lease, event, cancellation, and capability protocol | [infra/CONTROL-PLANE-WORKER-PROTOCOL.md](infra/CONTROL-PLANE-WORKER-PROTOCOL.md) |
| MULTITENANCY-AUTHORIZATION.md | Future tenant isolation, roles, credentials, quotas, and audit model | [infra/MULTITENANCY-AUTHORIZATION.md](infra/MULTITENANCY-AUTHORIZATION.md) |
| DISASTER-RECOVERY.md | RPO/RTO, failure scenarios, regional recovery, failback, and exercises | [infra/DISASTER-RECOVERY.md](infra/DISASTER-RECOVERY.md) |
| OPERATIONS.md | Single-process runbook: readiness, backup, restore, upgrade, incidents, shutdown | [infra/OPERATIONS.md](infra/OPERATIONS.md) |
| SECURITY.md | Security policy, threat model, disclosure, tool risks, deployment controls | [../SECURITY.md](../SECURITY.md) |
| COMPATIBILITY.md | Versioned surfaces, compatibility expectations, and upgrade process | [COMPATIBILITY.md](COMPATIBILITY.md) |
| SDK.md | Python/TypeScript behavior, parity, auth limitations, and production requirements | [SDK.md](SDK.md) |
| RUST_EMBEDDING.md | Embedding the engine crate and wiring resources, tools, hooks, checkpoints | [RUST_EMBEDDING.md](RUST_EMBEDDING.md) |
| COOKBOOK.md | Progressive examples from local execution to a cloud-ready operating profile | [COOKBOOK.md](COOKBOOK.md) |
| DECISIONS.md | Current architecture decisions and their operational consequences | [DECISIONS.md](DECISIONS.md) |
| MAINTAINING_DOCS.md | Sources of truth, capability labels, and documentation validation workflow | [MAINTAINING_DOCS.md](MAINTAINING_DOCS.md) |
| PANTALLAS.md | Interfaces (headless: CLI + HTTP API) | [frontend/PANTALLAS.md](frontend/PANTALLAS.md) |
| COMPONENTS.md | Reusable units (built-in tools, SDKs) | [frontend/COMPONENTS.md](frontend/COMPONENTS.md) |
| DESIGN-GUIDE.md | CLI terminal UX conventions | [frontend/DESIGN-GUIDE.md](frontend/DESIGN-GUIDE.md) |
| TESTS.md | Test strategy and scenarios | [TESTS.md](TESTS.md) |
| GAPS.md | Feature gap tracking (historical) | [GAPS.md](GAPS.md) |
| ROADMAP-PARITY.md | Competitive parity analysis | [ROADMAP-PARITY.md](ROADMAP-PARITY.md) |

## 4. Key Decisions

The maintained decision record is [DECISIONS.md](DECISIONS.md). The table below
is a compact historical summary.

| Date | Decision | Rationale |
|---|---|---|
| 2026-05-27 | Rust rewrite (from Python) | Single binary, portable, 10x performance |
| 2026-05-27 | YAML-only agent specs | Readability over flexibility (like Azure DevOps pipelines) |
| 2026-05-27 | Two-layer LLM (Adapter + Resource) | Separate provider HTTP details from domain interface |
| 2026-05-27 | Hexagonal architecture (adapters/) | Clean ports/adapters separation |
| 2026-05-27 | Zero silent failures | Every error propagated or logged at error level |

## 5. Historical PRDs

These rows record the original planning state and counts at the time; they are
not the current capability or test-status source of truth. Use
[GAPS.md](GAPS.md) and the subsystem documents above for current behavior.

| PRD | Status | Description |
|---|---|---|
| PRD-001 | done (v0.3.0) | Base engine |
| PRD-004 | done (v0.3.0) | Agent Contract — Input/Output Schema |
| PRD-006 | done (v0.4.x) | Apple-Grade Refinement — 20 sections |
| PRD-002 | backlog | — |
| PRD-003 | backlog | — |
| PRD-005 | backlog | Complete real integrations |
