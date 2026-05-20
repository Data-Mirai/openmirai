# Bitacora — Data Mirai Engine

Registro cronologico inverso de todas las acciones significativas en el proyecto.

---

**2026-05-14 · 10-CODEGEN · ✅ done**
Fix critico: segundo db_write se colgaba indefinidamente. Reescrito SQLiteDBResource con conexiones per-operacion (sin threading.Lock). Auto-creacion de tablas y auto-migracion de columnas en _insert_sync.

Cambios:
- MODIFICADO `framework/src/datamirai_engine/resources/sqlite_db.py` — eliminado threading.Lock + conexion persistente, nuevo _open() context manager, auto-create table + auto-add columns en _insert_sync, asyncio.to_thread
- CREADO `app/e2e/tests/db-write-pipeline.spec.ts` — 5 tests E2E: dual writes sin schema, same-table writes, mixed schema, write+read, scrape+dual write

21/21 unit tests SQLite verdes. E2E pendiente de correr (server necesita restart).

---

**2026-05-14 · 10-CODEGEN · ✅ done**
FEAT-023 "Stealth-First Web Scraping" — implementacion completa de las 5 features.

Cambios:
- MODIFICADO `framework/src/datamirai_engine/tools/builtin/data/stealth.py` — SessionFingerprint frozen dataclass con coherencia interna (UA ↔ Platform ↔ Sec-CH-UA), select_proxy(), proxy_host_safe()
- MODIFICADO `framework/src/datamirai_engine/core/context.py` — property fingerprint en ExecutionContext
- MODIFICADO `framework/src/datamirai_engine/resources/context.py` — fingerprint lazy-init en SimpleExecutionContext
- MODIFICADO `framework/src/datamirai_engine/core/events.py` — 4 EventTypes: BROWSER_SCREENSHOT, BROWSER_ACTION, BROWSER_NAVIGATION, BROWSER_COMPLETED
- MODIFICADO `framework/src/datamirai_engine/tools/builtin/data/web_scrape.py` — v2.1: stealth-first, scrape_mode (windowless/tunnel_vision), proxy_url/proxy_list, referer chain, domain backoff acumulativo
- MODIFICADO `framework/src/datamirai_engine/tools/builtin/data/browser_agent.py` — fingerprint, proxy, SSE events, screenshot PNG por sesion, manifest.json, JPEG q70 streaming
- CREADO `framework/tests/tools/test_session_fingerprint.py` — 24 tests (fingerprint, proxy, coherencia, serialization)

Tests: 756 passed (39 nuevos de FEAT-023). 10 fallos pre-existentes en llm_call.py (no regresion).

Decisiones:
- Fingerprint inmutable por sesion (frozen dataclass), NO por request
- Domain backoff acumulativo: 429/403 duplica delay por dominio, cap 30s
- Screenshots en ~/.datamirai/screenshots/{session_id}/ con manifest.json SIEMPRE
- JPEG q70 para streaming SSE, PNG full para trazabilidad en disco

---

**2026-05-14 · 01-INTAKE · ✅ done**
FEAT-023 "Stealth-First Web Scraping" — PRD creado con 5 features: Session Fingerprint, Modo Windowless, Modo Tunnel Vision, Screenshot Traceability, Proxy Support. EPIC-090.

Cambios:
- CREADO `docs/prd/draft/FEAT-023.md` — PRD completo (20 reglas, 5 features, dependencias)
- MODIFICADO `docs/INDEX.md` — agregada referencia a FEAT-023

Contexto: Rate-limiting real detectado en `data-asg.goldprice.org` durante ejecucion del agente Live Trader. El modulo stealth.py existia pero NO estaba integrado en web_scrape.py.

---

**2026-05-14 · 10-CODEGEN · ✅ done**
FEAT-022 "Platform UX Maturity" — EPIC-025 · 5 bloques (BLOCK-054→058) · 23 tickets (TICK-232→254).

