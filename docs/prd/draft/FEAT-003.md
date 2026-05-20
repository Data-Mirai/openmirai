# FEAT-003 — Tier 2: Auto-mejora (Tracer + Reflector + Playbook + Graph Self-Improvement)

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-021

---

## Problem Statement

**Tipo**: Feature nueva (4 capacidades de auto-mejora)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora con control total.

El engine ejecuta grafos pero no aprende de sus ejecuciones. Cada ejecucion es independiente — no hay forma de detectar patrones de exito/fallo, ni de mejorar prompts o configuraciones basandose en historial. El usuario tiene que optimizar manualmente cada nodo revisando logs crudos. Hoy tenemos `execution_span` (FEAT-001) que captura telemetria basica (duracion, tokens, status), pero no captura que inputs recibio cada nodo, que output produjo, ni si ese output fue util para el siguiente nodo.

La competencia (Hive Agents con ACE, Hermes Agent con self-improvement loop) ofrece mecanismos de auto-mejora. Sin esta capacidad, el usuario de Data Mirai tiene una desventaja operativa significativa: debe optimizar a mano lo que otras plataformas hacen de forma asistida.

---

## Objetivo

Cuando esto este implementado:
1. Cada ejecucion de grafo genera trazas detalladas automaticamente (sin agregar latencia al pipeline)
2. El sistema analiza trazas periodicamente y detecta patrones de exito, fallo y oportunidades de mejora
3. Los patrones se convierten en reglas inyectables (playbook) que mejoran futuras ejecuciones de nodos llm_call
4. El Designer Assistant sugiere mejoras estructurales al grafo basandose en reflexiones y reglas, y el usuario aprueba antes de que se aplique cualquier cambio

---

## Features

### 2.1 — Execution Tracer

**Problema**: `execution_span` (FEAT-001) captura telemetria basica: duracion, tokens, status, cost_estimate. Falta capturar que inputs recibio cada nodo, que output produjo, el data_map que alimento esos inputs, patrones de error recurrentes, y cuantos retries necesito. Sin esos datos, es imposible analizar automaticamente por que un grafo funciona bien o mal.

**Solucion**: Extender el sistema de trazas con un `ExecutionTracer` pasivo que captura datos ricos despues de cada ejecucion de nodo, sin agregar latencia al pipeline. El Tracer es un hook `post_block_exec` del GraphRunner — fire-and-forget.

**Arquitectura**:
- `ExecutionTracer` se registra como hook `post_block_exec` en el GraphRunner
- Captura despues de cada nodo: node_id, tool_type, inputs (truncados a 2000 chars), output (truncado a 2000 chars), duration_ms, tokens, status (success/error/skip), error_type si fallo, retry_count, el data_map que alimento los inputs
- Escritura a tabla `execution_trace` en SQLite via fire-and-forget (asyncio.create_task)
- No bloquea ejecucion — si la escritura falla, se pierde la traza pero el grafo continua
- Retencion configurable en `app_config`: `trace_retention_days` (default 90). Job de poda se ejecuta al iniciar el server y cada 24h

**Entidades nuevas**:

Tabla `execution_trace`:
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK sessions, NOT NULL |
| agent_id | TEXT | FK agents, NOT NULL |
| node_id | TEXT | NOT NULL |
| tool_type | TEXT | NOT NULL |
| inputs_snapshot | TEXT | NULL (JSON, truncado a 2000 chars) |
| output_snapshot | TEXT | NULL (JSON, truncado a 2000 chars) |
| data_map_used | TEXT | NULL (JSON) |
| duration_ms | INTEGER | NULL |
| tokens_input | INTEGER | NULL |
| tokens_output | INTEGER | NULL |
| status | TEXT | NOT NULL (success/error/skip) |
| error_type | TEXT | NULL |
| error_message | TEXT | NULL |
| retry_count | INTEGER | DEFAULT 0 |
| created_at | TEXT | NOT NULL |

