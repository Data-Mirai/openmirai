# FEAT-004 — Tier 3: Autonomia (Context Compiler + Memory Flush + Heartbeat + Resume + Sub-grafos)

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-022

---

## Problem Statement

**Tipo**: Feature nueva (5 capacidades de autonomia)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora con control total.

Hoy los nodos llm_call reciben contexto basico (data_map directo). No hay ensamblaje inteligente de contexto, no hay proteccion contra desbordamiento de context window, los triggers son solo cron fijos sin evaluacion de condiciones, no hay resume real desde checkpoints, y no se pueden componer agentes (un agente ejecutando otro).

---

## Objetivo

Cuando esto este implementado:
1. Cada llm_call recibe contexto ensamblado inteligentemente (reglas del playbook, memoria relevante, historial comprimido)
2. Grafos largos auto-persisten datos criticos antes de que el contexto se desborde
3. Triggers pueden evaluar condiciones antes de ejecutar (heartbeat)
4. Si un grafo falla a mitad, puede retomar desde el ultimo checkpoint exitoso
5. Un grafo puede ejecutar otro agente como un nodo mas (sincrono)

---

## Entidades nuevas

### heartbeat_state
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agent, NOT NULL |
| last_evaluated_at | TEXT | NULL |
| last_triggered_at | TEXT | NULL |
| consecutive_skips | INTEGER | DEFAULT 0 |
| status | TEXT | DEFAULT 'active' (active/paused) |

### Columnas nuevas en tablas existentes

**session** (3 columnas):
| Columna | Tipo | Constraint |
|---|---|---|
| memory_flushed_at | TEXT | NULL |
| resumed_from_session_id | TEXT | NULL |
| resumed_from_checkpoint_id | TEXT | NULL |
| parent_session_id | TEXT | NULL |

---

## Maquinas de estado

### Heartbeat State
```
active → paused → active
```
Invariante: un solo heartbeat_state por agent_id.

---

## Reglas de negocio nuevas

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-55 | Context Compiler NUNCA excede el context window del modelo | Compiler (trunca fases) |
| REGLA-56 | Session Context (fase 3) siempre se incluye — es la data del grafo | Compiler |
| REGLA-57 | Si Playbook o Memory no estan habilitados, esas fases se saltan | Config |
| REGLA-58 | Max 1 flush por session | Flag en session |
| REGLA-59 | Flush NO bloquea la ejecucion del nodo actual — se ejecuta entre nodos | Hook timing |
| REGLA-60 | Si no hay LLM configurado, flush se salta (no hay forma de generar resumen) | Guard |
| REGLA-61 | Heartbeat NO ejecuta grafo si la condicion es falsa | Trigger logic |
| REGLA-62 | Heartbeat respeta quiet_hours | Evaluator |
| REGLA-63 | Si la evaluacion de condicion falla, se trata como false (no ejecuta) | Error handling |
| REGLA-64 | Resume SIEMPRE crea session nueva. Nunca modifica la original | Service |
| REGLA-65 | State overrides se validan contra el schema de outputs del checkpoint | Service |
| REGLA-66 | Una session puede ser resumida multiples veces (cada una crea session nueva) | Service |
| REGLA-67 | Sub-agente NO accede al SharedState del padre. Solo inputs explicitos | GraphRunner aislado |
| REGLA-68 | Max nesting depth: 3 niveles (agente → sub-agente → sub-sub-agente) | Runner guard |
| REGLA-69 | Si sub-agente falla, el nodo run_agent falla (aplica retry policy del nodo) | Error propagation |
| REGLA-70 | Circular reference prohibida: agente A no puede ejecutar agente que directa o indirectamente ejecute A | Graph validation |

---

## Features

### 3.1 — Context Compiler para LLM Nodes

Inspirado en Hive (6 fases) y OpenClaw (context assembly pipeline).

**Problema**: Hoy el nodo llm_call recibe solo lo que viene del data_map. No tiene acceso a reglas aprendidas, memoria de largo plazo, ni historial comprimido. El contexto es estatico y limitado.

**Solucion**: Un Context Compiler que se ejecuta antes de cada llm_call y ensambla el contexto optimo en 5 fases.