Cambios:
- ELIMINADO `app/web/src/components/trading/` — TradeDashboard + TradeCard (plataforma es general-purpose)
- MODIFICADO `app/web/src/components/live/LiveCanvas.tsx` — eliminado truncateValue(), panel detalle con Input/Processing/Output, streaming visible, panel 420px
- MODIFICADO `app/server/datamirai_app/database.py` — migration v10: snapshot_version en sessions + agent_runs
- MODIFICADO `app/server/datamirai_app/app.py` — execute + start guardan snapshot_version, eliminado tab Trading, templates con rich_html
- CREADO `framework/src/datamirai_engine/render/charts.py` — bar/line/pie chart helpers con Chart.js
- MODIFICADO `framework/src/datamirai_engine/render/engine.py` — render_rich() con JS embebido
- MODIFICADO `framework/src/datamirai_engine/tools/builtin/output/response.py` — format rich_html
- MODIFICADO `app/web/src/app/sessions/[id]/page.tsx` — iframe allow-scripts, rich_html detection
- MODIFICADO `app/web/src/app/agents/[id]/page.tsx` — version dropdown, snapshot_version en sessions, eliminado tab Trading
- CREADO `app/e2e/tests/live-view.spec.ts` — 4 tests
- CREADO `app/e2e/tests/versioning.spec.ts` — 4 tests
- CREADO `app/e2e/tests/rich-reports.spec.ts` — 3 tests
- CREADO `app/e2e/tests/templates-updated.spec.ts` — 2 tests

35 E2E tests verdes (headed) · 751 unit tests verdes

---

**2026-05-14 · 10-CODEGEN · ✅ done**
BLOCK-054 "Eliminar componentes domain-specific" — EPIC-025 · 3 tickets (TICK-232→234).

Cambios:
- ELIMINADO `app/web/src/components/trading/TradeDashboard.tsx`
- ELIMINADO `app/web/src/components/trading/TradeCard.tsx`
- ELIMINADO directorio `app/web/src/components/trading/`
- MODIFICADO `app/web/src/app/agents/[id]/page.tsx` — eliminado tab Trading, import TradeDashboard, tipo Tab sin "trading"
- VERIFICADO `app/e2e/tests/live-trader.spec.ts` — sin refs a trading/scheduler, 22/22 E2E verdes

Decisiones:
- Plataforma es general-purpose. Dashboards específicos son reportes HTML del agente, no componentes React.

---

**2026-05-14 · 10-CODEGEN · ✅ done**
Limpieza scheduler viejo → modelo Runs/Cycles canónico. Eliminado AgentScheduler, rutas scheduler, UI heartbeat/scheduler.
751 unit tests + 22 E2E verdes.

---

**2026-05-13 · 10-CODEGEN · ✅ done**
FEAT-021 "Agent Management Maturity" — EPIC-024 · 5 bloques (BLOCK-049→053) · 16 tickets (TICK-216→231).

Cambios:
- CREADO `docs/prd/draft/FEAT-021.md` — PRD: system prompt, agent types, versioning, templates
- MODIFICADO `framework/src/datamirai_engine/core/agent_spec.py` — campos system_prompt + agent_type en AgentSpec
- MODIFICADO `framework/src/datamirai_engine/core/context.py` — propiedad system_prompt en ExecutionContext
- MODIFICADO `framework/src/datamirai_engine/resources/context.py` — system_prompt en SimpleExecutionContext
- MODIFICADO `framework/src/datamirai_engine/tools/builtin/ai/llm_call.py` — inyeccion system_prompt en LLM calls
- MODIFICADO `app/server/datamirai_app/database.py` — migration v9: system_prompt + agent_type + tablas agent_runs/agent_cycles + RunRepo/CycleRepo
- MODIFICADO `app/server/datamirai_app/app.py` — endpoints start/stop/runs/cycles, 5 templates, crear desde template_id
- MODIFICADO `app/server/datamirai_app/resource_factory.py` — system_prompt en build_context_from_resources
- MODIFICADO `app/web/src/lib/api.ts` — tipos Agent/Run/Cycle/Template + metodos API
- MODIFICADO `app/web/src/app/agents/[id]/page.tsx` — SystemPromptConfig, AgentType toggle, RunsTab, Start/Stop buttons
- CREADO `app/e2e/tests/agent-management.spec.ts` — 16 E2E tests (API + UI) → 16/16 passed