Indices:
- `idx_trace_session ON execution_trace(session_id)`
- `idx_trace_agent ON execution_trace(agent_id)`
- `idx_trace_agent_status ON execution_trace(agent_id, status)`
- `idx_trace_created ON execution_trace(created_at)` (para poda por retencion)

**Contratos API**:
- `GET /api/agents/{id}/traces` — trazas paginadas. Query params: `node_id`, `status`, `from_date`, `to_date`, `page`, `per_page` (default 50)
- `GET /api/sessions/{id}/traces` — trazas de una sesion especifica, ordenadas por created_at
- `GET /api/agents/{id}/traces/stats` — estadisticas agregadas: success rate por nodo, duracion promedio por nodo, top 5 errores recurrentes, total trazas

**Pantallas**:
- **AgentDetail** (`/agents/[id]`) → nuevo tab "Trazas":
  - Tabla de trazas con columnas: nodo, tool_type, status (badge color), duracion, tokens, fecha
  - Filtros: por nodo (dropdown), por status (success/error/skip), por rango de fechas
  - Graficas: success rate por nodo (bar chart horizontal), duracion promedio por nodo
  - Click en fila expande: inputs_snapshot, output_snapshot, data_map_used, error_message
- **SessionDetail** (`/sessions/[id]`) → seccion de spans mejorada:
  - Cada span muestra inputs/outputs truncados expandibles

