# FEAT-006 — Tier 5: Escala (Parallel + Templates + Observabilidad + MCP Server)

**Estado**: Draft
**Fecha**: 2026-05-09
**Epic**: EPIC-060

---

## Problem Statement

**Tipo**: Feature nueva (4 capacidades de escala)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine tiene core funcional con 16+ tools builtin, memoria 2 niveles, multi-LLM, session control, checkpoints, y vault. Pero presenta cuatro carencias que limitan la escalabilidad y la integracion con ecosistemas externos:

1. **Ejecucion secuencial rigida**: El GraphRunner ejecuta un nodo a la vez. Cuando hay nodos independientes (web_scrape a 3 fuentes, db_read + api_call sin dependencia), se ejecutan en serie perdiendo tiempo. No hay forma de expresar paralelismo en el grafo.

2. **Sin reutilizacion de grafos**: Cada agente se construye desde cero. No hay forma de guardar un grafo como template, compartirlo, ni importar templates de otros. El conocimiento de diseno se pierde entre proyectos.

3. **Observabilidad artesanal**: Hay execution_spans en SQLite pero no hay integracion con estandares de observabilidad. Sin OpenTelemetry no hay forma de correlacionar traces en herramientas externas (Grafana, Jaeger), ni de exportar metricas a sistemas de monitoreo existentes.

4. **Aislamiento del ecosistema**: Data Mirai ejecuta agentes pero no los expone como tools para otros sistemas. Claude Code, Cursor, y otras apps MCP-compatible no pueden invocar agentes de Data Mirai. El valor de los agentes queda encerrado en la UI.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Disenar grafos con caminos paralelos — multiples nodos ejecutandose al mismo tiempo con fork/join explicito
2. Guardar un agente como template reutilizable con metadata, importar templates existentes como base para nuevos agentes
3. Instrumentar la ejecucion con OpenTelemetry: traces por sesion, spans por nodo, metricas agregadas, exportable a cualquier backend OTLP
4. Exponer sus agentes como tools MCP para que Claude Code, Cursor, y otras apps los ejecuten directamente

---

## Features

### 5.1 — Ejecucion Paralela de Nodos (Fork-Join)

**Problema**: Hoy el GraphRunner es un cursor secuencial (definido en DATAMIRAI-ENGINE-PRD.md): ejecuta un nodo, mira edges de salida, sigue la que cumple la condicion, ejecuta el siguiente. Cuando hay nodos independientes sin dependencia de datos entre si, deberian ejecutarse en paralelo. Ejemplo: scraping de 3 fuentes distintas antes de un LLM que consolida — hoy tarda 3x lo necesario.

**Solucion**: Dos nuevos nodos logicos (`logic/parallel` y `logic/join`) que permiten expresar fork-join en el grafo. El GraphRunner detecta `logic/parallel`, lanza N caminos con `asyncio.gather`, y `logic/join` espera a que todos los caminos converjan antes de continuar.

**Arquitectura**:
- `logic/parallel` (fork): nodo que abre N caminos paralelos. Cada edge de salida es un camino independiente. No ejecuta logica propia — es un control-flow node que indica al runner "lanza todo esto en paralelo".
- `logic/join` (barrier): nodo que espera a que todos sus edges de entrada esten completos antes de continuar. Agrega los outputs de todos los caminos paralelos en un diccionario indexado por path_id.
- Diferencia con `logic/merge` existente: Merge acepta el PRIMER camino que llega (de bifurcacion condicional — un solo camino ejecuta). Join espera a TODOS los caminos (todos ejecutan en paralelo).
- El GraphRunner, al encontrar un nodo `logic/parallel`:
  1. Identifica todas las edges de salida del parallel → N caminos
  2. Para cada camino, sigue las edges hasta encontrar el `logic/join` correspondiente
  3. Ejecuta los N caminos con `asyncio.gather(*caminos)`
  4. Cada camino ejecuta sus nodos secuencialmente (el paralelismo es entre caminos, no dentro de un camino)
  5. Los outputs de cada camino se agregan al SharedState normalmente (cada nodo guarda su output bajo su node_id)
  6. El `logic/join` consolida: su output es `{ paths: { path_0: last_node_output, path_1: last_node_output, ... } }`
- Error handling configurable por parallel node:
  - `fail_fast: true` (default): si un camino falla, se cancelan los demas con `asyncio.cancel()`. La sesion reporta error del camino que fallo.
  - `fail_fast: false`: si un camino falla, los demas continuan. El join recibe outputs parciales + error info del camino fallido.
- Timeout por camino: hereda el timeout del agente por default, configurable por parallel node.
- Validacion del grafo: cada `logic/parallel` debe tener exactamente un `logic/join` correspondiente. El validador de GraphDef detecta parallels sin join y vice versa.