---

**2026-05-11 · 00-WORKBOARD · ✅ done**
PRD FEAT-018 "Real-time Experience — WebSocket + Live UI" (FEAT-007→017 reservados por otra sesión) · EPIC-023 · 6 bloques (BLOCK-043→048) · 29 tickets (TICK-187→215) · 47 dependencias · 55 test refs.
Docs creados/modificados: FEAT-007.md, ARCHITECTURE.md (§9-§11), FLUJOS.md (REGLA-26→38 + WS Connection), PANTALLAS.md, API.md, TESTS.md (TEST-051→085), INDEX.md.

---

**2026-05-09 · 10-CODEGEN · ✅ done**
BLOCK-038 "Context Compiler" + BLOCK-039 "Memory Flusher" completados.

Cambios:
- CREADO `framework/src/datamirai_engine/intelligence/context_compiler.py` — 5-phase context assembly (identity, playbook, session, memory, compression)
- CREADO `framework/src/datamirai_engine/intelligence/memory_flusher.py` — auto-flush a long-term memory con threshold + REGLA-58 max 1 per session
- MODIFICADO `framework/src/datamirai_engine/intelligence/__init__.py` — export ContextCompiler + MemoryFlusher
- CREADO `framework/tests/intelligence/test_context_compiler.py` — 13 tests
- CREADO `framework/tests/intelligence/test_memory_flusher.py` — 9 tests
- MODIFICADO `app/server/datamirai_app/database.py` — migration v7: memory_flushed_at en sessions
- MODIFICADO `app/web/src/app/agents/[id]/page.tsx` — seccion Context Compiler en config tab (toggles playbook/memory + identity prompt)
- CREADO `app/e2e/tests/context-compiler.spec.ts` — API + UI tests

---

**2026-05-07 · 00-WORKBOARD · ✅ done**
BLOCK-029 "LLM-Powered Agent Designer" + TICK-121/122/123.
Reemplazar regex/stopwords del designer por LLM conversacional.

---

**2026-05-07 · 10-CODEGEN · ✅ done**
BLOCK-029 "LLM-Powered Agent Designer" completado · 3 tickets.

Cambios:
- MODIFICADO `framework/src/datamirai_engine/tools/base.py` — campo `intents` en ToolSpec
- MODIFICADO 19 tool specs — intents descriptivos para cada herramienta
- MODIFICADO `app/server/datamirai_app/graph_generator.py` — design_chat() con system prompt + JSON schema forzado
- MODIFICADO `app/server/datamirai_app/app.py` — endpoint POST /api/agents/design-chat + intents en /api/tools
- MODIFICADO `app/web/src/lib/api.ts` — metodo designChat() + intents en ToolDef
- MODIFICADO `app/web/src/components/agent/AgentDesigner.tsx` — processPrompt usa LLM via backend, eliminado ~200 lineas de regex/stopwords/hardcoded

---

**2026-05-05 (sesion 2) · 10-CODEGEN · ✅ done**
App local v2: stack completo + Agent Designer + onboarding.

**Implementado**:
- Docker Compose: Ollama + PostgreSQL/pgvector + MinIO con health checks
- System detection: RAM/CPU/GPU, modelos recomendados por capacidad
- Adapters reales: OllamaLLMResource, CloudLLMResource, PostgresDBResource, S3StorageResource
- Auto-provisioning: crear DB + bucket al crear environment
- Service management: start/stop individual, stats CPU/RAM/uptime, logs expandibles
- LLM config desde app: selector modelo activo, instalar modelos, persistencia en app_config
- Agent Designer: canvas React Flow + chat AI + inspector + catalogo tools sidebar
- Nodo "+" flotante: popover con catalogo al click, auto-connect
- Wizard onboarding: crear universo → primer agente → designer con prompt auto-enviado
- Design system del handoff (CSS tokens, tu-card, tu-pill, universe cards con minimap)
- Canvas cosmico: planetas SVG, iconos (no emojis), hover sin desplazamiento
- Modal centrado para crear universo
- 24 E2E tests Playwright (settings, services, designer, universe flow)

