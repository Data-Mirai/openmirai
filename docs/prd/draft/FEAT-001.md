# FEAT-001 — MVP Features: Engine Production-Ready

**Estado**: Aprobado
**Fecha**: 2026-05-05
**Epic**: EPIC-010

---

## Problem Statement

**Tipo**: Feature nueva (9 capacidades core + session control)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora con control total.

Data Mirai Engine tiene core funcional (267 tests) pero le faltan features fundamentales para competir con LangGraph, Google ADK y Anthropic Managed Agents. Sin estas capacidades el engine no puede ir a produccion.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Ver progreso de ejecucion en tiempo real (streaming)
2. Pausar ejecucion en vivo, inspeccionar/modificar state, reanudar (session control)
3. Colocar nodos de decision humana obligatoria al disenar el agente (logic/human_input)
4. Rewind a cualquier paso y fork desde ahi (checkpointing)
5. Extender comportamiento sin tocar codigo core (hooks)
6. Desplegar versiones inmutables con rollback (versionamiento)
7. Almacenar credenciales seguras para recursos y herramientas (vault)
8. Monitorear con metricas OpenTelemetry (telemetria)
9. Describir agente en lenguaje natural y obtener grafo generado (AI assistant)
10. Consumir tools de MCP servers estandar (MCP client)

---

## Entidades nuevas

### checkpoint
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK session, NOT NULL |
| step_number | INTEGER | NOT NULL |
| node_id | TEXT | NOT NULL |
| state_snapshot | JSON | NOT NULL |
| cursor_position | TEXT | NOT NULL |
| pending_edges | JSON | — |
| created_at | TEXT | NOT NULL |

### interrupt
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK session, NOT NULL |
| checkpoint_id | TEXT | FK checkpoint, NOT NULL |
| node_id | TEXT | NOT NULL |
| interrupt_type | TEXT | NOT NULL (session_control / human_input_block) |
| prompt | JSON | — |
| response | JSON | NULL |
| status | TEXT | PENDING / RESOLVED / EXPIRED / CANCELLED |
| created_at | TEXT | NOT NULL |
| resolved_at | TEXT | NULL |

### agent_snapshot
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agent, NOT NULL |
| version | INTEGER | NOT NULL, auto-increment por agent |
| graph_def | JSON | NOT NULL |
| config | JSON | — |
| tools_config | JSON | — |
| status | TEXT | ACTIVE / DEPRECATED / ROLLED_BACK |
| created_at | TEXT | NOT NULL |

### vault
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL, UNIQUE |
| description | TEXT | NULL |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### credential
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| vault_id | TEXT | FK vault, NOT NULL |
| name | TEXT | NOT NULL |
| credential_type | TEXT | NOT NULL (api_key / static_bearer / oauth) |
| encrypted_value | TEXT | NOT NULL |
| oauth_config | JSON | NULL |
| expires_at | TEXT | NULL |
| status | TEXT | ACTIVE / EXPIRED / REVOKED |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### execution_span
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK session, NOT NULL |
| node_id | TEXT | NOT NULL |
| span_type | TEXT | NOT NULL (block_exec / llm_call / tool_call / hook) |
| started_at | TEXT | NOT NULL |
| ended_at | TEXT | NULL |
| duration_ms | INTEGER | NULL |
| tokens_input | INTEGER | NULL |
| tokens_output | INTEGER | NULL |
| cost_estimate | REAL | NULL |
| status | TEXT | success / error / skipped |
| error_message | TEXT | NULL |
| attributes | JSON | NULL |

---

## Maquinas de estado

### Session (actualizada)
```
pending → running → completed / failed / timeout
running ↔ interrupted
interrupted → cancelled
```

### Interrupt
```
PENDING → RESOLVED / EXPIRED / CANCELLED
```

### Agent Snapshot
```
ACTIVE → DEPRECATED / ROLLED_BACK
DEPRECATED → ACTIVE (rollback)
```
Invariante: max 1 ACTIVE por agent_id.

### Credential
```
ACTIVE → EXPIRED → ACTIVE (refresh)
ACTIVE → REVOKED
EXPIRED → REVOKED
```

---