**Reglas**:
| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-40 | Tracer NUNCA bloquea ejecucion. Es fire-and-forget via asyncio.create_task | Hook implementation en runner.py |
| REGLA-41 | Inputs/outputs truncados a 2000 chars maximo antes de persistir | ExecutionTracer._truncate() |
| REGLA-42 | Trazas se podan automaticamente despues de trace_retention_days | Background task en app.py startup |

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/__init__.py`
- CREAR `framework/src/datamirai_engine/intelligence/tracer.py` — clase ExecutionTracer con metodos: `capture(session_id, agent_id, node_id, tool_type, inputs, output, data_map, duration_ms, tokens_in, tokens_out, status, error_type, error_msg, retry_count)`, `_truncate(data, max_chars=2000)`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — en `post_block_exec` hook y en error path, invocar Tracer si esta configurado
- MODIFICAR `app/server/datamirai_app/database.py` — tabla execution_trace + ExecutionTraceRepo con metodos: `create()`, `list_by_agent(agent_id, filters)`, `list_by_session(session_id)`, `get_stats(agent_id)`, `prune(retention_days)`
- CREAR `app/server/datamirai_app/routes/traces.py` — endpoints GET
- MODIFICAR `app/server/datamirai_app/app.py` — registrar router de trazas, startup task de poda
- MODIFICAR `app/web/src/lib/api.ts` — funciones fetchAgentTraces, fetchSessionTraces, fetchTraceStats
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — tab Trazas
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — inputs/outputs en spans

---

### 2.2 — Reflector

**Problema**: Las trazas se acumulan pero nadie las analiza. El usuario tendria que revisar manualmente cientos de trazas para encontrar patrones. "El nodo N3 falla el 40% de las veces con error de timeout" o "el prompt del nodo N5 funciona mejor cuando recibe contexto largo" son insights que requieren analisis estadistico + interpretacion semantica que un humano no va a hacer.

**Solucion**: Un `Reflector` que periodicamente analiza lotes de trazas usando el LLM configurado del agente y genera "reflexiones" — insights estructurados sobre patrones de exito, fallos recurrentes y oportunidades de mejora.

**Arquitectura**:
- `Reflector` se ejecuta cuando se acumulan N trazas nuevas (configurable en `app_config`: `reflect_every_n_traces`, default 20) o manualmente via API
- Toma un batch de trazas recientes de un agente (max 50 por batch — REGLA-45)
- Construye un prompt estructurado con las trazas serializadas y lo envia al LLM del agente:
  - "Analiza estas {n} trazas de ejecucion del agente '{name}'. Genera insights sobre: 1) patrones de exito (que nodos/configuraciones funcionan consistentemente), 2) fallos recurrentes (mismos errores repetidos), 3) prompts que podrian mejorarse (basado en outputs pobres), 4) data_maps problematicos (inputs que faltan o no se usan), 5) anomalias (comportamiento inesperado)"
- El LLM retorna JSON con array de reflexiones, cada una con: type, node_id (si aplica), insight, confidence (0.0-1.0)
- Se guardan en tabla `reflection`
- Internamente, el Reflector lleva un contador de trazas nuevas por agente desde la ultima reflexion. Al alcanzar el umbral, ejecuta automaticamente al proximo request del agente (no background — se ejecuta como parte del request POST /reflect o se triggerea lazy)

**Entidades nuevas**:

Tabla `reflection`:
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents, NOT NULL |
| reflection_type | TEXT | NOT NULL (success_pattern / failure_pattern / optimization / anomaly) |
| node_id | TEXT | NULL (si aplica a nodo especifico) |
| insight | TEXT | NOT NULL |
| confidence | REAL | NOT NULL (0.0 a 1.0) |
| trace_ids | TEXT | NOT NULL (JSON array de IDs de trazas analizadas) |
| status | TEXT | DEFAULT 'active' (active / applied / dismissed) |
| created_at | TEXT | NOT NULL |

Indices:
- `idx_reflection_agent ON reflection(agent_id)`
- `idx_reflection_agent_status ON reflection(agent_id, status)`

Tabla `reflection_counter` (control interno):
| Columna | Tipo | Constraint |
|---|---|---|
| agent_id | TEXT | PK |
| traces_since_last | INTEGER | DEFAULT 0 |
| last_reflected_at | TEXT | NULL |

**Contratos API**:
- `GET /api/agents/{id}/reflections` — listar reflexiones. Query params: `status` (active/applied/dismissed), `type` (success_pattern/failure_pattern/optimization/anomaly), `page`, `per_page`
- `POST /api/agents/{id}/reflect` — ejecutar reflexion manual. Retorna las reflexiones generadas. Si no hay trazas nuevas suficientes, retorna 400 con mensaje
- `PATCH /api/reflections/{id}` — actualizar status (applied/dismissed). Body: `{"status": "dismissed"}`
- `GET /api/agents/{id}/reflections/summary` — resumen: count por tipo, count activas, ultima reflexion, top insights por confidence

**Pantallas**:
- **AgentDetail** (`/agents/[id]`) → nuevo tab "Insights":
  - Header: resumen (N activas, ultima reflexion hace X, boton "Analizar ahora")
  - Lista de reflexiones ordenadas por confidence desc:
    - Badge de tipo (success_pattern=verde, failure_pattern=rojo, optimization=amarillo, anomaly=morado)
    - Badge de confidence (alta >0.7 = solido, media 0.5-0.7 = outline, baja <0.5 = dashed + label "baja confianza")
    - Texto del insight
    - Nodo afectado (si aplica, con link al nodo en el editor)
    - Expandible: lista de trace_ids con mini-preview de cada traza
    - Acciones: "Promover a regla" (→ 2.3), "Descartar"
  - Filtros: por tipo, por status

**Reglas**:
| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-43 | Reflector usa el LLM configurado del agente (model + provider del agent config). Si no hay LLM configurado, retorna error 400 | Reflector + API route |
| REGLA-44 | Reflexiones con confidence < 0.5 se muestran con label "baja confianza" en UI | Frontend |
| REGLA-45 | Max 50 trazas por batch de reflexion para no exceder context window | Reflector._build_batch() |

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/reflector.py` — clase Reflector con metodos: `reflect(agent_id, traces, llm_adapter) -> list[Reflection]`, `_build_prompt(traces, agent_name) -> str`, `_parse_response(llm_output) -> list[dict]`
- MODIFICAR `app/server/datamirai_app/database.py` — tablas reflection + reflection_counter, ReflectionRepo con metodos: `create()`, `list_by_agent(agent_id, filters)`, `update_status(id, status)`, `get_summary(agent_id)`, `increment_counter(agent_id)`, `reset_counter(agent_id)`, `get_counter(agent_id)`
- CREAR `app/server/datamirai_app/routes/reflections.py` — endpoints
- MODIFICAR `app/server/datamirai_app/app.py` — registrar router
- MODIFICAR `app/web/src/lib/api.ts` — funciones fetchReflections, triggerReflection, updateReflection, fetchReflectionSummary
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — tab Insights