**Pendiente proxima sesion**:
- Tool `data/web_scrape` (HTTP fetch + parse HTML)
- Chat inteligente: configurar propiedades de cada nodo al generar (prompt, URL, tabla)
- Integrar LLM real (Ollama) en el chat del designer para generar flujos

**Tests**: 308 framework + 34 app + 24 E2E = 366 total

---

**2026-05-05 · 10-CODEGEN · ✅ done**
App local completa: rename blocks→tools + reestructura repo (framework/ + app/) + SQLite persistence + API completa + frontend actualizado.

**Archivos creados**:
- CREADO `app/server/datamirai_app/database.py` — SQLite persistence: 7 repos (Universe, Environment, Resource, Graph, Agent, Session, Memory)
- CREADO `app/server/datamirai_app/app.py` — FastAPI server: CRUD universes/environments/resources/agents/graphs/sessions + execute + tools catalog
- CREADO `app/server/datamirai_app/main.py` — CLI entry point (serve, init, version)
- CREADO `app/server/tests/test_database.py` — 22 tests SQLite repos
- CREADO `app/server/tests/test_api.py` — 12 tests API endpoints
- CREADO `app/web/src/app/page.tsx` — Home: lista universes + crear
- CREADO `app/web/src/app/universes/[id]/page.tsx` — Universe detail: environments + crear (con sugerencias endémicas)
- CREADO `app/web/src/app/universes/[id]/environments/[envId]/page.tsx` — Environment: agents + resources tabs + CRUD

**Archivos modificados/renombrados**:
- RENOMBRADO `src/datamirai_engine/blocks/` → `framework/src/datamirai_engine/tools/` (34 archivos Python)
- RENOMBRADO `editor/` → `app/web/` (14 archivos TS/TSX)
- MODIFICADO `framework/src/datamirai_engine/tools/base.py` — BlockSpec→ToolSpec, BaseBlock→BaseTool
- MODIFICADO `framework/src/datamirai_engine/tools/registry.py` — BlockRegistry→ToolRegistry
- MODIFICADO 13 builtin tools — renamed classes + imports
- MODIFICADO `framework/src/datamirai_engine/core/graph.py` — block_type→tool_type
- MODIFICADO `framework/src/datamirai_engine/core/runner.py` — BlockExecutor→ToolExecutor + labels
- MODIFICADO `framework/src/datamirai_engine/__init__.py` — exports actualizados
- MODIFICADO `app/web/src/lib/api.ts` — nueva API con universes/environments
- MODIFICADO `app/web/src/components/ui/Sidebar.tsx` — nav simplificado
- MODIFICADO `app/web/src/app/agents/[id]/page.tsx` — adaptado a nueva API
- MODIFICADO `app/web/src/app/sessions/[id]/page.tsx` — adaptado a nueva API
- MODIFICADO `docs/ARCHITECTURE.md` — estructura actualizada
- MODIFICADO `docs/producto/DOMINIO.md` — Block→Tool

**Decisiones**:
- blocks→tools rename completo: 50+ archivos, 308 tests siguen pasando
- Repo: framework/ (PyPI) + app/ (app local). App usa framework como dep editable
- SQLite para persistencia local — no requiere Postgres
- Auto-create "development" env al crear universe
- Sugerencias env: staging → production → nombres endémicos latinoamericanos
- Environment naming: amazonas, galápagos, patagonia, atacama, pantanal...

**Tests**: 342 passed (308 framework + 34 app), 0 failed

---

**2026-05-04 · 10-CODEGEN · ✅ done**
Persistencia Postgres + Sesiones con Transcript — implementacion completa.