**Arquitectura**:
- `ContextCompiler` invocado por el nodo `ai/llm_call` antes de llamar al LLM adapter
- 5 fases de ensamblaje (en orden):
  1. **Agent Identity**: system prompt del agente (personalidad, rol, limites) — siempre presente
  2. **Playbook Rules**: busqueda FTS en playbook del agente, max 5 reglas relevantes al contexto actual del nodo
  3. **Session Context**: datos del data_map (outputs de nodos previos) — lo que ya funciona hoy
  4. **Long-term Memory**: busqueda semantica en memoria del agente, top 3 memorias relevantes
  5. **Compression**: si el total excede 60% del context window del modelo, comprimir historial preservando ultimos datos
- Token budgeting: cada fase tiene un budget maximo. Si se excede, se trunca la fase (no se eliminan fases completas)
- El compiler retorna un prompt ensamblado listo para enviar al LLM

**Entidades nuevas**: Ninguna (usa tablas existentes de playbook_rule, agent_memory)

**Configuracion en agent config**:
```json
{
  "context_compiler": {
    "enable_playbook": true,
    "enable_memory": true,
    "max_playbook_rules": 5,
    "max_memory_results": 3,
    "compression_threshold": 0.6,
    "identity_prompt": "Eres un agente de analisis de datos..."
  }
}
```

**Contratos API**: No nuevos (se configura como parte del agente)

**Pantallas**:
- AgentDesigner → seccion "Context Compiler" en config del agente: toggles para cada fase, identity prompt editable

**Reglas**: REGLA-55, REGLA-56, REGLA-57

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/context_compiler.py`
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/ai/llm_call.py` — invocar compiler antes de llamar adapter
- MODIFICAR agente config schema

---

### 3.2 — Memory Flush Automatico

Inspirado en OpenClaw (memory flush cuando tokens se acercan al limite).

**Problema**: En grafos largos con muchos nodos llm_call encadenados, el contexto acumulado crece. Si excede el context window, se pierde informacion. Hoy no hay mecanismo para auto-persistir datos criticos.

**Solucion**: Cuando el contexto acumulado de una session se acerca al limite, el engine auto-persiste un resumen de la session en curso a LongTermMemory antes de comprimir.

**Arquitectura**:
- `MemoryFlusher` como hook pre-block que monitorea token count acumulado
- Trigger: `tokens_acumulados >= context_window x flush_threshold` (default 0.75)
- Al activarse:
  1. Genera resumen de la session hasta el momento (usa LLM)
  2. Guarda resumen en LongTermMemory del agente
  3. El Context Compiler puede comprimir el historial porque los datos criticos ya estan persistidos
- Solo un flush por session (flag `memory_flushed_at` en session)
- El flush es sincrono pero rapido (un solo LLM call para generar resumen)

**Entidades nuevas**: Columna `memory_flushed_at` en tabla session (NULL = no flushed)

**Contratos API**: No nuevos (es automatico)

**Pantallas**: Sin cambios de UI (es mecanismo interno del engine)

**Reglas**: REGLA-58, REGLA-59, REGLA-60

**Archivos**:
- CREAR `framework/src/datamirai_engine/intelligence/memory_flusher.py`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — registrar hook
- MODIFICAR `app/server/datamirai_app/database.py` — columna memory_flushed_at

---

### 3.3 — Heartbeat Trigger

Inspirado en OpenClaw (heartbeat vs cron).

**Problema**: El trigger/schedule es cron fijo: ejecuta el grafo cada N minutos sin importar si hay trabajo. No evalua condiciones.

**Solucion**: Nuevo trigger `trigger/heartbeat` que periodicamente evalua una condicion y solo ejecuta el grafo si la condicion es verdadera.

**Arquitectura**:
- Nuevo tool `trigger/heartbeat` en el catalogo de tools
- Configuracion:
  - `interval`: frecuencia de evaluacion (ej: "30m", "1h", "6h")
  - `condition_type`: tipo de condicion (db_query / api_check / file_exists / custom_expression)
  - `condition_config`: configuracion especifica del tipo
  - `quiet_hours`: horario donde NO se evalua (ej: 22:00-07:00)
- El heartbeat corre como background task en la app local
- Cada intervalo: evalua condicion → si true, ejecuta grafo → si false, no hace nada
- Output del trigger: `{ triggered: boolean, condition_result: any, evaluated_at: timestamp }`

**Entidades nuevas**: Tabla `heartbeat_state` (ver seccion Entidades)