---

### 2.3 — Playbook (Reglas Aprendidas)

**Problema**: Las reflexiones son insights pasivos — el usuario las lee pero no se integran en la ejecucion. Si el Reflector detecta "el nodo N3 funciona mejor cuando le pasas contexto estructurado", esa informacion no se aplica automaticamente. El usuario tiene que ir al editor, abrir el nodo, y cambiar el prompt manualmente.

**Solucion**: Las reflexiones de alta confianza se pueden "promover" a reglas del playbook — instrucciones que se inyectan automaticamente en el system prompt de nodos `ai/llm_call` para mejorar la ejecucion. Con feedback loop: si la regla ayuda, sube; si perjudica, se desactiva sola.

**Arquitectura**:
- `PlaybookManager` mantiene reglas activas por agente
- Cada regla tiene: texto, tipo (optimization/guardrail/preference), node_filter (si aplica a tool_type o nodo especifico), helpful_count, harmful_count
- Inyeccion: antes de cada `ai/llm_call`, el runner (via hook `pre_llm_call`) consulta PlaybookManager por reglas relevantes. PlaybookManager busca por FTS (full-text search) contra el prompt del nodo + filtro por node_id/tool_type. Retorna max 10 reglas (REGLA-46). El hook las inyecta como bloque `[PLAYBOOK RULES]` al inicio del system prompt
- Feedback loop: despues de cada `ai/llm_call` que tenia reglas inyectadas, el hook `post_block_exec` registra que reglas se usaron. Si el nodo tuvo status=success → helpful_count++ para cada regla usada. Si status=error → harmful_count++
- Auto-desactivacion: si harmful_count > helpful_count para una regla, se marca como `disabled` automaticamente (REGLA-47). Excepcion: reglas manuales (sin reflection_id) nunca se auto-desactivan (REGLA-48)

**Entidades nuevas**:

Tabla `playbook_rule`:
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents, NOT NULL |
| reflection_id | TEXT | NULL (FK reflection, NULL si es regla manual) |
| rule_text | TEXT | NOT NULL |
| rule_type | TEXT | NOT NULL (optimization / guardrail / preference) |
| node_filter | TEXT | NULL (node_id o tool_type especifico, NULL = aplica a todos los llm_call) |
| helpful_count | INTEGER | DEFAULT 0 |
| harmful_count | INTEGER | DEFAULT 0 |
| status | TEXT | DEFAULT 'active' (active / disabled / archived) |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Indices:
- `idx_playbook_agent ON playbook_rule(agent_id)`
- `idx_playbook_agent_status ON playbook_rule(agent_id, status)`

Virtual table (FTS5):
- `playbook_rule_fts USING fts5(rule_text, content=playbook_rule, content_rowid=rowid)`
- Triggers para mantener FTS sincronizado con inserts/updates/deletes

Tabla `playbook_application` (registro de que reglas se aplicaron a que ejecucion):
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| trace_id | TEXT | FK execution_trace, NOT NULL |
| rule_id | TEXT | FK playbook_rule, NOT NULL |
| outcome | TEXT | NULL (success / error — se llena post-ejecucion) |
| created_at | TEXT | NOT NULL |

Indice:
- `idx_application_trace ON playbook_application(trace_id)`
- `idx_application_rule ON playbook_application(rule_id)`