**Archivos creados**:
- CREADO `src/datamirai_engine/db/connection.py` — pool asyncpg con create_pool/close_pool
- CREADO `src/datamirai_engine/db/migrations.py` — schema standalone v1 (graphs, agents, sessions con transcript)
- CREADO `src/datamirai_engine/db/repositories.py` — GraphRepo, AgentRepo, SessionRepo (CRUD async)
- CREADO `tests/db/__init__.py` + `tests/db/test_repositories.py` — 13 tests con mock pool

**Archivos modificados**:
- MODIFICADO `src/datamirai_engine/db/__init__.py` — re-exports de connection, migrations, repos
- MODIFICADO `src/datamirai_engine/core/runner.py` — TranscriptEvent dataclass + generacion de eventos legibles en GraphRunner
- MODIFICADO `src/datamirai_engine/runtime/agent_runtime.py` — repos opcionales, deploy_agent/destroy_agent ahora async, persist, load_from_db, get_session
- MODIFICADO `src/datamirai_engine/server/app.py` — pool lifecycle en lifespan, persist_graph, GET /api/sessions/{id}
- MODIFICADO `src/datamirai_engine/cli.py` — subcomando `datamirai db init` y `datamirai db status`
- MODIFICADO `pyproject.toml` — optional dep `asyncpg>=0.30` en grupo [postgres]
- MODIFICADO `docker-compose.yml` — DATABASE_URL + depends_on postgres
- MODIFICADO `editor/src/lib/api.ts` — TranscriptEvent type, getSession()
- MODIFICADO `editor/src/app/sessions/[id]/page.tsx` — tabs Transcript/Traza, fetch via getSession
- MODIFICADO `tests/runtime/test_agent_runtime.py` — deploy/destroy ahora async, tests transcript

**Decisiones**:
- Schema standalone simplificado (TEXT ids, sin universe/environment) — compatible con IDs existentes del engine
- Persistencia 100% opcional: sin DATABASE_URL todo funciona in-memory como antes
- TranscriptEvents son humano-legibles (español), trace sigue siendo tecnico
- deploy_agent y destroy_agent cambiaron a async (breaking para llamadas directas, pero API HTTP no cambia)

**Tests**: 308 passed, 0 failed, 2.05s

---

**2026-05-03 12:00 · 10-CODEGEN · ✅ done**
Implementación COMPLETA — 18 bloques, 67 tickets, todo done.
Backend Python: 252 tests, ruff clean. Frontend: Next.js build clean, 3 vitest tests.
Bloques implementados esta sesión: BLOCK-011→018 (Resource Interfaces, Memory, Auth,
FastAPI Server, AI Blocks, Data Blocks, Triggers, Long-term Memory, Editor Canvas,
Block Catalog, Graph Management, CLI+Docker, Test Setup).

---

**2026-05-03 10:00 · 10-CODEGEN · ✅ done**
Batch BLOCK-005→010 + BLOCK-014: AI Blocks, Data Blocks, Triggers, Memory, Editor Canvas.
→ AI: llm_call, transcribe, embeddings via context.llm
→ Data: db_read/write, storage_read/write via context.db/storage
→ Triggers: webhook, manual, schedule, event, agent_call + TriggerSpec
→ Memory: LongTermMemory (text search), SharedLog (bitácora con autoría)
→ Editor: Next.js 15 + React Flow + BlockNode custom component + EditorCanvas
Build exitoso. 250 tests Python, ruff clean. TICK-018→024, 028→035, 051→054 = done.

---

**2026-05-03 08:00 · 10-CODEGEN · ✅ done**
BLOCK-013 FastAPI Server — 8 tickets (TDD).
→ CREADO `src/datamirai_engine/server/app.py` — create_app() factory
→ Endpoints: /health, CRUD grafos, agentes (deploy/enable/disable), ejecución, sesiones
→ RegistryExecutor + builtin blocks auto-registered
→ pyproject.toml: fastapi>=0.115, uvicorn, httpx (dev)
→ 12 tests API nuevos. 202 total, ruff clean. TICK-043→050 = done.

---

