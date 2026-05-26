# Data Mirai Engine — Índice de Documentación

## Overview

Motor open source de ejecución de grafos agentivos. Un solo artefacto Python que incluye runtime de agentes + editor visual. Alternativa a LangGraph y Google ADK sin vendor lock-in.

Un Universe = instancia de aplicación con environments, recursos (DB, vector, storage, LLM) y agentes. Auth delegado a provider externo (Clerk, Auth0, etc.) — engine recibe identidad verificada y evalúa permisos.

## PRD y Arquitectura
- [DATAMIRAI-ENGINE-PRD.md](prd/draft/DATAMIRAI-ENGINE-PRD.md) — PRD completo del engine
- [FEAT-001.md](prd/draft/FEAT-001.md) — MVP Features: Engine Production-Ready (9 features + session control)
- [FEAT-002.md](prd/draft/FEAT-002.md) — Tier 1: Fundamento (Multi-LLM + Persistencia + Búsqueda)
- [FEAT-003.md](prd/draft/FEAT-003.md) — Tier 2: Auto-mejora (Tracer + Reflector + Playbook + Graph Self-Improvement)
- [FEAT-004.md](prd/draft/FEAT-004.md) — Tier 3: Autonomía (Context Compiler + Memory Flush + Heartbeat + Resume + Sub-grafos)
- [FEAT-005.md](prd/draft/FEAT-005.md) — Tier 4: Inteligencia (User Modeling + Tool Scoring + Nudge + Curator)
- [FEAT-006.md](prd/draft/FEAT-006.md) — Tier 5: Escala (Parallel + Templates + OpenTelemetry + MCP Server)
- [FEAT-007.md](prd/draft/FEAT-007.md) — MCP Client + Server (200+ integraciones)
- [FEAT-008.md](prd/draft/FEAT-008.md) — Token-level Streaming (UX real-time)
- [FEAT-009.md](prd/draft/FEAT-009.md) — Guardrails & Safety (enterprise-ready)
- [FEAT-010.md](prd/draft/FEAT-010.md) — Sandbox Code Execution (código seguro)
- [FEAT-011.md](prd/draft/FEAT-011.md) — Multi-Canal (Telegram, Slack, WhatsApp, Discord, Email)
- [FEAT-012.md](prd/draft/FEAT-012.md) — Agent Testing & Evaluation
- [FEAT-013.md](prd/draft/FEAT-013.md) — YAML Declarative Agents
- [FEAT-014.md](prd/draft/FEAT-014.md) — Production Deploy (Docker + Helm + Health)
- [FEAT-015.md](prd/draft/FEAT-015.md) — ~~Cost Control & Budgets~~ (OBSOLETA — reemplazada por FEAT-027)
- [FEAT-016.md](prd/draft/FEAT-016.md) — Voice & Multimodal
- [FEAT-017.md](prd/draft/FEAT-017.md) — A2A Protocol (Agent-to-Agent)
- [FEAT-018.md](prd/draft/FEAT-018.md) — Real-time Experience: WebSocket + Live UI
- [FEAT-019.md](prd/draft/FEAT-019.md) — Data Pipeline Integrity: Persistencia, Fuentes, Provisioning, Diseño Iterativo
- [FEAT-020.md](prd/draft/FEAT-020.md) — Intelligent Data Pipeline: Web Scraping v2 + Knowledge Vault + Rich Output
- [FEAT-021.md](prd/draft/FEAT-021.md) — Agent Management Maturity: System Prompt, Agent Types, Versioning, Templates
- [FEAT-022.md](prd/draft/FEAT-022.md) — Platform UX Maturity: Live View, Versionado, Rich Reports, Limpieza
- [FEAT-023.md](prd/draft/FEAT-023.md) — Stealth-First Web Scraping: Anti-Deteccion, Modos Windowless/Tunnel Vision, Trazabilidad Visual
- [FEAT-027.md](prd/draft/FEAT-027.md) — Energy Metering + Execution Lifecycle (reemplaza FEAT-015, absorbe FEAT-022 §22.1)
- [FEAT-028.md](prd/draft/FEAT-028.md) — NVIDIA NIM Provider (100+ modelos subsidiados, OpenAI-compatible)
- [FEAT-030.md](prd/draft/FEAT-030.md) — Content Generation MCP Servers (Imagen, Video, Audio)
- [FEAT-031.md](prd/draft/FEAT-031.md) — Web Scraping a Escala: Apify + Scraping Infrastructure
- [FEAT-032.md](prd/draft/FEAT-032.md) — Engine Rust: Paridad Completa con Python (tools, memory, resources)
- [FEAT-033.md](prd/draft/FEAT-033.md) — Crates.io Quality Refactor: trait defaults, enums tipados, error chains, performance, docs
- [FEAT-034.md](prd/draft/FEAT-034.md) — API Simplification & DX Overhaul: enums tipados, convenience factories, @tool decorator, JSON canónico, 3 capas (agente/motor/host)
- [ARCHITECTURE.md](ARCHITECTURE.md) — Stack, distribución, convenciones, real-time (§9-§11)

## Producto
- [DOMINIO.md](producto/DOMINIO.md) — Entidades, roles, auth, glosario
- [FLUJOS.md](producto/FLUJOS.md) — Máquinas de estado, reglas (pendiente)

## Database
- [SCHEMA.md](database/SCHEMA.md) — Tablas, tipos (pendiente)

## Backend
- [API.md](backend/API.md) — WebSocket protocol, canales, payloads, deprecación SSE

## Frontend
- [PANTALLAS.md](frontend/PANTALLAS.md) — Componentes transversales, cambios por página, hook useReactive
- [DESIGN-GUIDE.md](frontend/DESIGN-GUIDE.md) — Tokens de color, tipografía, variantes de átomos

## Tests
- [TESTS.md](TESTS.md) — Escenarios GWT (50 tests para FEAT-001)

## Infra
- [INFRA.md](infra/INFRA.md) — Deploy, Helm chart (pendiente)