Ejemplo:
```
[trigger] -> [parallel] -> [web_scrape: fuente A] -> [join] -> [llm_call]
                        -> [web_scrape: fuente B] -/
                        -> [db_read: datos locales] -/
```

**Entidades nuevas**: Ninguna. Son nuevos tool_types (`logic/parallel`, `logic/join`) registrados en el ToolRegistry existente.

**Contratos API**: No hay endpoints nuevos. Los nodos se manipulan con los endpoints existentes de graphs (`PUT /api/graphs/{id}` para actualizar definition). La ejecucion usa el mismo `POST /api/agents/{id}/execute`.

**Pantallas**:
- **EditorCanvas**: catalogo muestra nuevos bloques "Parallel" y "Join" en categoria Logic. Parallel se visualiza como nodo con multiples edges de salida divergentes (icono fork). Join se visualiza como nodo con multiples edges de entrada convergentes (icono barrier). Validacion visual: si un parallel no tiene join, warning en el canvas.
- **ToolCatalog**: nuevos entries en categoria Logic con icon `git-fork` (parallel) y `git-merge` (join, distinto del merge existente — usar icono `combine` o similar).

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-80 | Cada `logic/parallel` DEBE tener exactamente un `logic/join` correspondiente en el grafo. El validador rechaza grafos con parallel sin join o join sin parallel | GraphDef validator |
| REGLA-81 | Max 10 caminos paralelos por nodo parallel. Configurable en config del nodo (`max_paths`, default 10). Si se excede, error de validacion | GraphDef validator + runtime check |
| REGLA-82 | El paralelismo es entre caminos, no dentro de un camino. Cada camino se ejecuta secuencialmente internamente | GraphRunner |
| REGLA-83 | En modo `fail_fast: true`, si un camino falla, los demas se cancelan inmediatamente via `asyncio.cancel()`. El error del camino fallido se propaga al join | GraphRunner |
| REGLA-84 | En modo `fail_fast: false`, caminos fallidos se registran con error info en el output del join. El join continua con outputs parciales: `{ paths: { path_0: output, path_1: { error: ... } } }` | GraphRunner + JoinTool |
| REGLA-85 | No se permiten parallels anidados en v1 (un camino paralelo no puede contener otro parallel). Simplifica la implementacion y el debug. Futuro: permitir nesting | GraphDef validator |
| REGLA-86 | Checkpoints se generan para cada nodo dentro de cada camino paralelo, igual que en ejecucion secuencial. El step_number se incrementa globalmente | GraphRunner |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/tools/builtin/logic/parallel.py` — ParallelTool con spec `logic/parallel` y JoinTool con spec `logic/join`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — agregar logica de deteccion de parallel node, ejecucion con `asyncio.gather`, manejo de caminos paralelos, convergencia en join. Metodos nuevos: `_execute_parallel()`, `_trace_parallel_paths()`, `_resolve_join()`
- MODIFICAR `framework/src/datamirai_engine/core/graph.py` — agregar validacion: parallel-join pairing, max paths, no nesting. Metodo `validate_parallel_structure()`
- MODIFICAR `app/server/datamirai_app/app.py` — registrar nuevos tools en el registry (parallel, join)
- MODIFICAR `app/web/src/components/editor/ToolCatalog.tsx` — agregar parallel y join al catalogo visual
- MODIFICAR `app/web/src/components/editor/ToolNode.tsx` — renderizado especial para parallel (fork visual) y join (barrier visual)
- CREAR `framework/tests/tools/test_parallel.py` — tests de ejecucion paralela: happy path, fail_fast, fail_tolerant, timeout, validacion
- MODIFICAR `framework/tests/tools/test_logic_blocks.py` — (si existe) agregar tests de validacion parallel-join pairing

---

### 5.3 — Graph Template Marketplace

**Problema**: Cada agente se construye desde cero. Un usuario que disena un buen patron de scraping + LLM no puede reutilizarlo ni compartirlo. No hay forma de "guardar como template" un grafo funcional ni de importar grafos de otros usuarios. El conocimiento de diseno de agentes se pierde entre proyectos y entre usuarios.

**Solucion**: Templates como snapshots de grafos completos con metadata. En version local: guardados en SQLite, gestionados via API y UI. Un template es un grafo completo (nodos, edges, data_maps, configs) congelado como plantilla reutilizable. Import copia el grafo al workspace del usuario. Export genera template desde un grafo existente.

**Arquitectura**:
- Template = snapshot del graph_def (JSON completo de GraphDef) + metadata (nombre, descripcion, categoria, tags, autor)
- Guardar como template: toma el graph_def actual de un agente, lo congela como template con metadata
- Importar template: crea un nuevo agente (con nuevo graph_id) copiando el graph_def del template. El usuario puede modificarlo libremente — no hay link entre template y agente importado
- Export: genera JSON descargable del template para importar en otra instancia
- Categorias predefinidas: `scraping`, `analysis`, `automation`, `data-pipeline`, `chatbot`, `monitoring`, `custom`
- Tags: libres, definidos por el autor del template
- Version del template: string libre (v1.0, v2.0, etc.). No hay upgrade automatico — es informativo
- Busqueda: por nombre, categoria, tags (filtro combinado en la query SQL)
- Preview: el graph_def del template se renderiza con React Flow en modo read-only para que el usuario vea la estructura antes de importar

**Entidades nuevas**:

Tabla `graph_templates` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| category | TEXT | DEFAULT 'custom' |
| tags | TEXT | JSON array, DEFAULT '[]' |
| graph_def | TEXT | JSON NOT NULL (GraphDef completo) |
| author | TEXT | DEFAULT 'local' |
| version | TEXT | DEFAULT '1.0' |
| downloads | INTEGER | DEFAULT 0 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Indice: `CREATE INDEX idx_template_category ON graph_templates(category)`

**Contratos API**:
- `POST /api/templates` — crear template desde agente existente. Body: `{ agent_id, name, description?, category?, tags?, version? }`. Toma el graph_def del agente referenciado. Response: `{ template: GraphTemplate }`
- `GET /api/templates` — listar templates con filtros. Query params: `category?`, `tag?`, `search?` (busca en name+description), `sort?` (recent/popular/name). Response: `{ templates: GraphTemplate[], total: int }`
- `GET /api/templates/{id}` — detalle del template con graph_def completo para preview. Response: `{ template: GraphTemplate }`
- `POST /api/templates/{id}/import` — importar como nuevo agente. Body: `{ environment_id, agent_name? }`. Crea nuevo graph + agent en el environment indicado. Incrementa downloads del template. Response: `{ agent: Agent, graph: Graph }`
- `DELETE /api/templates/{id}` — eliminar template. Response: `204`
- `POST /api/templates/{id}/export` — exportar como JSON descargable. Response: `{ export: { template: GraphTemplate, format_version: "1.0", exported_at: string } }` (JSON que el usuario puede guardar como archivo e importar en otra instancia)
- `POST /api/templates/import-file` — importar template desde JSON exportado. Body: JSON del export. Response: `{ template: GraphTemplate }`

**Pantallas**:
- **Nueva pagina `/templates`**: catalogo visual de templates. Grid de cards, cada card muestra: nombre, descripcion truncada, categoria como badge, tags como chips, autor, version, downloads count. Click en card abre detalle. Barra de busqueda + filtros por categoria y tags. Boton "Importar desde archivo" para JSON externo.
- **Detalle de template (modal o pagina)**: preview del grafo (React Flow en modo read-only — zoom, pan, pero no editable), metadata completa, boton "Usar como base" (import), boton "Exportar JSON".
- **AgentDetail (`/agents/[id]`)**: nuevo boton "Guardar como template" en la toolbar. Abre modal con form: nombre, descripcion, categoria (select), tags (input con chips), version. Confirmar crea el template.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-87 | Un template es un snapshot inmutable del graph_def al momento de creacion. Cambios posteriores al agente original NO actualizan el template | Service (copia deep del graph_def) |
| REGLA-88 | Import de template crea entidades nuevas (graph + agent). No hay link entre template y agente importado — son independientes post-import | Service |
| REGLA-89 | El graph_def del template debe ser valido (pasar validacion de GraphDef). No se permiten templates con grafos rotos | Validacion en POST /api/templates |
| REGLA-90 | Export genera JSON autocontenido. No hay referencias externas (credential IDs, resource IDs). Configs sensibles (API keys) se excluyen del template — solo estructura y data_maps | Serializer |
| REGLA-91 | Un template con downloads > 0 muestra warning al intentar eliminar, pero se permite la eliminacion. Los agentes importados no se ven afectados | UI warning + API permite delete |
| REGLA-92 | Categorias predefinidas: scraping, analysis, automation, data-pipeline, chatbot, monitoring, custom. El usuario puede usar cualquiera de estas. Futuro: categorias custom | Validacion en API (enum) |

**Archivos a crear/modificar**:
- MODIFICAR `app/server/datamirai_app/database.py` — tabla `graph_templates` + indice + `GraphTemplateRepo` (CRUD + filtros + import/export). Bump `SCHEMA_VERSION` a 3
- CREAR `app/server/datamirai_app/routes/templates.py` — endpoints de templates (blueprint FastAPI)
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de templates
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para todos los endpoints de templates
- CREAR `app/web/src/app/templates/page.tsx` — pagina catalogo de templates
- CREAR `app/web/src/app/templates/[id]/page.tsx` — pagina detalle de template con preview React Flow
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — boton "Guardar como template" + modal de creacion
- CREAR `app/web/src/components/templates/TemplateCard.tsx` — card de template para el catalogo
- CREAR `app/web/src/components/templates/TemplatePreview.tsx` — preview React Flow read-only del grafo
- CREAR `app/web/src/components/templates/SaveAsTemplateModal.tsx` — modal para guardar agente como template

---

### 5.4 — Observabilidad (OpenTelemetry)

**Problema**: Data Mirai tiene `execution_spans` en SQLite (FEAT-001) que registran duracion por nodo, tokens consumidos, y estado. Pero esta telemetria es propietaria — no es compatible con herramientas estandar de observabilidad. Un usuario con Grafana, Jaeger, o Datadog no puede correlacionar ejecuciones de Data Mirai con el resto de su infraestructura. No hay traces distribuidos, no hay metricas exportables, no hay logs estructurados con trace_id.

**Solucion**: Integrar OpenTelemetry SDK en el GraphRunner como layer encima de los execution_spans existentes. Traces por sesion, spans por nodo, metricas agregadas, logs estructurados. Exporters configurables: console (default para desarrollo), JSON file (para debugging local), OTLP (para Grafana/Jaeger/Datadog). La telemetria es opt-in y nunca bloquea la ejecucion.

**Arquitectura**:
- `TelemetryProvider`: clase que encapsula la inicializacion de OpenTelemetry SDK
  - `TracerProvider` con configurable exporter
  - `MeterProvider` para metricas
  - `LoggerProvider` para logs estructurados
- Integracion en GraphRunner:
  - Al iniciar sesion: crea trace con `trace_id` = session_id (o derivado)
  - Al ejecutar cada nodo: crea span hijo con atributos (node_id, tool_type, step_number)
  - Al completar nodo: cierra span con duracion, tokens, status
  - Spans de LLM calls son hijos del span del nodo que los invoca
- Metricas automaticas (counters + histograms):
  - `datamirai.session.duration` — histograma de duracion de sesiones
  - `datamirai.session.count` — counter de sesiones por status (completed/failed/timeout)
  - `datamirai.node.duration` — histograma de duracion por tool_type
  - `datamirai.node.errors` — counter de errores por tool_type
  - `datamirai.llm.tokens.input` — counter de tokens de entrada por provider
  - `datamirai.llm.tokens.output` — counter de tokens de salida por provider
  - `datamirai.llm.latency` — histograma de latencia por provider y modelo
  - `datamirai.llm.cost` — counter de costo estimado por provider
- Logs estructurados: JSON con trace_id, span_id, severity, message, attributes. Correlacionados con traces.
- Exporters:
  - `console` (default): imprime traces/metricas a stdout. Zero config, util para desarrollo.
  - `json_file`: escribe a `~/.datamirai/telemetry/` como archivos JSON rotados. Util para debugging local sin herramientas externas.
  - `otlp`: envia a endpoint OTLP (gRPC o HTTP). Configurable: endpoint URL, headers, protocol (grpc/http). Compatible con Grafana Alloy, Jaeger, Datadog Agent, cualquier OTLP receiver.
  - `none`: desactivado completamente. Sin overhead.
- Configuracion persistida en tabla `app_config` existente (key-value):
  - `telemetry.enabled` — boolean (default: true)
  - `telemetry.exporter` — string: none / console / json_file / otlp (default: console)
  - `telemetry.otlp.endpoint` — string (solo si exporter=otlp)
  - `telemetry.otlp.protocol` — string: grpc / http (default: http)
  - `telemetry.otlp.headers` — JSON object (headers custom, e.g. auth)
- La telemetria se inicializa al arrancar el server. Cambios de config requieren restart (o hot-reload via endpoint).
- Los execution_spans existentes en SQLite se mantienen como fuente de verdad local. OpenTelemetry es una capa de export adicional — no reemplaza los spans de SQLite.

**Entidades nuevas**: Ninguna. Usa tabla `app_config` existente para persistir configuracion. Los execution_spans existentes son la fuente de datos.

**Contratos API**:
- `GET /api/telemetry/config` — ver configuracion actual de telemetria. Response: `{ enabled: bool, exporter: string, otlp?: { endpoint: string, protocol: string, headers: object } }`
- `PATCH /api/telemetry/config` — cambiar configuracion. Body: campos parciales. Valida que si exporter=otlp, endpoint es requerido. Response: `{ config: TelemetryConfig, restart_required: bool }`
- `POST /api/telemetry/test` — enviar trace de prueba al exporter configurado. Util para verificar que OTLP endpoint funciona. Response: `{ success: bool, error?: string }`

**Pantallas**:
- **Settings (`/settings`) → seccion "Observabilidad"**:
  - Toggle on/off de telemetria
  - Selector de exporter (None / Console / JSON File / OTLP)
  - Si OTLP: campos endpoint, protocol (select grpc/http), headers (key-value editor)
  - Boton "Test Connection" (solo para OTLP) — envia trace de prueba
  - Indicador de estado: "Activo — exportando a console" / "Activo — OTLP endpoint OK" / "Desactivado"
  - Warning si se cambia config: "Requiere reiniciar el servidor para aplicar cambios"
- **SessionDetail (`/sessions/[id]`)**: si OTLP configurado, mostrar link "Ver trace en [herramienta]" con URL construida a partir del trace_id. Formato configurable en settings (template URL con placeholder `{trace_id}`).

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-93 | Telemetria NUNCA bloquea ejecucion. Todo export es async fire-and-forget. Si el exporter falla, se loguea warning y se continua | TelemetryProvider con try/except + fire-and-forget |
| REGLA-94 | Los execution_spans de SQLite son fuente de verdad local. OpenTelemetry es export adicional, no reemplazo. Si OTEL falla, los spans de SQLite siguen intactos | Arquitectura dual: SQLite primero, OTEL despues |
| REGLA-95 | Metricas y traces no contienen datos sensibles (prompts, responses, API keys). Solo metadata: duracion, tokens count, status, tool_type, provider name | TelemetryProvider attribute filter |
| REGLA-96 | Si exporter=otlp y endpoint no responde, la telemetria cae silenciosamente a console log de warning. No retry infinito, no backpressure | Exporter con timeout + fallback |
| REGLA-97 | El trace_id de OpenTelemetry se almacena en el session record para correlacion. Endpoint de session retorna trace_id si telemetria esta activa | Session service |
| REGLA-98 | JSON file exporter rota archivos por dia. Max 30 dias de retencion (configurable). Cleanup automatico al arrancar el server | json_file exporter |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/telemetry/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/telemetry/provider.py` — TelemetryProvider: init TracerProvider, MeterProvider, LoggerProvider. Metodos: `start_session_trace()`, `start_node_span()`, `record_metric()`, `shutdown()`
- CREAR `framework/src/datamirai_engine/telemetry/exporters.py` — factory de exporters: console, json_file, otlp. Configuracion desde dict
- CREAR `framework/src/datamirai_engine/telemetry/metrics.py` — definicion de metricas (counters, histograms) con nombres `datamirai.*`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — integrar TelemetryProvider: crear trace al inicio de sesion, spans por nodo, metricas al completar. Opt-in: si no hay provider configurado, no hay overhead
- MODIFICAR `framework/src/datamirai_engine/core/context.py` — agregar `telemetry: TelemetryProvider | None` al ExecutionContext
- CREAR `app/server/datamirai_app/routes/telemetry.py` — endpoints de config + test
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de telemetry, inicializar TelemetryProvider al startup desde app_config
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de telemetry
- MODIFICAR `app/web/src/app/settings/page.tsx` — seccion "Observabilidad" con form de config
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — link a trace externo si OTLP configurado
- CREAR `framework/tests/telemetry/test_provider.py` — tests de inicializacion, spans, metricas, fallback
- CREAR `framework/tests/telemetry/test_exporters.py` — tests de cada exporter

