# FEAT-017 — A2A Protocol (Agent-to-Agent)

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-035

---

## Problem Statement

**Tipo**: Feature nueva (protocolo de interoperabilidad entre agentes)
**Actor**: Usuario avanzado — persona que opera multiples sistemas de agentes y necesita que colaboren.

Hoy los agentes de Data Mirai Engine son islas. Un agente no puede invocar a otro agente — ni dentro de la misma instancia de Data Mirai, ni en instancias externas, ni en runtimes de terceros (LangGraph, Google ADK, CrewAI). Esto tiene tres problemas:

1. **Sin composicion entre agentes locales**: Si el usuario tiene un agente que analiza datos y otro que genera reportes, no hay forma de que el segundo llame al primero automaticamente. El usuario tiene que ejecutar manualmente el primero, copiar el output, y pasarlo como input al segundo. No hay orquestacion agent-to-agent.

2. **Sin interoperabilidad externa**: El ecosistema de agentes esta fragmentado. Google propuso el protocolo A2A (Agent-to-Agent) como estandar para que agentes de diferentes runtimes colaboren. Data Mirai no lo implementa. Un agente Data Mirai no puede ni exponerse como servicio A2A ni consumir agentes A2A externos.

3. **Sin delegacion de tareas**: En architecturas multi-agente, un agente "orquestador" necesita delegar sub-tareas a agentes especializados. Hoy la unica forma es encadenar sesiones manualmente via API. No hay primitiva de delegacion dentro del grafo ni tracking de tareas delegadas.

Para ser un engine serio de agentes en produccion, Data Mirai necesita interoperabilidad — tanto interna como con el ecosistema externo.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Exponer cualquier agente de Data Mirai como servicio A2A descubrible (con Agent Card)
2. Consumir agentes A2A externos desde un grafo de Data Mirai (tool a2a/call_agent)
3. Orquestar delegacion de tareas a agentes internos o externos, sincrono o asincrono
4. Compartir contexto relevante con agentes externos sin exponer todo el estado interno
5. Monitorear el status de tareas delegadas a agentes externos

---

## Features

### 17.1 — A2A Server

**Problema**: Los agentes de Data Mirai no son accesibles desde fuera excepto via la API REST interna que esta diseñada para la UI, no para consumo de otros agentes. No hay Agent Card (metadata descubrible), no hay endpoint estandar de ejecucion, no hay schema de input/output publicado.

**Solucion**: Implementar la especificacion A2A server que expone agentes de Data Mirai como servicios descubribles y ejecutables por cualquier cliente A2A compatible. Cada agente tiene un Agent Card con metadata y un endpoint de ejecucion estandar.

**Arquitectura**:
- **Agent Card** (conforme a A2A spec):
  - `name: str` — nombre del agente
  - `description: str` — descripcion de que hace
  - `url: str` — URL del endpoint de ejecucion
  - `version: str` — version del agente
  - `capabilities: list[str]` — capacidades declaradas (ej: "data_analysis", "content_generation")
  - `input_schema: dict` — JSON Schema de los inputs esperados (derivado del trigger del agente)
  - `output_schema: dict` — JSON Schema de los outputs que produce (derivado del output del grafo)
  - `authentication: dict` — metodo de auth requerido (none, api_key, oauth)
  - `provider: dict` — metadata del provider (name: "Data Mirai Engine", url: "https://datamirai.dev")
- **Endpoints A2A**:
  - `GET /.well-known/agent.json` — Agent Card discovery (conforme a A2A spec). Retorna lista de Agent Cards de todos los agentes habilitados para A2A.
  - `GET /a2a/agents` — listar agentes expuestos como A2A. Response: `{ agents: AgentCard[] }`
  - `GET /a2a/agents/{id}/card` — Agent Card individual. Response: `AgentCard`
  - `POST /a2a/agents/{id}/execute` — ejecutar agente. Body: `{ input: dict, callback_url?: str, correlation_id?: str }`. Response sincrona: `{ task_id: str, status: "completed", output: dict }`. Response asincrona: `{ task_id: str, status: "accepted", status_url: str }`.
  - `GET /a2a/tasks/{taskId}` — status de tarea asincrona. Response: `{ task_id: str, status: "pending" | "running" | "completed" | "failed", output?: dict, error?: str }`