**Contratos API**:
- `GET /api/agents/{id}/playbook` — listar reglas del playbook. Query params: `status`, `type`, `page`, `per_page`
- `POST /api/agents/{id}/playbook` — crear regla manual. Body: `{"rule_text": "...", "rule_type": "optimization", "node_filter": null}`
- `POST /api/reflections/{id}/promote` — promover reflexion a regla. Crea playbook_rule con reflection_id vinculado. Marca reflexion como status=applied
- `PATCH /api/playbook/{id}` — editar regla. Body: campos editables (rule_text, rule_type, node_filter, status)
- `DELETE /api/playbook/{id}` — eliminar regla (hard delete, cascadea applications)
- `POST /api/playbook/{id}/feedback` — reportar helpful/harmful manualmente. Body: `{"outcome": "helpful"}` o `{"outcome": "harmful"}`

**Pantallas**:
- **AgentDetail** (`/agents/[id]`) → nuevo tab "Playbook":
  - Header: N reglas activas, N desactivadas automaticamente
  - Boton "Crear regla manual" → modal con: textarea para rule_text, select para rule_type, input opcional para node_filter
  - Lista de reglas:
    - Badge de tipo (optimization=azul, guardrail=rojo, preference=verde)
    - Texto de la regla
    - Metricas: helpful (thumbs up + count), harmful (thumbs down + count), ratio
    - Status badge: active=verde, disabled=gris con tooltip "desactivada automaticamente: mas fallos que exitos"
    - Acciones: Editar, Eliminar, Reactivar (si disabled)
    - Si tiene reflection_id: link "Ver insight original"
  - En tab Insights: cada reflexion activa tiene boton "Promover a regla" que abre modal de confirmacion con preview