**Contratos API**:
- `GET /api/agents/{id}/heartbeat` — estado del heartbeat
- `POST /api/agents/{id}/heartbeat/pause` — pausar
- `POST /api/agents/{id}/heartbeat/resume` — reanudar
- `POST /api/agents/{id}/heartbeat/trigger` — forzar evaluacion

**Pantallas**:
- AgentDetail → seccion Heartbeat: estado, ultima evaluacion, ultimos triggers, boton forzar
- EditorCanvas → trigger/heartbeat en catalogo con configuracion visual

**Reglas**: REGLA-61, REGLA-62, REGLA-63

**Archivos**:
- CREAR `framework/src/datamirai_engine/tools/builtin/trigger/heartbeat.py`
- CREAR `app/server/datamirai_app/heartbeat_runner.py` — background task que ejecuta evaluaciones
- MODIFICAR `app/server/datamirai_app/database.py` — tabla heartbeat_state
- CREAR `app/server/datamirai_app/routes/heartbeat.py`
- MODIFICAR frontend catalogo + agent detail

---

### 3.4 — Resume desde Checkpoint

**Problema**: Hoy tenemos checkpoints (FEAT-001) que guardan SharedState despues de cada nodo. Pero si un grafo falla en nodo 7 de 12, no hay forma de retomar desde el checkpoint del nodo 6. El usuario tiene que re-ejecutar desde el inicio.

**Solucion**: Implementar resume que crea nueva session desde un checkpoint existente, restaurando SharedState y continuando desde el nodo siguiente.

**Arquitectura**:
- `POST /api/sessions/{id}/resume-from/{checkpointId}` — crea nueva session
- La nueva session:
  1. Carga SharedState del checkpoint
  2. Identifica el nodo siguiente al checkpoint (via cursor_position + pending_edges)
  3. Ejecuta el grafo desde ese punto
  4. Genera sus propios checkpoints de ahi en adelante
- La session original NO se modifica (inmutabilidad de REGLA-13)
- Metadata de la nueva session incluye: `resumed_from_session_id`, `resumed_from_checkpoint_id`

**Entidades nuevas**: Columnas en tabla session: `resumed_from_session_id TEXT NULL`, `resumed_from_checkpoint_id TEXT NULL`

**Contratos API**:
- `POST /api/sessions/{id}/resume-from/{checkpointId}` — crear session resumida
  - Body opcional: `{ "state_overrides": { "node_id": { ... } } }` para modificar state antes de resumir

**Pantallas**:
- SessionDetail → en cada checkpoint: boton "Resumir desde aqui"
- Si la session tiene status "failed": banner sugerido "Quieres retomar desde el ultimo checkpoint exitoso?"
- SessionDetail de session resumida: badge "Resumido desde [session original] checkpoint [N]" con link

**Reglas**: REGLA-64, REGLA-65, REGLA-66

**Archivos**:
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — metodo resume_from_checkpoint()
- MODIFICAR `app/server/datamirai_app/database.py` — columnas en session
- MODIFICAR `app/server/datamirai_app/routes/sessions.py` — endpoint resume
- MODIFICAR frontend SessionDetail

---

### 3.5 — Sub-grafos (Bloque agent/run_agent)

**Problema**: No se pueden componer agentes. Si tienes un agente "investigador" y un agente "escritor", no puedes hacer que el agente "manager" ejecute al investigador primero y despues al escritor.

**Solucion**: Nuevo bloque `agent/run_agent` que ejecuta otro agente completo como un nodo sincrono del grafo.

**Arquitectura**:
- Nuevo tool `agent/run_agent` en el catalogo
- Configuracion:
  - `agent_id`: ID del agente a ejecutar (selector en UI)
  - `input_mapping`: mapeo de datos del SharedState actual → inputs del sub-agente
  - `timeout_seconds`: timeout maximo (default 300s)
- Ejecucion:
  1. Instanciar nuevo GraphRunner para el sub-agente
  2. Crear nueva session del sub-agente con los inputs mapeados
  3. Ejecutar el grafo completo del sub-agente
  4. Esperar a que termine (sincrono)
  5. Retornar output del sub-agente al SharedState del grafo padre
- Contexto aislado: el sub-agente NO ve el SharedState del padre. Solo recibe los inputs explicitos.
- Output del nodo: `{ agent_id, session_id, status, result: { ...outputs del sub-agente }, duration_ms }`