- **A2A Gateway Service**:
  - Recibe requests A2A, los traduce a sesiones internas de Data Mirai
  - Crea session con trigger_data del input A2A
  - Para modo sincrono: espera que la session complete y retorna output
  - Para modo asincrono: retorna task_id inmediatamente, hace callback cuando completa
  - Rate limiting configurable por agente
- **Habilitacion por agente**: los agentes no se exponen como A2A por defecto. El usuario habilita A2A explicitamente por agente, configurando input/output schemas y auth.

**Entidades nuevas**:

Tabla `a2a_config` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL, UNIQUE |
| enabled | INTEGER | NOT NULL DEFAULT 0 |
| input_schema | TEXT | JSON, DEFAULT '{}' |
| output_schema | TEXT | JSON, DEFAULT '{}' |
| capabilities | TEXT | JSON array, DEFAULT '[]' |
| auth_method | TEXT | NOT NULL DEFAULT 'none' |
| auth_config | TEXT | JSON, DEFAULT '{}' |
| rate_limit_rpm | INTEGER | DEFAULT 60 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Tabla `a2a_task` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| input | TEXT | JSON, NOT NULL |
| output | TEXT | JSON, NULL |
| error | TEXT | NULL |
| callback_url | TEXT | NULL |
| correlation_id | TEXT | NULL |
| caller_info | TEXT | JSON, DEFAULT '{}' |
| created_at | TEXT | NOT NULL |
| completed_at | TEXT | NULL |

CREATE INDEX idx_a2a_config_agent ON a2a_config(agent_id);
CREATE INDEX idx_a2a_task_agent ON a2a_task(agent_id);
CREATE INDEX idx_a2a_task_status ON a2a_task(status);

**Contratos API** (endpoints internos para gestionar A2A, separados de los endpoints A2A publicos):
- `GET /api/agents/{id}/a2a-config` — obtener config A2A del agente. Response: `{ config: A2AConfig | null }`
- `PUT /api/agents/{id}/a2a-config` — configurar A2A del agente. Body: `{ enabled, input_schema?, output_schema?, capabilities?, auth_method?, auth_config?, rate_limit_rpm? }`. Response: `{ config: A2AConfig }`
- `GET /api/agents/{id}/a2a-tasks` — listar tareas A2A del agente. Query params: `status`, `limit`. Response: `{ tasks: A2ATask[], total: int }`

**Pantallas**:
- **AgentDetail → tab "A2A"** (o seccion dentro de Settings del agente):
  - Toggle "Enable A2A" (on/off)
  - Cuando habilitado:
    - URL del endpoint A2A (copiable)
    - Editor de input_schema (JSON Schema editor o form builder)
    - Editor de output_schema (JSON Schema editor)
    - Editor de capabilities (tags input)
    - Selector de auth method (none, api_key) + config
    - Rate limit config (RPM input)
    - Preview del Agent Card generado
  - Seccion "Recent Tasks": tabla de tareas A2A recientes con status, caller, fecha

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-300 | Agentes NO se exponen como A2A por defecto. Requiere habilitacion explicita con enabled=true en a2a_config | Check en A2A gateway |
| REGLA-301 | El endpoint /.well-known/agent.json solo incluye agentes con A2A habilitado y con status enabled | Filter query |
| REGLA-302 | Input validation: el input del request A2A se valida contra input_schema. Si no cumple, 400 con errores descriptivos | JSON Schema validation |
| REGLA-303 | Rate limiting se aplica por agente. Exceder → 429 con Retry-After header | Rate limiter middleware |
| REGLA-304 | El output de una session A2A se filtra segun output_schema. Solo se retorna lo declarado, no todo el SharedState | Output filter en gateway |

