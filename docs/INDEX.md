# Data Mirai Engine — Documentation Index

## 1. Overview

Open-source agent execution engine compiled in Rust. Runs agentic workflows defined as YAML graphs. One binary, any LLM, 48+ built-in tools, 7 LLM providers, 665 tests. Alternative to LangGraph, CrewAI, and Google ADK. Version: v0.4.3.

## 2. Stack

Rust engine + Axum HTTP server + SQLite + 7 LLM providers. Full details → [ARCHITECTURE.md](ARCHITECTURE.md)

## 3. Documentation Map

| Document | Description | Path |
|---|---|---|
| ARCHITECTURE.md | Stack, modules, conventions, API reference | [ARCHITECTURE.md](ARCHITECTURE.md) |
| DOMINIO.md | Domain glossary, roles, capabilities | [producto/DOMINIO.md](producto/DOMINIO.md) |
| FLUJOS.md | Execution flows, state machines, business rules | [producto/FLUJOS.md](producto/FLUJOS.md) |
| SCHEMA.md | SQLite schemas, migrations | [database/SCHEMA.md](database/SCHEMA.md) |
| API.md | HTTP API endpoints and contracts | [backend/API.md](backend/API.md) |
| PRIMITIVES.md | Reusable code patterns | [backend/PRIMITIVES.md](backend/PRIMITIVES.md) |
| TESTS.md | Test scenarios | [TESTS.md](TESTS.md) |
| GAPS.md | Feature gap tracking | [GAPS.md](GAPS.md) |
| ROADMAP-PARITY.md | Competitive parity analysis | [ROADMAP-PARITY.md](ROADMAP-PARITY.md) |

## 4. Key Decisions

| Date | Decision | Rationale |
|---|---|---|
| 2026-05-27 | Rust rewrite (from Python) | Single binary, portable, 10x performance |
| 2026-05-27 | YAML-only agent specs | Readability over flexibility (like Azure DevOps pipelines) |
| 2026-05-27 | Two-layer LLM (Adapter + Resource) | Separate provider HTTP details from domain interface |
| 2026-05-27 | Hexagonal architecture (adapters/) | Clean ports/adapters separation |
| 2026-05-27 | Zero silent failures | Every error propagated or logged at error level |

## 5. PRDs

| PRD | Status | Description |
|---|---|---|
| PRD-001 | done (v0.3.0) | Base engine — 23 features, 636 tests |
| PRD-004 | done (v0.3.0) | Agent Contract — Input/Output Schema |
| PRD-006 | done (v0.4.x) | Apple-Grade Refinement — 20 sections |
| PRD-002 | backlog | — |
| PRD-003 | backlog | — |
| PRD-005 | backlog | Complete real integrations |
