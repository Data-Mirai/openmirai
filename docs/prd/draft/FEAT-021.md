# FEAT-021 — Agent Management Maturity: System Prompt, Agent Types, Versioning, Templates

**Estado**: Draft
**Fecha**: 2026-05-13
**Epic**: EPIC-024

---

## Problem Statement

**Tipo**: Feature nueva (5 capacidades de gestión de agentes)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Análisis competitivo vs Anthropic Console (Managed Agents) revela 5 gaps que impiden que Data Mirai sea competitivo como plataforma de agentes autónomos:

1. **System prompt fragmentado**: El identity prompt del agente vive en `config.context_compiler.identity_prompt` pero NO se inyecta a los nodos LLM durante ejecución. El agente no tiene "personalidad" unificada — cada nodo `ai/llm_call` opera sin contexto de identidad. Para cambiar el comportamiento del agente hay que editar cada nodo individualmente.

2. **Sin distinción managed vs live**: Todos los agentes usan el mismo modelo de ejecución (sessions por trigger). No existe concepto de agente que corre en loop continuo 24/7 (live agent). El live trader del usuario necesita un modelo start/stop con ciclos repetitivos, no sessions discretas.

3. **Versionado sin rollback funcional**: La tabla `agent_snapshots` existe y los endpoints publish/rollback están en el API client, pero falta validar que el flujo completo funcione end-to-end. UI tiene tab "Versions" pero puede no estar conectada.

4. **Templates sin backend**: El API client tiene `listTemplates()` apuntando a `GET /api/agents/templates`, pero el endpoint puede no devolver templates útiles. No hay seed data de templates predefinidos.

5. **Skills no existen**: Anthropic tiene skills (xlsx, pdf, etc.) como add-ons reutilizables. Data Mirai no tiene concepto de paquetes de capacidades compartidos entre agentes. Playbook rules son comportamiento, no capacidades.

---

## Objetivo

Cuando esto esté implementado:
1. El agente tiene un `system_prompt` top-level que se inyecta automáticamente a TODOS los nodos `ai/llm_call` del grafo — un solo punto de control para la identidad del agente
2. Existen dos tipos: `managed` (sessions por trigger, modelo actual) y `live` (start/stop con runs y cycles continuos)
3. Versionado publish/rollback funciona end-to-end: UI → API → DB → restauración
4. 3-5 templates predefinidos disponibles en creación de agentes
5. (Futuro) Skills como paquetes reutilizables — se documenta diseño pero no se implementa en este FEAT

---

## Features

### 21.1 — System Prompt Top-Level

**Problema**: `config.context_compiler.identity_prompt` existe en UI pero el runner no lo extrae ni lo pasa a `ai/llm_call`. El LLM opera sin identidad del agente.

**Solución**: Agregar `system_prompt` como campo top-level en AgentSpec y tabla `agents`. El runner lo extrae y lo inyecta como system prompt en cada nodo `ai/llm_call` del grafo.

**Cambios**:

1. `AgentSpec` (framework): campo `system_prompt: str = ""`
2. Tabla `agents` (DB): columna `system_prompt TEXT DEFAULT ''`
3. `AgentRepo`: CRUD incluye system_prompt
4. `GraphRunner.run()`: recibe system_prompt, lo pasa al executor
5. `llm_call` executor: si recibe system_prompt del agente, lo prepende al prompt (o lo usa como system message si el adapter lo soporta)
6. API: endpoints create/update/get agent incluyen system_prompt
7. UI: campo textarea en Config tab (reemplaza identity_prompt por system_prompt)

**Regla**: Si un nodo `ai/llm_call` tiene su propio system_prompt en config, se concatena: `agent.system_prompt + "\n\n" + node.config.system_prompt`. El del agente va primero (identidad), el del nodo después (instrucciones específicas).

### 21.2 — Agent Types: Managed vs Live

**Problema**: Solo existe enabled/disabled. No hay modelo de ejecución para agentes que corren 24/7 en loop.

**Solución**: Campo `agent_type` con dos valores: `managed` (default, comportamiento actual) y `live` (nuevo).

**Modelo Managed** (sin cambios):
- Triggers → Sessions (pending → running → completed/failed)
- Cada session es una ejecución discreta del grafo

**Modelo Live** (nuevo):
- Start/Stop → Runs (started → running → stopped)
- Dentro de cada Run: Cycles (cada iteración del grafo)
- El grafo se ejecuta en loop hasta que el usuario lo detiene

**Nuevas tablas**:

```sql
CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'started',  -- started, running, stopping, stopped, failed
    started_at TEXT NOT NULL,
    stopped_at TEXT,
    duration_ms INTEGER,
    total_cycles INTEGER DEFAULT 0,
    stop_reason TEXT,  -- manual, error, max_cycles
    config TEXT DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS agent_cycles (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    cycle_number INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',  -- running, completed, failed, skipped
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER,
    result TEXT DEFAULT '{}',
    error_message TEXT
);
```