**Archivos a crear/modificar**:
- CREAR `app/server/datamirai_app/routes/a2a.py` — endpoints A2A publicos (execute, tasks, well-known, cards)
- CREAR `app/server/datamirai_app/routes/a2a_config.py` — endpoints internos de config A2A
- CREAR `app/server/datamirai_app/a2a_gateway.py` — A2A Gateway Service (traduce requests a sessions)
- CREAR `app/server/datamirai_app/middleware/rate_limiter.py` — rate limiting por agente
- MODIFICAR `app/server/datamirai_app/database.py` — tablas a2a_config, a2a_task
- MODIFICAR `app/server/datamirai_app/app.py` — registrar routes A2A
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para A2A config
- CREAR `app/web/src/components/a2a/A2AConfigPanel.tsx` — panel de configuracion A2A
- CREAR `app/web/src/components/a2a/AgentCardPreview.tsx` — preview del Agent Card
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab/seccion A2A

---

### 17.2 — A2A Client

**Problema**: Un agente de Data Mirai no puede invocar agentes A2A externos. Si hay un servicio A2A disponible (otro Data Mirai, un agente de LangGraph expuesto via A2A, cualquier servicio compatible), no hay forma de llamarlo desde un grafo.

**Solucion**: Nuevo tool `a2a/call_agent` que permite a un nodo del grafo ejecutar un agente A2A externo. Incluye service discovery para registrar y descubrir agentes externos.

**Arquitectura**:
- `A2AClient`:
  - `discover(url) -> AgentCard` — obtiene Agent Card de un endpoint externo
  - `execute(agent_url, input, mode) -> A2AResponse` — ejecuta agente externo
  - `check_task(status_url) -> A2ATaskStatus` — verifica status de tarea asincrona
  - Implementa retry con backoff para llamadas fallidas
  - Timeout configurable por llamada
- `A2AResponse`:
  - `task_id: str`
  - `status: str` — completed, accepted (async)
  - `output: dict | None` — output si completed
  - `status_url: str | None` — URL para polling si async
- `ExternalAgentRegistry`:
  - Almacena URLs de agentes A2A externos conocidos
  - Periodicamente refresca Agent Cards (configurable, default cada hora)
  - Cache de Agent Cards para evitar discovery en cada llamada
- Tool spec (`a2a/call_agent`):
  - Inputs:
    - `agent_url: str` (optional) — URL directa del agente A2A externo
    - `agent_registry_id: str` (optional) — ID del agente en el registro local
    - `input: dict` (required) — datos a enviar al agente
    - `mode: str` (optional) — `sync` (default) o `async`
  - Config:
    - `timeout_seconds: int` — timeout para llamadas sincronas (default 120)
    - `auth: dict` — credenciales para autenticarse con el agente externo
  - Outputs:
    - `output: dict` — respuesta del agente externo
    - `task_id: str` — ID de la tarea en el agente externo
    - `status: str` — status final
    - `agent_name: str` — nombre del agente que respondio (del Agent Card)
- Para modo asincrono: el nodo crea una interrupcion (interrupt) y espera el callback o el usuario lo resuelve manualmente

**Entidades nuevas**:

Tabla `external_agent` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| url | TEXT | NOT NULL |
| agent_card | TEXT | JSON, DEFAULT '{}' |
| auth_method | TEXT | DEFAULT 'none' |
| auth_config | TEXT | JSON, DEFAULT '{}' |
| last_discovered_at | TEXT | NULL |
| status | TEXT | NOT NULL DEFAULT 'unknown' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

CREATE INDEX idx_external_agent_status ON external_agent(status);