**Reglas**:
| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-46 | Max 10 reglas activas inyectadas por turno de llm_call | PlaybookManager.get_relevant_rules(max=10) |
| REGLA-47 | Reglas con harmful_count > helpful_count se desactivan automaticamente | PlaybookManager.check_auto_disable() ejecutado en cada feedback |
| REGLA-48 | Reglas manuales (reflection_id IS NULL) nunca se auto-desactivan | PlaybookManager.check_auto_disable() filtra por reflection_id |

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/playbook.py` — clase PlaybookManager con metodos: `get_relevant_rules(agent_id, node_id, tool_type, prompt_text, max=10) -> list[Rule]`, `record_feedback(rule_id, outcome)`, `check_auto_disable(rule_id)`, `inject_rules(prompt, rules) -> str`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — en hooks `pre_llm_call` y `post_block_exec`, integrar PlaybookManager si esta configurado
- MODIFICAR `app/server/datamirai_app/database.py` — tablas playbook_rule, playbook_rule_fts, playbook_application. PlaybookRuleRepo con metodos: `create()`, `list_by_agent(agent_id, filters)`, `update(id, kwargs)`, `delete(id)`, `record_feedback(id, outcome)`, `search_fts(query, agent_id)`, `promote_from_reflection(reflection_id)`. PlaybookApplicationRepo con metodos: `create(trace_id, rule_id)`, `update_outcome(trace_id, outcome)`
- CREAR `app/server/datamirai_app/routes/playbook.py` — endpoints
- MODIFICAR `app/server/datamirai_app/app.py` — registrar router
- MODIFICAR `app/web/src/lib/api.ts` — funciones fetchPlaybook, createRule, promoteReflection, updateRule, deleteRule, sendFeedback
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — tab Playbook

---

### 2.4 — Graph Self-Improvement via Designer Assistant

**Problema**: Reflexiones y playbook mejoran la ejecucion en runtime (inyectan contexto al LLM), pero no mejoran la estructura del grafo. Si un nodo necesita un prompt completamente diferente, si falta un nodo de validacion entre dos pasos, o si un data_map no esta conectando los campos correctos, el usuario tiene que descubrirlo solo revisando insights y aplicando cambios manualmente en el editor.

**Solucion**: Extender el Designer Assistant (FEAT-001 — AI assistant) con un modo "mejora" que consume reflexiones activas y reglas del playbook, analiza el grafo actual, y genera sugerencias concretas de cambios que el usuario puede aceptar o rechazar individualmente.

**INVARIANTE CRITICO**: El grafo NUNCA se modifica automaticamente. El asistente SUGIERE, el usuario APRUEBA. Sin excepcion.

**Arquitectura**:
- Extender el endpoint existente `POST /api/agents/generate` (o crear uno nuevo dedicado) para modo "improvement"
- Input al LLM: reflexiones activas + reglas del playbook (con sus metricas) + grafo actual del agente (GraphDef completo con nodos, edges, configs) + historial resumido de trazas (stats por nodo)
- El LLM analiza y genera sugerencias concretas. Cada sugerencia es un `GraphSuggestion` tipado:
  - `prompt_change`: cambiar el prompt/config de un nodo existente. Propone nuevo valor
  - `add_node`: agregar un nodo nuevo (tipo, config, posicion en el grafo, edges nuevas)
  - `modify_config`: cambiar temperatura, max_tokens, retry_policy, etc. de un nodo
  - `modify_data_map`: cambiar el data_map de una edge (agregar/quitar campos)
  - `remove_node`: eliminar un nodo y reconectar edges
- Cada sugerencia incluye: tipo, nodo afectado, cambio propuesto (JSON del cambio concreto), razon en lenguaje natural, confidence
- Flujo UI:
  1. Usuario hace click en "Analizar y sugerir mejoras" en toolbar del editor
  2. Llamada a API que genera sugerencias
  3. Panel lateral muestra sugerencias pendientes con badge de count
  4. Usuario expande sugerencia, ve preview del cambio (diff visual si es prompt, diagrama si es estructura)
  5. Acepta o rechaza individualmente
  6. Al aceptar, se aplica el cambio al grafo en el editor (el usuario aun debe guardar)

**Entidades nuevas**:

Tabla `graph_suggestion`:
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents, NOT NULL |
| suggestion_type | TEXT | NOT NULL (prompt_change / add_node / modify_config / modify_data_map / remove_node) |
| target_node_id | TEXT | NULL (NULL para add_node) |
| description | TEXT | NOT NULL (resumen legible para el usuario) |
| proposed_change | TEXT | NOT NULL (JSON del cambio concreto) |
| reason | TEXT | NOT NULL (explicacion de por que se sugiere, basada en que reflexion/regla) |
| confidence | REAL | NOT NULL (0.0 a 1.0) |
| source_reflection_ids | TEXT | NULL (JSON array) |
| source_rule_ids | TEXT | NULL (JSON array) |
| status | TEXT | DEFAULT 'pending' (pending / accepted / rejected / applied) |
| created_at | TEXT | NOT NULL |

Indices:
- `idx_suggestion_agent ON graph_suggestion(agent_id)`
- `idx_suggestion_agent_status ON graph_suggestion(agent_id, status)`

**Formato de proposed_change por tipo**:

`prompt_change`:
```json
{
  "field": "prompt_template",
  "old_value": "Resumir el siguiente texto: {content}",
  "new_value": "Resumir el siguiente texto en maximo 3 parrafos. Incluir puntos clave al final.\n\nTexto: {content}"
}
```

`add_node`:
```json
{
  "node": {"id": "n_validation", "tool_type": "logic/condition", "config": {...}},
  "insert_after": "n3",
  "insert_before": "n4",
  "edges_to_add": [...],
  "edges_to_remove": [...]
}
```

`modify_config`:
```json
{
  "field": "temperature",
  "old_value": 0.7,
  "new_value": 0.3
}
```

`modify_data_map`:
```json
{
  "edge_source": "n2",
  "edge_target": "n3",
  "old_data_map": {"prompt": "n2.response"},
  "new_data_map": {"prompt": "n2.response", "context": "n1.output.full_text"}
}
```

`remove_node`:
```json
{
  "node_id": "n5",
  "reconnect_from": "n4",
  "reconnect_to": "n6"
}
```

**Contratos API**:
- `POST /api/agents/{id}/suggest-improvements` — generar sugerencias basadas en reflexiones/playbook/stats. Retorna array de sugerencias creadas. Requiere al menos 1 reflexion activa (REGLA-50). Si no hay, retorna 400
- `GET /api/agents/{id}/suggestions` — listar sugerencias. Query params: `status` (pending/accepted/rejected/applied), `page`, `per_page`
- `POST /api/suggestions/{id}/accept` — marcar como accepted
- `POST /api/suggestions/{id}/reject` — marcar como rejected
- `POST /api/suggestions/{id}/apply` — aplicar cambio al grafo. Lee la suggestion, aplica proposed_change al graph_def del agente, guarda grafo actualizado, marca suggestion como applied. Retorna grafo actualizado

**Pantallas**:
- **AgentDesigner** (`/agents/[id]` en modo editor):
  - Toolbar: boton "Sugerir mejoras" (icono lightbulb). Disabled si no hay reflexiones activas. Tooltip: "Analiza reflexiones y reglas para sugerir mejoras al grafo"
  - Panel lateral derecho "Mejoras sugeridas":
    - Badge con count de sugerencias pendientes
    - Lista de sugerencias ordenadas por confidence desc:
      - Icono por tipo (prompt_change=pencil, add_node=plus, modify_config=settings, modify_data_map=link, remove_node=trash)
      - Description (texto legible)
      - Badge de confidence
      - Nodo afectado (highlight en el canvas al hover)
    - Expandir sugerencia muestra:
      - Reason completo
      - Preview del cambio: diff de texto para prompts, JSON formateado para configs, diagrama mini para estructura
      - Reflexiones/reglas fuente (links)
      - Botones: "Aceptar y aplicar", "Rechazar"
    - Al aplicar: el cambio se refleja en el canvas del editor. El usuario debe guardar para persistir

**Reglas**:
| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-49 | NUNCA modificar grafo sin aprobacion explicita del usuario. Cada sugerencia requiere accept + apply | API guards: apply verifica status=accepted |
| REGLA-50 | Sugerencias se generan solo cuando hay al menos 1 reflexion activa | Service: POST suggest-improvements valida count |
| REGLA-51 | Una sugerencia aplicada se marca como 'applied' y no se regenera en futuras llamadas a suggest-improvements | Service: filtro en generacion |

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/suggester.py` — clase GraphSuggester con metodos: `suggest(agent_graph, reflections, rules, trace_stats, llm_adapter) -> list[Suggestion]`, `_build_prompt(graph, reflections, rules, stats) -> str`, `_parse_suggestions(llm_output) -> list[dict]`, `apply_suggestion(graph_def, suggestion) -> dict` (retorna graph_def modificado)
- MODIFICAR `app/server/datamirai_app/database.py` — tabla graph_suggestion, GraphSuggestionRepo con metodos: `create()`, `list_by_agent(agent_id, filters)`, `update_status(id, status)`, `get(id)`
- CREAR `app/server/datamirai_app/routes/suggestions.py` — endpoints
- MODIFICAR `app/server/datamirai_app/app.py` — registrar router
- MODIFICAR `app/web/src/lib/api.ts` — funciones fetchSuggestions, generateSuggestions, acceptSuggestion, rejectSuggestion, applySuggestion
- MODIFICAR `app/web/src/components/agent/AgentDesigner.tsx` — boton "Sugerir mejoras" en toolbar, panel lateral de sugerencias
- MODIFICAR `app/web/src/components/editor/EditorCanvas.tsx` — highlight de nodo al hover sobre sugerencia