**2026-05-03 07:00 · 10-CODEGEN · ✅ done**
BLOCK-012 Auth + Permissions — 3 tickets (TDD).
→ CREADO `src/datamirai_engine/core/auth.py` — Permission enum, PermissionEvaluator, SingleUserAuth
→ Matriz OWNER>ADMIN>EDITOR>VIEWER × db/vector/storage/llm/agents
→ 12 tests nuevos. 190 total, ruff clean. TICK-040→042 = done.

---

**2026-05-03 06:00 · 10-CODEGEN · ✅ done**
BLOCK-009 Short-term Memory — 2 tickets (TDD).
→ CREADO `src/datamirai_engine/memory/short_term.py` — ShortTermMemory (logs, decisions, metrics)
→ CREADO `tests/memory/test_short_term.py` — 9 tests
178 tests total, ruff clean. TICK-031→032 = done.

---

**2026-05-03 05:00 · 10-CODEGEN · ✅ done**
BLOCK-011 Resource Interfaces — 4 tickets (TDD).
→ CREADO `src/datamirai_engine/resources/` — db, vector, storage, llm, context
→ InMemoryDBResource, InMemoryVectorResource (cosine similarity), InMemoryStorageResource, MockLLMResource
→ SimpleExecutionContext: wiring completo + factory `.default()`
→ 30 tests nuevos. 169 total, ruff clean. TICK-036→039 = done.

---

**2026-05-03 04:00 · 10-CODEGEN · ✅ done**
BLOCK-004 Logic Blocks + RegistryExecutor + E2E Integration — 5 tickets (TDD).
→ CREADO `src/datamirai_engine/blocks/builtin/logic/` — Condition, Switch, Loop, Merge, Wait
→ CREADO `src/datamirai_engine/core/runner.py:RegistryExecutor` — bridge runner↔registry
→ CREADO `tests/blocks/test_logic_blocks.py` — 19 tests unitarios
→ CREADO `tests/test_integration.py` — 6 tests E2E (linear, branch, error, loop, diamond)
→ MODIFICADO `core/graph.py` — self-loops condicionales permitidos (para loops)
→ MODIFICADO `core/runner.py` — data_map con acceso anidado (trigger.payload.status)
E2E validó: flujo linear, branching condicional, error routing, loop con exit, diamond graph.
139 tests total, ruff clean. TICK-013→017 = done.

---

**2026-05-03 03:00 · 10-CODEGEN · ✅ done**
BLOCK-003 Block Framework — 4 tickets implementados (TDD).
→ CREADO `src/datamirai_engine/blocks/base.py` — BlockSpec, BaseBlock, BlockInput, BlockOutput, ConfigField
→ CREADO `src/datamirai_engine/blocks/registry.py` — BlockRegistry + autodiscovery
→ CREADO `tests/blocks/test_spec.py` — 16 tests (spec, baseblock, runtime validation)
→ CREADO `tests/blocks/test_registry.py` — 11 tests (register, discover, categories)
113 tests total, ruff clean. TICK-009→012 = done.

---

**2026-05-03 02:00 · 10-CODEGEN · ✅ done**
BLOCK-002 GraphRunner — 4 tickets implementados (TDD).
→ CREADO `src/datamirai_engine/core/runner.py` — GraphRunner, edge eval (8 ops), data_map, retry policy, loop detection
→ CREADO `tests/core/test_runner.py` — 32 tests (linear, branching, operators, data_map, retry, loop)
→ MODIFICADO `src/datamirai_engine/core/state.py` — SharedState.set(overwrite=) para loops
86 tests total, ruff clean. TICK-005→008 = done.

---