**Contratos API**:
- `GET /api/external-agents` — listar agentes externos registrados. Response: `{ agents: ExternalAgent[] }`
- `POST /api/external-agents` — registrar agente externo. Body: `{ name, url, auth_method?, auth_config? }`. Response: `{ agent: ExternalAgent }`. Automaticamente hace discovery del Agent Card.
- `GET /api/external-agents/{id}` — detalle del agente externo. Response: `{ agent: ExternalAgent }`
- `PATCH /api/external-agents/{id}` — actualizar config. Body: campos parciales. Response: `{ agent: ExternalAgent }`
- `DELETE /api/external-agents/{id}` — eliminar agente externo. Response: `204`
- `POST /api/external-agents/{id}/discover` — re-descubrir Agent Card. Response: `{ agent: ExternalAgent }` (con card actualizado)
- `POST /api/external-agents/{id}/test` — test de conexion. Response: `{ success: bool, latency_ms: int, error?: str }`

**Pantallas**:
- **Settings → seccion "External Agents"**:
  - Lista de agentes externos registrados con nombre, URL, status badge (connected/unreachable/unknown)
  - Boton "+ Register Agent" abre formulario: URL, nombre (auto-detectado de Agent Card), auth config
  - Por agente: acciones Test, Refresh Card, Delete
  - Click en agente muestra Agent Card completo: capabilities, input/output schemas, provider info