---

## Maquinas de estado

### Reflection
```
active → applied (promovida a regla via POST /reflections/{id}/promote)
active → dismissed (descartada por usuario via PATCH /reflections/{id})
```

### Playbook Rule
```
active → disabled (auto-desactivacion por harmful > helpful, REGLA-47)
active → archived (usuario archiva manualmente)
disabled → active (usuario reactiva manualmente)
```
Invariante: reglas manuales (reflection_id IS NULL) nunca transicionan a disabled automaticamente.

### Graph Suggestion
```
pending → accepted (usuario acepta via POST /suggestions/{id}/accept)
pending → rejected (usuario rechaza via POST /suggestions/{id}/reject)
accepted → applied (cambio aplicado al grafo via POST /suggestions/{id}/apply)
```
Invariante: solo sugerencias en status=accepted pueden transicionar a applied.

---

## Dependencias entre features

```
2.1 (Tracer) ← independiente, se implementa primero
2.2 (Reflector) ← depende de 2.1 (necesita trazas para analizar)
2.3 (Playbook) ← depende de 2.2 (reflexiones se promueven a reglas)
2.4 (Graph Improvement) ← depende de 2.2 y 2.3 (consume reflexiones + reglas)
```

Depende de FEAT-001:
- 1.1 (Multi-LLM adapter) — Reflector y Suggester necesitan llamar al LLM configurado del agente
- 1.9 (AI Assistant) — 2.4 extiende el Designer Assistant existente
- 1.8 (Telemetria / execution_span) — 2.1 complementa los spans con datos mas ricos