---

### 5.5 — MCP Server Nativo

**Problema**: Data Mirai ejecuta agentes pero no los expone al exterior. Claude Code, Cursor, Windsurf, y otras apps MCP-compatible son hoy el IDE principal de muchos desarrolladores. Estos IDEs pueden consumir tools MCP para extender sus capacidades. Pero un agente de Data Mirai no puede ser invocado como tool MCP — el valor de los agentes queda encerrado en la UI web de Data Mirai. Si un usuario tiene un agente de scraping + analisis, no puede usarlo desde Claude Code como `datamirai_execute_mi_scraper`.

**Solucion**: MCP server nativo que expone agentes de Data Mirai como tools MCP. Cada agente disponible genera un tool MCP dinamicamente. El server soporta transporte stdio (para Claude Code / Cursor) y SSE (para apps web). Autenticacion via API key simple. Se levanta como proceso separado o como endpoint del FastAPI existente.

**Arquitectura**:
- MCP server que expone tools dinamicas basadas en los agentes registrados:
  - `datamirai_list_agents` — lista agentes disponibles con nombre, descripcion, y status
  - `datamirai_execute_{agent_slug}` — ejecuta el agente con inputs dados. El slug se genera del nombre del agente (slugify). El tool acepta un JSON de trigger_data como input. Retorna el resultado de la sesion (sincrono: espera a que termine, o async: retorna session_id para polling).
  - `datamirai_get_session_status` — estado de una sesion (para ejecuciones async)
  - `datamirai_get_session_result` — resultado completo de una sesion completada