- **NodeConfigPanel para a2a/call_agent**:
  - Selector de agente: dropdown con agentes del registro + opcion "Custom URL"
  - Preview del Agent Card del agente seleccionado
  - Modo: sync/async toggle
  - Auth config (si el agente lo requiere)
  - Timeout slider
  - Mapeo de input: data_map que conecta outputs de nodos previos a los inputs esperados por el agente externo

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-305 | Los credentials de auth para agentes externos se almacenan de la misma forma que otras credenciales (referencia a vault). Nunca en plaintext en external_agent | Vault reference |
| REGLA-306 | Si el agente externo no responde en timeout_seconds, el nodo falla con error descriptivo incluyendo URL y timeout | asyncio.wait_for |
| REGLA-307 | El input enviado al agente externo se valida contra el input_schema del Agent Card (si disponible). Mismatch → warning, no bloqueo (porque schemas pueden estar desactualizados) | Validation con warning |
| REGLA-308 | Discovery automatico: al registrar una URL, se intenta obtener Agent Card. Si falla, se registra con status "unreachable" pero no se rechaza | Graceful discovery |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/a2a/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/a2a/client.py` — A2AClient
- CREAR `framework/src/datamirai_engine/a2a/models.py` — AgentCard, A2AResponse, A2ATaskStatus
- CREAR `framework/src/datamirai_engine/a2a/registry.py` — ExternalAgentRegistry
- CREAR `framework/src/datamirai_engine/tools/builtin/agent/call_agent.py` — tool a2a/call_agent
- CREAR `app/server/datamirai_app/routes/external_agents.py` — endpoints CRUD + discover + test
- MODIFICAR `app/server/datamirai_app/database.py` — tabla external_agent
- MODIFICAR `app/server/datamirai_app/app.py` — registrar routes de external_agents
- MODIFICAR `app/web/src/lib/api.ts` — funciones client
- CREAR `app/web/src/app/settings/external-agents/page.tsx` — pagina de gestion
- CREAR `app/web/src/components/a2a/ExternalAgentCard.tsx` — card de visualizacion de Agent Card
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — config para a2a/call_agent

---

### 17.3 — Task Delegation

**Problema**: Invocar un agente externo via A2A es atomico — llamas, recibes respuesta, listo. Pero en architecturas multi-agente complejas, se necesita delegar tareas con tracking: saber que tareas estan pendientes, cuales completaron, cuales fallaron. Tambien se necesita delegacion asincrona donde el agente principal continua trabajando mientras espera al agente delegado.

**Solucion**: Sistema de delegacion de tareas que orquesta llamadas A2A (internas y externas) con tracking, callbacks y status. Un agente "orquestador" puede delegar multiples sub-tareas en paralelo y esperar resultados selectivamente.

**Arquitectura**:
- `TaskDelegation` dataclass:
  - `id: str` — UUID
  - `parent_session_id: str` — session del agente que delega
  - `parent_node_id: str` — nodo que origino la delegacion
  - `target_type: str` — `internal` (agente Data Mirai local) o `external` (agente A2A externo)
  - `target_agent_id: str` — ID del agente destino (interno) o external_agent ID
  - `input: dict` — datos enviados
  - `output: dict | None` — resultado recibido
  - `status: str` — `pending`, `running`, `completed`, `failed`, `timeout`
  - `mode: str` — `sync` o `async`
  - `callback_url: str | None` — para delegaciones async externas
  - `created_at, completed_at`
- `DelegationService`:
  - `delegate_internal(session, node_id, target_agent_id, input, mode) -> TaskDelegation`
    - Si sync: crea session interna del agente target, espera completion, retorna output
    - Si async: crea session interna, registra delegacion, retorna inmediatamente. Se resuelve via event.
  - `delegate_external(session, node_id, external_agent_id, input, mode) -> TaskDelegation`
    - Usa A2AClient (17.2) para ejecutar
    - Si sync: espera respuesta
    - Si async: registra callback_url, espera callback o polling
  - `check_status(delegation_id) -> TaskDelegation` — verificar status
  - `on_callback(task_id, output) -> None` — procesar callback de agente externo
- Tool nuevo `a2a/delegate`:
  - Inputs:
    - `target: str` (required) — `internal:{agent_id}` o `external:{agent_id}`
    - `input: dict` (required) — datos a enviar
    - `mode: str` (optional) — `sync` (default) o `async`
    - `timeout_seconds: int` (optional) — timeout para sync (default 120)
  - Outputs:
    - `delegation_id: str` — ID de la delegacion
    - `status: str` — status final (sync) o actual (async)
    - `output: dict | None` — resultado si sync y completado
- Para delegacion async: el nodo crea una interrupcion (DELEGATION_PENDING). Cuando el agente delegado completa, el DelegationService resuelve la interrupcion y la session se reanuda desde el checkpoint.
- Soporte para delegacion multiple: un nodo `logic/parallel_delegate` puede delegar a N agentes en paralelo y esperar todos (fan-out/fan-in).

**Entidades nuevas**:

Tabla `task_delegation` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| parent_session_id | TEXT | FK sessions(id) ON DELETE CASCADE, NOT NULL |
| parent_node_id | TEXT | NOT NULL |
| target_type | TEXT | NOT NULL (internal / external) |
| target_agent_id | TEXT | NOT NULL |
| target_session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| a2a_task_id | TEXT | NULL |
| input | TEXT | JSON, NOT NULL |
| output | TEXT | JSON, NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| mode | TEXT | NOT NULL DEFAULT 'sync' |
| error | TEXT | NULL |
| created_at | TEXT | NOT NULL |
| completed_at | TEXT | NULL |

CREATE INDEX idx_delegation_parent_session ON task_delegation(parent_session_id);
CREATE INDEX idx_delegation_status ON task_delegation(status);
CREATE INDEX idx_delegation_target ON task_delegation(target_type, target_agent_id);

**Contratos API**:
- `GET /api/sessions/{id}/delegations` — listar delegaciones de una session. Response: `{ delegations: TaskDelegation[] }`
- `GET /api/delegations/{id}` — detalle de una delegacion. Response: `{ delegation: TaskDelegation }`
- `POST /a2a/callbacks/{taskId}` — endpoint publico para recibir callbacks de agentes externos. Body: `{ output: dict }`. Response: `200`. Resuelve la delegacion y reanuda la session si aplica.

**Pantallas**:
- **Session detail → seccion "Delegations"**:
  - Lista de delegaciones con: target agent name, status badge, modo (sync/async badge), duracion
  - Click en delegacion expande: input enviado, output recibido, target session (link)
  - Timeline visual de delegaciones paralelas si las hay
- **AgentDetail → tab "A2A" → seccion "Delegations Received"**:
  - Lista de tareas que este agente recibio de otros (tanto A2A como delegaciones internas)
  - Filtros por status

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-309 | Delegacion interna crea session real del agente target. Visible en historial de sessions de ese agente con tag `source: delegation, parent_session_id: X` | Session creation en DelegationService |
| REGLA-310 | Delegacion async crea interrupcion DELEGATION_PENDING en la session padre. La session se pausa hasta que se resuelve. Checkpoint se crea para resumir | Interrupt + checkpoint system |
| REGLA-311 | Si el agente target falla, la delegacion se marca como failed. La session padre recibe el error en el output de la delegacion — NO se cancela automaticamente. El grafo puede manejar el error | Error passthrough |
| REGLA-312 | No se permite delegacion circular: agente A delega a B, B delega a A. Se detecta via parent_session_id chain. Profundidad maxima: 5 niveles | Depth check en DelegationService |
| REGLA-313 | Callback endpoint (/a2a/callbacks) es publico pero validado. Solo acepta callbacks para task_ids existentes en a2a_task. Cualquier otro retorna 404 | ID validation |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/a2a/delegation.py` — TaskDelegation, DelegationService
- CREAR `framework/src/datamirai_engine/tools/builtin/agent/delegate.py` — tool a2a/delegate
- CREAR `app/server/datamirai_app/routes/delegations.py` — endpoints de delegacion + callback
- MODIFICAR `app/server/datamirai_app/database.py` — tabla task_delegation
- MODIFICAR `app/server/datamirai_app/app.py` — registrar routes de delegations
- MODIFICAR `app/web/src/lib/api.ts` — funciones client
- CREAR `app/web/src/components/a2a/DelegationTimeline.tsx` — timeline visual de delegaciones