No depende de FEAT-002 (Tier 1). Puede implementarse en paralelo si FEAT-001 esta completo.

---

## Notas de implementacion

- **Truncado inteligente**: inputs_snapshot y output_snapshot se truncan a 2000 chars. Para JSON, truncar el string serializado, no campos individuales. Agregar `"__truncated": true` al JSON si se trunco
- **FTS5 para Playbook**: SQLite FTS5 permite busqueda full-text eficiente sobre rule_text. Los triggers de sincronizacion (insert/update/delete) mantienen el indice actualizado. No requiere dependencia externa
- **Prompt engineering del Reflector**: el prompt debe ser estructurado y pedir respuesta en JSON estricto. Incluir schema esperado en el prompt. Parsear con try/except y fallback a reflexion generica si el LLM no retorna JSON valido
- **Prompt engineering del Suggester**: incluir el GraphDef completo serializado + reflexiones + reglas. Si excede context window, priorizar reflexiones de alta confianza y reglas activas con mejor ratio helpful/harmful
- **Feedback automatico vs manual**: el feedback loop del Playbook es automatico (basado en status del nodo post-ejecucion). El endpoint POST /playbook/{id}/feedback es para override manual del usuario
- **No background jobs persistentes**: la poda de trazas y el check de reflexion pendiente se ejecutan como startup tasks del server FastAPI, no como procesos separados. Mantiene la arquitectura simple de un solo proceso

---

## Reglas de negocio nuevas (resumen)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-40 | Tracer NUNCA bloquea ejecucion | asyncio.create_task fire-and-forget |
| REGLA-41 | Inputs/outputs truncados a 2000 chars max | ExecutionTracer._truncate() |
| REGLA-42 | Trazas se podan por retencion configurada | Startup task + periodic |
| REGLA-43 | Reflector usa LLM del agente | Reflector |
| REGLA-44 | Reflexiones confidence < 0.5 con label "baja confianza" | Frontend |
| REGLA-45 | Max 50 trazas por batch de reflexion | Reflector._build_batch() |
| REGLA-46 | Max 10 reglas inyectadas por turno llm_call | PlaybookManager |
| REGLA-47 | Reglas harmful > helpful se auto-desactivan | PlaybookManager |
| REGLA-48 | Reglas manuales nunca se auto-desactivan | PlaybookManager |
| REGLA-49 | Grafo NUNCA se modifica sin aprobacion del usuario | API guard |
| REGLA-50 | Sugerencias requieren al menos 1 reflexion activa | Service validation |
| REGLA-51 | Sugerencia aplicada no se regenera | Service filter |

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md`
- `docs/prd/draft/FEAT-001.md`
- `docs/ARCHITECTURE.md`