**Nuevos endpoints**:
- `POST /api/agents/{id}/start` → inicia Run para live agent
- `POST /api/agents/{id}/stop` → detiene Run activo
- `GET /api/agents/{id}/runs` → lista runs
- `GET /api/runs/{id}/cycles` → lista cycles de un run

**Config Live Agent**:
```json
{
  "live": {
    "cycle_interval_seconds": 300,
    "max_cycles": null,
    "retry_on_error": true,
    "cooldown_on_error_seconds": 60
  }
}
```

**UI diferenciada**:
- Managed: botón "Execute" + tab Sessions
- Live: botón "Start"/"Stop" + tab Runs (con cycles dentro)

### 21.3 — Agent Versioning (Validación + UI)

**Problema**: Infraestructura existe (`agent_snapshots` table, endpoints) pero puede no funcionar end-to-end.

**Solución**: Validar flujo completo, arreglar si hay bugs, asegurar que UI muestra versiones y permite rollback.

**Checklist**:
1. `POST /api/agents/{id}/publish` → crea snapshot con graph_def + config + version auto-increment
2. `GET /api/agents/{id}/versions` → lista snapshots ordenados por version DESC
3. `POST /api/agents/{id}/rollback` → restaura graph_def y config desde snapshot
4. UI tab "Versions" muestra lista con botón rollback
5. E2E test: publish → edit → rollback → verificar que volvió al estado anterior

### 21.4 — Agent Templates

**Problema**: `GET /api/agents/templates` existe pero puede no devolver templates útiles.

**Solución**: 5 templates hardcoded en backend, cada uno con graph_def + system_prompt + config preconfigurado.

**Templates**:

| ID | Nombre | Descripción | Nodos |
|---|---|---|---|
| `web-researcher` | Web Researcher | Busca info en web, analiza con LLM, genera reporte | trigger/manual → data/web_scrape → ai/llm_call → output/response |
| `data-monitor` | Data Monitor | Monitorea fuente de datos periodicamente | trigger/schedule → data/web_scrape → ai/llm_call → data/db_write → output/response |
| `document-processor` | Document Processor | Procesa documentos con LLM | trigger/webhook → ai/llm_call → data/db_write → output/response |
| `news-digest` | News Digest | Agrega noticias semanales | trigger/schedule → data/web_scrape → ai/llm_call → output/response |
| `live-trader` | Live Trader | Monitoreo continuo + análisis + señales | trigger/manual → data/web_scrape → ai/llm_call → data/db_write → output/response |

Cada template incluye: `id`, `name`, `description`, `agent_type` (managed/live), `system_prompt`, `graph_def` (nodes + edges con data_map), `config`.

**Endpoint**: `GET /api/agents/templates` devuelve lista. `POST /api/environments/{envId}/agents` acepta `template_id` opcional que pre-popula el agente.

### 21.5 — Skills (Diseño futuro — NO se implementa)

Documentar diseño para implementación futura:
- Skills = paquetes de instrucciones + tools que se adjuntan a agentes
- Almacenadas en tabla `skills` (id, name, instructions, tools, created_at)
- Agente referencia skills por ID en su config
- Al ejecutar, las instrucciones de skills se inyectan al system prompt

No se implementa en este FEAT. Se prioriza en roadmap futuro.

---

## Cambios por archivo

### Framework (`framework/`)

| Archivo | Cambio |
|---|---|
| `core/agent_spec.py` | Campo `system_prompt: str`, campo `agent_type: str` |
| `core/runner.py` | Pasar system_prompt a executor, soportar loop mode para live agents |
| `tools/builtin/ai/llm_call.py` | Recibir system_prompt del contexto, inyectar en LLM call |
| `core/context.py` | Agregar `system_prompt` y `agent_config` a ExecutionContext |

### App Server (`app/server/`)

| Archivo | Cambio |
|---|---|
| `database.py` | Migration v9: system_prompt + agent_type en agents, tablas agent_runs + agent_cycles. Repos nuevos |
| `app.py` | Endpoints start/stop, runs/cycles. system_prompt en agent CRUD. Templates con seed data |

### App Web (`app/web/`)

| Archivo | Cambio |
|---|---|
| `lib/api.ts` | Tipos actualizados, métodos start/stop/runs/cycles |
| `app/agents/[id]/page.tsx` | system_prompt en Config, UI condicional managed/live, tab Runs |
| Componente AgentDesigner | Template picker en creación |

### Tests

| Archivo | Cambio |
|---|---|
| `app/e2e/tests/agent-management.spec.ts` | E2E: system prompt, agent types, templates, versioning |

---

## Dependencias

- FEAT-019 (data pipeline) para db_write en templates
- FEAT-008 (streaming) para live canvas en live agents

## Fuera de scope

- Skills (21.5) — solo diseño, no implementación
- Cloud environments / sandboxing — es Capa 3
- MCP skills marketplace — futuro