---

### 17.4 — Shared Context

**Problema**: Cuando un agente delega a otro (interno o externo), necesita compartir contexto relevante: datos del state, resultados de nodos previos, metadata. Pero no debe compartir TODO el state (privacidad, tamano, relevancia). Hoy no hay mecanismo para seleccionar que compartir.

**Solucion**: Sistema de context sharing que permite al agente orquestador seleccionar que partes del state compartir con el agente delegado, y un normalizador que parsea los resultados del agente externo al formato interno.

**Arquitectura**:
- `ContextSelector`:
  - Define que datos del SharedState compartir con el agente delegado
  - Configuracion por nodo de delegacion:
    - `include_paths: list[str]` — paths del state a incluir (ej: `nodes.scraper.output.content`, `trigger_data.user_id`)
    - `exclude_paths: list[str]` — paths a excluir (ej: `nodes.llm_1.output.raw` — para evitar compartir datos grandes)
    - `transform: dict` — rename keys para mapear a los inputs esperados del agente destino
  - `select(shared_state, config) -> dict` — extrae y transforma los datos seleccionados
- `ResponseNormalizer`:
  - Recibe output del agente externo y lo normaliza al formato interno
  - Configuracion:
    - `output_map: dict` — mapeo de keys del output externo a keys internas (ej: `{ "result.text": "analysis_result", "result.score": "confidence" }`)
  - `normalize(external_output, config) -> dict` — produce output normalizado que se integra al SharedState
- Integracion con delegation:
  - Al delegar, el DelegationService:
    1. Usa ContextSelector para extraer datos del state
    2. Merge con el `input` del nodo de delegacion
    3. Envia al agente target
  - Al recibir respuesta:
    1. Usa ResponseNormalizer para convertir output
    2. Coloca resultado normalizado en el SharedState del nodo de delegacion