- Tools dinamicas: cuando un agente se crea/elimina/cambia de status, la lista de tools MCP se actualiza. Solo agentes con status `enabled` se exponen como tools.
- Transportes:
  - `stdio` (default): para integracion con Claude Code, Cursor, Windsurf. El server se levanta como proceso hijo. Configuracion en `claude_desktop_config.json` / `.cursor/mcp.json`.
  - `sse`: para integracion con apps web. Endpoint en el FastAPI existente (`/mcp/sse`).
- Autenticacion:
  - Para stdio: sin auth (el proceso ya es local, la confianza es implicita)
  - Para SSE: API key en header `Authorization: Bearer {key}`. Las API keys se gestionan desde la UI.
- El server MCP necesita acceso a la misma SQLite DB que el server principal para leer agentes y crear sesiones.
- CLI entry point: `datamirai mcp` levanta el MCP server en modo stdio. `datamirai mcp --transport sse --port 8001` para SSE.
- Generacion de config: endpoint que genera el JSON de configuracion para copiar a Claude Code / Cursor.

**Entidades nuevas**:

Tabla `mcp_api_keys` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| key_hash | TEXT | NOT NULL (bcrypt hash del API key) |
| permissions | TEXT | JSON, DEFAULT '["execute"]' |
| created_at | TEXT | NOT NULL |
| last_used_at | TEXT | NULL |
| status | TEXT | NOT NULL DEFAULT 'active' (active / revoked) |