## Reglas de negocio nuevas

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-12 | Cada bloque completado genera exactamente un checkpoint | GraphRunner |
| REGLA-13 | Checkpoints inmutables. Fork/rewind crean nueva session | DB (no UPDATE) |
| REGLA-14 | Pause solo en running. Espera que bloque actual termine | API guard |
| REGLA-15 | State editable solo en interrupted. Se notifica al usuario que el flujo se pausa, hace cambios, guarda, reanuda | API guard |
| REGLA-16 | Max 1 interrupt PENDING por session | Service |
| REGLA-17 | Respuesta de interrupt se inyecta como output del nodo | GraphRunner |
| REGLA-18 | Exactamente 1 snapshot ACTIVE por agente | Service (transaccion atomica) |
| REGLA-19 | Sessions usan snapshot, no draft | Service |
| REGLA-20 | Valores de credenciales nunca en responses API | Serializer |
| REGLA-21 | Credenciales cifradas en reposo. Sin clave configurada, vault no opera — UI muestra wizard | Service + UI |
| REGLA-22 | Hooks timeout 30s. Si excede, se trata como continue | GraphRunner |
| REGLA-23 | Hook abort = session falla | GraphRunner |
| REGLA-24 | Un nodo usa UN tool (builtin o MCP), no hibridos | Graph validation |
| REGLA-25 | Telemetria async, nunca bloquea ejecucion | Service (fire-and-forget) |

### Reglas actualizadas

**REGLA-02** (actualizada): Output de bloque → SharedState[node_id]. Inmutable durante ejecucion normal. Modificable unicamente durante session_control interrupt: flujo se pausa → usuario es notificado → hace cambios → guarda → reanuda con cambios aplicados. Se crea nuevo checkpoint.

**REGLA-08** (actualizada): Memory short_term (transcript) en RAM durante session → persiste a Postgres al terminar. Checkpoints son sistema aparte: snapshots del SharedState (JSON completo de outputs acumulados) en cada paso. Los resultados del agente son diferentes al snapshot de todo lo que hizo el agente para llegar al resultado.

---

## Contratos API

### Streaming
- `GET /api/sessions/{id}/stream` — SSE con eventos tipados (session.*, block.*, llm.*, checkpoint.*, hook.*, interrupt.*)

### Session Control
- `POST /api/sessions/{id}/pause` — pausar session running
- `POST /api/sessions/{id}/resume` — reanudar session interrupted
- `GET /api/sessions/{id}/state` — inspeccionar SharedState
- `PATCH /api/sessions/{id}/state` — modificar state (solo en interrupted)

### Checkpoints
- `GET /api/sessions/{id}/checkpoints` — listar checkpoints
- `POST /api/sessions/{id}/rewind` — rewind a paso N (crea nueva session)
- `POST /api/sessions/{id}/fork` — fork desde paso N con overrides

### Interrupts
- `GET /api/sessions/{id}/interrupts` — listar interrupts
- `POST /api/interrupts/{id}/resolve` — resolver con respuesta
- `POST /api/interrupts/{id}/cancel` — cancelar

### Versionamiento
- `POST /api/agents/{id}/publish` — crear snapshot
- `GET /api/agents/{id}/versions` — listar versiones
- `GET /api/agent-snapshots/{id}` — detalle snapshot
- `POST /api/agents/{id}/rollback` — rollback a version N

### Vault
- `POST /api/vaults` / `GET /api/vaults` / `GET /api/vaults/{id}` / `DELETE /api/vaults/{id}`
- `POST /api/vaults/{id}/credentials` / `GET /api/vaults/{id}/credentials` / `PATCH /api/credentials/{id}` / `DELETE /api/credentials/{id}`

### Telemetria
- `GET /api/sessions/{id}/spans` — spans de una session
- `GET /api/agents/{id}/metrics` — metricas agregadas

### AI Assistant
- `POST /api/agents/generate` — generar grafo desde descripcion natural
- `GET /api/agents/templates` — listar templates

### MCP
- `POST /api/mcp-servers` / `GET /api/mcp-servers` / `DELETE /api/mcp-servers/{id}`
- `GET /api/mcp-servers/{id}/tools` — descubrir tools
- `POST /api/mcp-servers/{id}/test` — probar conexion

---

## Pantallas

### Modificadas
- **SessionDetail** (`/sessions/[id]`): tabs Live (streaming), Checkpoints, Spans. Controles pause/resume. Panel interrupt. State inspector
- **AgentDetail** (`/agents/[id]`): tabs Versions, Metrics. Secciones Hooks config, MCP servers
- **EditorCanvas**: MCP tools en catalogo, bloque logic/human_input, boton "Crear con IA"
- **Settings** (`/settings`): seccion telemetria config

### Nuevas
- **VaultPage** (`/vault`): gestion de vaults y credenciales con wizard

---

## Notas de implementacion

- **MCP conexion**: lazy por nodo. Se conecta cuando el cursor llega al bloque que usa tool de ese MCP server
- **Vault sin keys**: UI muestra que recurso necesita configuracion. Wizard guiado. Recurso marcado "incompleto" hasta configurar
- **AI Assistant**: agente built-in "graph-builder" que usa ai/llm_call con ToolRegistry como contexto para generar GraphDef
- **logic/human_input**: tool #17 en categoria Logic del catalogo

---

## doc_refs
- `docs/producto/DOMINIO.md` — actor, roles
- `docs/producto/FLUJOS.md` — maquinas de estado, reglas
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