- Tool a2a/delegate (extendido):
  - Config adicional:
    - `context_config: dict` — configuracion del ContextSelector
    - `response_map: dict` — configuracion del ResponseNormalizer

**Entidades**: No se crean tablas nuevas. La configuracion de context/response mapping vive en la config del nodo de delegacion (dentro del graph definition).

**Contratos API**: No se crean endpoints nuevos. Los cambios son internos al framework.

**Pantallas**:
- **NodeConfigPanel para a2a/delegate** (expandido):
  - Seccion "Shared Context":
    - Lista de paths a incluir del state (con autocomplete basado en nodos del grafo)
    - Lista de paths a excluir
    - Mapeo de transformacion: key de state → key para agente destino (tabla editable)
  - Seccion "Response Mapping":
    - Mapeo de keys del output externo → keys en el state interno (tabla editable)
    - Preview de como quedaria el output normalizado

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-314 | Por defecto, NADA del state se comparte. El usuario debe configurar explicitamente include_paths. No hay "compartir todo" automatico | Default include_paths = [] |
| REGLA-315 | API keys, credentials y tokens NUNCA se incluyen en context compartido, aunque el usuario los ponga en include_paths. Blacklist hardcodeada | ContextSelector blacklist filter |
| REGLA-316 | Si include_paths referencia un path que no existe en el state, se ignora silenciosamente (no error). Puede que el nodo aun no haya ejecutado | Safe path resolution |
| REGLA-317 | Response normalizer es best-effort. Si un key del output_map no existe en la respuesta externa, se omite. No falla | Safe key resolution |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/a2a/context.py` — ContextSelector, ResponseNormalizer
- MODIFICAR `framework/src/datamirai_engine/a2a/delegation.py` — integrar ContextSelector y ResponseNormalizer en el flujo de delegacion
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/agent/delegate.py` — agregar config de context_config y response_map
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — seccion Shared Context y Response Mapping para a2a/delegate

---

## Dependencias entre features

```
17.1 (A2A Server) ← independiente, se implementa primero (base para recibir requests)
17.2 (A2A Client) ← independiente de 17.1, puede implementarse en paralelo (base para enviar requests)
17.3 (Task Delegation) ← depende de 17.1 (recibir delegaciones internas) + 17.2 (delegar a externos)
17.4 (Shared Context) ← depende de 17.3 para integrarse al flujo de delegacion
```

Orden de implementacion: 17.1 + 17.2 (paralelo) → 17.3 → 17.4

---

## Entidades nuevas (resumen consolidado)

### a2a_config
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL, UNIQUE |
| enabled | INTEGER | NOT NULL DEFAULT 0 |
| input_schema | TEXT | JSON, DEFAULT '{}' |
| output_schema | TEXT | JSON, DEFAULT '{}' |
| capabilities | TEXT | JSON array, DEFAULT '[]' |
| auth_method | TEXT | NOT NULL DEFAULT 'none' |
| auth_config | TEXT | JSON, DEFAULT '{}' |
| rate_limit_rpm | INTEGER | DEFAULT 60 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### a2a_task
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| input | TEXT | JSON, NOT NULL |
| output | TEXT | JSON, NULL |
| error | TEXT | NULL |
| callback_url | TEXT | NULL |
| correlation_id | TEXT | NULL |
| caller_info | TEXT | JSON, DEFAULT '{}' |
| created_at | TEXT | NOT NULL |
| completed_at | TEXT | NULL |