Indice: `CREATE INDEX idx_mcp_key_status ON mcp_api_keys(status)`

**Contratos API**:
- `POST /api/mcp/keys` — crear API key. Body: `{ name, permissions? }`. Genera key aleatoria, almacena hash. Response: `{ id, name, key: "tk_..." (solo se muestra una vez), permissions, created_at }`. La key plaintext solo se retorna en la respuesta de creacion — despues solo se ve el hash.
- `GET /api/mcp/keys` — listar API keys (sin key plaintext). Response: `{ keys: [{ id, name, permissions, status, created_at, last_used_at }] }`
- `DELETE /api/mcp/keys/{id}` — revocar API key (soft delete: status → revoked). Response: `204`
- `GET /api/mcp/config` — generar JSON de configuracion MCP para copiar. Query param: `client` (claude_code / cursor / generic). Response: `{ config: object, instructions: string }`. Ejemplo para Claude Code:
  ```json
  {
    "config": {
      "mcpServers": {
        "datamirai": {
          "command": "datamirai",
          "args": ["mcp"],
          "env": {}
        }
      }
    },
    "instructions": "Copiar el contenido de 'config' a ~/.claude/claude_desktop_config.json"
  }
  ```
- `GET /api/mcp/status` — estado del MCP server (running/stopped, transport, connected clients). Response: `{ running: bool, transport: string, clients_connected: int, tools_available: int }`