**Entidades nuevas**: Columna en tabla session: `parent_session_id TEXT NULL` (para trazar jerarquia)

**Contratos API**: No nuevos (el bloque se ejecuta como cualquier otro nodo)

**Pantallas**:
- EditorCanvas → catalogo: nuevo bloque "Ejecutar Agente" en categoria "Agent"
- NodeConfigPanel → para run_agent: selector de agente, mapeo de inputs, timeout
- SessionDetail → si tiene parent: link "Ejecutado por [session padre]". Si ejecuto sub-agentes: lista con links.

**Reglas**: REGLA-67, REGLA-68, REGLA-69, REGLA-70

**Archivos**:
- CREAR `framework/src/datamirai_engine/tools/builtin/agent/run_agent.py`
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — soporte para nesting depth tracking
- MODIFICAR `app/server/datamirai_app/database.py` — columna parent_session_id
- MODIFICAR frontend catalogo + node config + session detail

---

## Contratos API

### Context Compiler
No nuevos — se configura como parte del agente.

### Memory Flush
No nuevos — es automatico.

### Heartbeat
- `GET /api/agents/{id}/heartbeat` — estado del heartbeat
- `POST /api/agents/{id}/heartbeat/pause` — pausar
- `POST /api/agents/{id}/heartbeat/resume` — reanudar
- `POST /api/agents/{id}/heartbeat/trigger` — forzar evaluacion

### Resume
- `POST /api/sessions/{id}/resume-from/{checkpointId}` — crear session resumida
  - Body opcional: `{ "state_overrides": { "node_id": { ... } } }`

### Sub-grafos
No nuevos — el bloque se ejecuta como cualquier otro nodo.

---

## Pantallas

### Modificadas
- **AgentDesigner** (`/agents/[id]`): seccion Context Compiler (toggles, identity prompt). Seccion Heartbeat (estado, controles)
- **EditorCanvas**: trigger/heartbeat en catalogo. Bloque agent/run_agent en catalogo
- **NodeConfigPanel**: config para heartbeat (interval, condition, quiet_hours). Config para run_agent (agent selector, input mapping, timeout)
- **SessionDetail** (`/sessions/[id]`): boton "Resumir desde aqui" en checkpoints. Banner resume para sessions fallidas. Badge "Resumido desde..." en sessions resumidas. Links padre/hijo para sub-grafos

### Sin cambios de UI
- Memory Flush (mecanismo interno del engine)

---

## Dependencias entre features

```
3.1 (Context Compiler) ← depende de Tier 1 (LLM adapter) + Tier 2 (playbook, memoria)
3.2 (Memory Flush) ← depende de 3.1 (usa Context Compiler para determinar token count) + Tier 2 (memoria backend)
3.3 (Heartbeat) ← independiente, puede implementarse en paralelo
3.4 (Resume) ← independiente (usa checkpoints de FEAT-001)
3.5 (Sub-grafos) ← independiente (usa GraphRunner existente)
```

Orden sugerido de implementacion:
1. 3.3 + 3.4 + 3.5 en paralelo (independientes)
2. 3.1 (Context Compiler)
3. 3.2 (Memory Flush, depende de 3.1)

---

## Notas de implementacion

- **Context Compiler token counting**: usar tiktoken para modelos OpenAI, estimacion por caracteres para otros. Budget por fase configurable.
- **Memory Flush**: el LLM call para generar resumen usa el mismo adapter configurado en el agente. Si no hay adapter, flush se salta.
- **Heartbeat evaluador**: cada condition_type tiene un evaluator registrado. Custom expression usa safe_eval (sin exec/import).
- **Resume validacion**: verificar que el checkpoint pertenece a la session indicada. Verificar que el graph_def no cambio (o advertir si cambio).
- **Sub-grafos circular check**: construir grafo de dependencias entre agentes en tiempo de validacion del graph_def. Detectar ciclos con DFS.
- **Nesting depth**: se propaga via ExecutionContext. Cada nivel incrementa depth. Si depth >= 3, run_agent rechaza ejecucion.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md`
- `docs/prd/draft/FEAT-001.md`
- `docs/prd/draft/FEAT-002.md`
- `docs/prd/draft/FEAT-003.md`
- `docs/ARCHITECTURE.md`