### external_agent
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| url | TEXT | NOT NULL |
| agent_card | TEXT | JSON, DEFAULT '{}' |
| auth_method | TEXT | DEFAULT 'none' |
| auth_config | TEXT | JSON, DEFAULT '{}' |
| last_discovered_at | TEXT | NULL |
| status | TEXT | NOT NULL DEFAULT 'unknown' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### task_delegation
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| parent_session_id | TEXT | FK sessions(id) ON DELETE CASCADE, NOT NULL |
| parent_node_id | TEXT | NOT NULL |
| target_type | TEXT | NOT NULL (internal / external) |
| target_agent_id | TEXT | NOT NULL |
| target_session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| a2a_task_id | TEXT | NULL |
| input | TEXT | JSON, NOT NULL |
| output | TEXT | JSON, NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| mode | TEXT | NOT NULL DEFAULT 'sync' |
| error | TEXT | NULL |
| created_at | TEXT | NOT NULL |
| completed_at | TEXT | NULL |

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-300 | Agentes no se exponen A2A por defecto. Requiere enabled=true | Gateway check |
| REGLA-301 | well-known solo incluye agentes A2A habilitados | Filter query |
| REGLA-302 | Input validado contra schema. Mismatch → 400 | JSON Schema validation |
| REGLA-303 | Rate limit por agente. Exceder → 429 | Rate limiter |
| REGLA-304 | Output filtrado segun output_schema | Output filter |
| REGLA-305 | Auth de agentes externos via vault | Vault reference |
| REGLA-306 | Timeout de agente externo → error descriptivo | asyncio.wait_for |
| REGLA-307 | Input vs schema de Agent Card: warning, no bloqueo | Validation con warning |
| REGLA-308 | Discovery falla → status unreachable, no rechazo | Graceful discovery |
| REGLA-309 | Delegacion interna crea session real con tag source:delegation | Session creation |
| REGLA-310 | Delegacion async crea interrupcion DELEGATION_PENDING | Interrupt system |
| REGLA-311 | Fallo en target → delegation failed, session padre no se cancela | Error passthrough |
| REGLA-312 | No delegacion circular. Max profundidad: 5 | Depth check |
| REGLA-313 | Callback solo para task_ids existentes | ID validation |
| REGLA-314 | Context sharing: nada por defecto. Include explicito | Default [] |
| REGLA-315 | API keys/credentials NUNCA en context compartido | Blacklist filter |
| REGLA-316 | Path inexistente en state → ignorar silenciosamente | Safe resolution |
| REGLA-317 | Key inexistente en output externo → omitir | Safe resolution |

---

## Notas de implementacion

- **A2A protocol reference**: basado en la propuesta de Google (mayo 2025). El protocolo no esta 100% finalizado como estandar. Implementamos el subset estable: Agent Card discovery, execute endpoint, task status. Si el protocolo evoluciona, se adapta.
- **Delegacion interna reutiliza la misma infraestructura de sessions**. Cuando agente A delega a agente B internamente, B crea una session real. Es identico a ejecutar B directamente, excepto que el trigger_data viene del contexto de A (no de un trigger real). La session de B se linkea via task_delegation.target_session_id.
- **Delegacion async usa el sistema de interrupts existente**. El nodo crea una interrupcion de tipo DELEGATION_PENDING (nuevo tipo). Cuando la delegacion completa, se resuelve la interrupcion con el output y la session se reanuda desde el checkpoint. Esto reutiliza toda la infra de checkpoints + resume ya implementada.
- **Rate limiting simple**: in-memory counter con sliding window (1 minuto). Suficiente para app local single-instance. No se necesita Redis. Si en futuro se necesita distribuido, se migra a Redis.
- **Los endpoints A2A (/a2a/*) son separados de los endpoints internos (/api/*)**. Diferente auth, diferente rate limiting, diferente middleware. Los A2A son publicos (accesibles desde fuera), los API son para la UI local.
- **ContextSelector blacklist**: hardcodear paths que NUNCA se comparten: `*.api_key`, `*.credential`, `*.token`, `*.secret`, `*.password`. Aplicado como filtro final despues del include_paths.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones, interrupt system, checkpoints
- `docs/prd/draft/FEAT-002.md` — Multi-LLM (Agent Card puede referenciar capabilities de LLM)