**Pantallas**:
- **Settings (`/settings`) → seccion "MCP Server"**:
  - Toggle on/off del MCP server
  - Selector de transporte: stdio / SSE
  - Si SSE: campo de puerto (default 8001)
  - Status indicator: "Corriendo — N tools disponibles" / "Detenido"
  - Lista de API keys con acciones (crear, revocar). Al crear, modal muestra la key una sola vez con boton copiar.
  - Seccion "Conectar": tabs por client (Claude Code, Cursor, Otro). Cada tab muestra la config JSON lista para copiar con boton "Copiar config". Instrucciones paso a paso debajo.
- **AgentDetail (`/agents/[id]`)**: badge "MCP" si el agente esta expuesto como tool MCP (status enabled). Tooltip: "Este agente esta disponible como tool MCP: datamirai_execute_{slug}".

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-99 | Solo agentes con status `enabled` se exponen como tools MCP. Agentes disabled/draft no son visibles para clientes MCP | MCP server tool list filter |
| REGLA-100 | API key plaintext solo se retorna una vez (en POST response). Despues solo se almacena el hash. Si el usuario pierde la key, debe crear una nueva | Service |
| REGLA-101 | API keys revocadas rechazan requests inmediatamente (401). No se eliminan fisicamente para auditoria | Auth middleware |
| REGLA-102 | En transporte stdio, no se requiere API key (confianza local implicita). En transporte SSE, API key es obligatoria | MCP server auth layer |
| REGLA-103 | El MCP server comparte la misma SQLite DB que el server principal. No es una DB separada. Usa WAL mode para lectura concurrente | MCP server init |
| REGLA-104 | Ejecucion de agente via MCP es sincrona por default (el tool bloquea hasta que la sesion termine). Timeout configurable (default: 300s). Si excede timeout, retorna session_id para polling via `datamirai_get_session_status` | MCP tool handler |
| REGLA-105 | El slug del agente para el tool name se genera con: lowercase, reemplazar espacios por `_`, remover caracteres especiales, max 50 chars. Ejemplo: "Mi Scraper de Noticias" -> `datamirai_execute_mi_scraper_de_noticias` | Slugify util |
| REGLA-106 | Actualizacion de last_used_at en API key es async fire-and-forget. No bloquea la request MCP | Auth middleware |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/mcp/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/mcp/server.py` — Data MiraiMCPServer: clase principal que implementa MCP server protocol. Metodos: `list_tools()`, `call_tool()`, `start(transport)`, `stop()`
- CREAR `framework/src/datamirai_engine/mcp/tools.py` — generacion dinamica de tool definitions basada en agentes registrados. Mapping agent → MCP tool spec
- CREAR `framework/src/datamirai_engine/mcp/auth.py` — middleware de autenticacion por API key
- CREAR `framework/src/datamirai_engine/mcp/transports.py` — configuracion de transporte stdio y SSE
- MODIFICAR `app/server/datamirai_app/database.py` — tabla `mcp_api_keys` + `MCPApiKeyRepo` (CRUD + hash + validacion). Bump `SCHEMA_VERSION` (coordinado con 5.3)
- CREAR `app/server/datamirai_app/routes/mcp.py` — endpoints de API keys, config, status
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de MCP, opcionalmente montar SSE endpoint
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de MCP
- MODIFICAR `app/web/src/app/settings/page.tsx` — seccion "MCP Server" con toggle, keys, config copiable
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — badge MCP si agente expuesto
- CREAR `framework/tests/mcp/test_server.py` — tests de MCP server: tool generation, execution, auth
- CREAR `framework/tests/mcp/test_auth.py` — tests de API key validation, revocation

---

## Dependencias entre features

```
5.1 (Parallel)       <- independiente, no depende de las demas
5.3 (Templates)      <- independiente, no depende de las demas
5.4 (Observabilidad) <- independiente, pero beneficia a 5.1 (traces de ejecucion paralela muestran caminos)
5.5 (MCP Server)     <- independiente del resto, pero beneficia tener 5.1 (agentes con parallel son mas utiles como tools)
```

Orden sugerido de implementacion: 5.1 -> 5.3 -> 5.4 -> 5.5

Razon:
- 5.1 primero: cambia el core del runner. Mejor hacerlo temprano para que los demas features se testen con parallel disponible.
- 5.3 segundo: es self-contained (solo app layer, no toca framework core). Buen candidato para paralelizar con 5.1 si hay dos sesiones.
- 5.4 tercero: instrumenta el runner ya modificado (con parallel). Los traces de ejecucion paralela son un buen caso de prueba.
- 5.5 ultimo: expone todo lo construido. Conviene que parallel y templates ya existan para que los agentes expuestos por MCP sean mas potentes.

Dentro de cada feature, orden sugerido:
- **5.1**: ParallelTool + JoinTool -> GraphDef validator -> GraphRunner parallel execution -> tests -> UI
- **5.3**: DB schema + repo -> API endpoints -> UI paginas -> tests
- **5.4**: TelemetryProvider + exporters -> runner integration -> API config -> UI settings -> tests
- **5.5**: MCP server core -> tool generation -> auth -> CLI entry point -> API + UI -> tests

---

## Entidades nuevas (resumen consolidado)

### graph_templates
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| category | TEXT | DEFAULT 'custom' |
| tags | TEXT | JSON array, DEFAULT '[]' |
| graph_def | TEXT | JSON NOT NULL |
| author | TEXT | DEFAULT 'local' |
| version | TEXT | DEFAULT '1.0' |
| downloads | INTEGER | DEFAULT 0 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### mcp_api_keys
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| key_hash | TEXT | NOT NULL |
| permissions | TEXT | JSON, DEFAULT '["execute"]' |
| created_at | TEXT | NOT NULL |
| last_used_at | TEXT | NULL |
| status | TEXT | NOT NULL DEFAULT 'active' |

---

## Maquinas de estado

### MCP API Key
```
active -> revoked
```
Sin transicion inversa. Key revocada es permanente — crear nueva key.

No hay otras maquinas de estado nuevas. Los templates y la telemetria son datos pasivos sin transiciones complejas. Los nodos parallel/join son control-flow, no tienen estado propio.

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-80 | Cada parallel DEBE tener exactamente un join correspondiente | GraphDef validator |
| REGLA-81 | Max 10 caminos paralelos por nodo (configurable, default 10) | GraphDef validator + runtime |
| REGLA-82 | Paralelismo entre caminos, no dentro de un camino | GraphRunner |
| REGLA-83 | fail_fast=true: camino falla -> cancelar los demas | GraphRunner |
| REGLA-84 | fail_fast=false: caminos fallidos reportan error en output del join | GraphRunner + JoinTool |
| REGLA-85 | No parallels anidados en v1 | GraphDef validator |
| REGLA-86 | Checkpoints se generan por nodo dentro de caminos paralelos | GraphRunner |
| REGLA-87 | Template es snapshot inmutable del graph_def | Service |
| REGLA-88 | Import crea entidades nuevas, sin link al template | Service |
| REGLA-89 | graph_def del template debe ser valido | Validacion en API |
| REGLA-90 | Export sin referencias externas ni datos sensibles | Serializer |
| REGLA-91 | Template con downloads > 0 se puede eliminar (con warning) | UI + API |
| REGLA-92 | Categorias predefinidas para templates | Validacion en API |
| REGLA-93 | Telemetria nunca bloquea ejecucion (async fire-and-forget) | TelemetryProvider |
| REGLA-94 | execution_spans SQLite = fuente de verdad. OTEL = export adicional | Arquitectura dual |
| REGLA-95 | Traces no contienen datos sensibles (prompts, keys) | Attribute filter |
| REGLA-96 | OTLP endpoint caido -> fallback silencioso a warning | Exporter timeout |
| REGLA-97 | trace_id almacenado en session para correlacion | Session service |
| REGLA-98 | JSON file exporter rota por dia, max 30 dias | Exporter |
| REGLA-99 | Solo agentes enabled se exponen como tools MCP | MCP server filter |
| REGLA-100 | API key plaintext solo una vez en response de creacion | Service |
| REGLA-101 | Keys revocadas rechazan inmediatamente (401) | Auth middleware |
| REGLA-102 | stdio sin auth, SSE requiere API key | MCP auth layer |
| REGLA-103 | MCP server comparte SQLite DB con server principal (WAL) | MCP init |
| REGLA-104 | Ejecucion MCP sincrona default, timeout 300s, fallback a polling | MCP tool handler |
| REGLA-105 | Slug de agente: lowercase, _ por espacios, sin especiales, max 50 | Slugify util |
| REGLA-106 | last_used_at update es async fire-and-forget | Auth middleware |

---

## Notas de implementacion

- **asyncio.gather para parallel**: El runner ya es async. La ejecucion paralela usa `asyncio.gather(*tasks, return_exceptions=fail_fast==False)`. Cada task es una corutina que ejecuta un camino secuencialmente.
- **Parallel path tracing**: Para saber que nodos pertenecen a cada camino paralelo, el runner hace un graph traversal desde cada edge del parallel hasta encontrar el join. Esto se resuelve en "compile time" (antes de ejecutar), no en runtime.
- **OpenTelemetry SDK**: dependencia opcional del framework. `pip install datamirai-engine[telemetry]` instala `opentelemetry-api`, `opentelemetry-sdk`, `opentelemetry-exporter-otlp-proto-http`. Sin el extra, la telemetria no se activa y no hay overhead.
- **MCP SDK**: usa `mcp` (Model Context Protocol SDK de Anthropic). Dependencia opcional: `pip install datamirai-engine[mcp]`. El server implementa el protocolo MCP estandar.
- **Templates y SCHEMA_VERSION**: 5.3 y 5.5 ambos agregan tablas al schema. Si se implementan en paralelo, coordinar el bump de SCHEMA_VERSION (usar migraciones incrementales, no reemplazar).
- **Preview React Flow de templates**: reutilizar el componente EditorCanvas existente en modo read-only (props: `editable=false`, `showToolbar=false`). No crear un renderizador nuevo.
- **MCP tool names**: el formato `datamirai_execute_{slug}` es deliberado — el prefijo `datamirai_` evita colision con tools de otros MCP servers. El slug se genera determinista del nombre del agente.
- **Backward compatibility**: todos los features son opt-in. Un grafo existente sin parallel funciona identico. Telemetria desactivada no tiene overhead. MCP server solo se levanta si el usuario lo activa. Templates son una tabla nueva sin impacto en el flujo existente.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine (modelo de ejecucion secuencial que 5.1 extiende)
- `docs/ARCHITECTURE.md` — stack, convenciones, estructura del repo
- `docs/prd/draft/FEAT-001.md` — MVP features (execution_spans que 5.4 extiende, session model que 5.5 usa)
- `docs/prd/draft/FEAT-002.md` — Tier 1 (multi-LLM que 5.5 expone via MCP, memory que templates pueden incluir)