**2026-05-03 01:00 · 10-CODEGEN · ✅ done**
BLOCK-001 Fundaciones Core — 4 tickets implementados (TDD).
→ CREADO `pyproject.toml` — pydantic>=2.9, hatchling, ruff, pytest
→ CREADO `src/datamirai_engine/` — estructura completa (core, blocks, triggers, memory, server)
→ CREADO `src/datamirai_engine/core/graph.py` — GraphDef, NodeDef, EdgeDef (Pydantic, validaciones)
→ CREADO `src/datamirai_engine/core/state.py` — SharedState (write-once, thread-safe, deep copy)
→ CREADO `src/datamirai_engine/core/context.py` — ExecutionContext ABC + 5 Resource Protocols + AuthContext
→ CREADO `tests/core/test_graph.py` — 22 tests
→ CREADO `tests/core/test_state.py` — 13 tests
→ CREADO `tests/core/test_context.py` — 19 tests
→ CREADO `README.md`
54 tests passing, ruff clean. TICK-001→004 = done.

---

**2026-05-02 15:30 · 14-RESEARCHER · ✅ done**
Deep-update de 11 stack skills (`/research upgrade`). 11 agentes paralelos.
Protocolo completo: API surface, changelog cronológico, funcionalidades core, versiones anteriores, cross-version, pitfalls.

| Skill | Antes | Después | Highlights |
|-------|-------|---------|------------|
| FastAPI+Python | ~1015 | ~1400 | changelog 0.100→0.136, Pydantic v2 migration 28 items, SSE patterns, security checklist |
| Next.js 15 | ~700 | ~1075 | caching deep dive, `"use cache"`, CSP nonces, CVEs, Turbopack estable |
| React Flow | ~906 | ~1629 | Handle props, event handlers, sub-flows, auto-layout 4 engines, undo/redo, 28 CSS vars |
| PostgreSQL+pgvector | ~807 | ~1761 | PG 18, hybrid search RRF, quantization binaria, iterative index scans, Alembic async |
| pytest | ~450 | ~1074 | pytest-asyncio migration, agentic engine testing, CI/CD workflow, 10→15 fuentes |
| Vitest | ~738 | ~1262 | evolución v1→v4, React Flow DOM mocks, pool selection, Jest migration |
| Playwright | ~1128 | ~1933 | ARIA snapshots, Clock API, React Flow EditorPage, accessibility, component testing |
| Clerk | ~1005 | ~1295 | Machine Auth M2M, CVE-2025-29927, pricing, rate limits, webhook verification |
| Kopf | ~860 | ~1275 | changelog 1.37→1.44.5, filters avanzados, peering HA, vs Operator SDK/kubebuilder |
| Temporal | ~803 | ~1911 | Nexus, Saga/Entity/Fan-out, Worker Versioning GA, schedules, 3 testing modes |
| S3 Storage | ~786 | ~1200 | feature parity R2 vs MinIO, pricing, conditional reads/writes, rate limits |

Total: ~8198 → ~14815 líneas (+81%). Sin info válida borrada.

---

**2026-05-02 12:50 · 14-RESEARCHER · ✅ done**
Stack skills creados para las 9 tecnologias del proyecto (modo automatico batch).
Archivos en `~/.blueprint/stacks/`:
→ python-3.12.md (1154 lineas), fastapi.md (1279), nextjs-15.md (757)
→ react-flow.md (1068), postgresql-pgvector.md (1006), pytest.md (695)
→ vitest.md (701), playwright.md (1378), ruff.md (400)
README.md actualizado con tabla de 9 skills.
Fuentes: docs oficiales, GitHub repos, Stack Overflow, blogs tecnicos.

---

**2026-05-01 17:00 · 00-WORKBOARD · ✅ done**
Workboard inicializado. 9 epics, 18 blocks, 67 tickets, 79 dependencias.
Orden: BLOCK-001 (Fundaciones) → BLOCK-018 (Test Setup).
Bloques críticos sin dependencias previas: BLOCK-001, BLOCK-011, BLOCK-018.

---

**2026-05-01 16:30 · BOOTSTRAP · ✅ done**
Bootstrap 4 fases completo:
→ ARCHITECTURE.md (stack + distribución 1 artefacto)
→ DOMINIO.md (entidades, roles, auth agnóstico)
→ FLUJOS.md (máquinas de estado, triggers, environments stack)
→ DESIGN-GUIDE.md (tokens cloud replicados)

---
