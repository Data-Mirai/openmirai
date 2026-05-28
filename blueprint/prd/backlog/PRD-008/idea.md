# PRD-008 — Live Agent: Ejecucion Ciclica + Memoria Persistente

| Campo | Valor |
|-------|-------|
| **ID** | PRD-008 |
| **Fecha** | 2026-05-28 |
| **Estado** | in_progress |
| **Branch** | prd/PRD-008 |
| **Target** | v0.5.0 |

---

## Diagrama General

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    PRD-008 — DOS MODOS DE EJECUCION                    │
│                                                                         │
│  MANAGED (agent_type: managed)                                          │
│  ─────────────────────────────                                          │
│                                                                         │
│  Trigger externo                                                        │
│       │ (webhook, manual, API call)                                     │
│       ▼                                                                 │
│  ┌────────────┐     ┌────────────┐     ┌────────────┐                  │
│  │ inject     │──▶  │ run graph  │──▶  │  return    │                  │
│  │ memory     │     │            │     │  result    │                  │
│  └────────────┘     └────────────┘     └─────┬──────┘                  │
│                                               │                         │
│                                          persist                        │
│                                          memory                         │
│                                               │                         │
│                                               ▼                         │
│                                        ┌────────────┐                  │
│                                        │ MemoryStore│                  │
│                                        └────────────┘                  │
│                                                                         │
│  LIVE (agent_type: live)                                                │
│  ───────────────────────                                                │
│                                                                         │
│  ┌──────┐     ┌─────────────────────────────────────────────┐          │
│  │ PLAY │──▶  │              CYCLE LOOP                     │          │
│  └──────┘     │                                             │          │
│               │  ┌────────┐   ┌────────┐   ┌────────┐     │          │
│               │  │inject  │─▶│  run   │─▶│persist │     │          │
│               │  │memory  │   │ graph  │   │memory  │     │          │
│               │  └────────┘   └────────┘   └───┬────┘     │          │
│               │                                 │          │          │
│               │       sleep(interval) ◀─────────┘          │          │
│               │                                             │          │
│  ┌──────┐     └─────────────────────────────────────────────┘          │
│  │ STOP │──▶  break loop                                               │
│  └──────┘                                                               │
│                                                                         │
│  MEMORIA (graph.memory) — compartida por ambos modos                   │
│  ────────────────────────────────────────────────────                   │
│                                                                         │
│  graph.memory declara keys + valores iniciales + nivel de persistencia │
│  → inyectados en SharedState como nodo virtual "memory"                │
│  → accesibles via ${memory.key} en data_map/config                     │
│  → persistidos via nodo state/memory al final del grafo                │
│  → almacenados por agent_id en MemoryStore                             │
│                                                                         │
│  TRES NIVELES DE PERSISTENCIA (configurable):                          │
│                                                                         │
│  persist: none                                                          │
│  ─────────────                                                          │
│  Cada ciclo/ejecucion arranca con valores iniciales.                   │
│  Memoria es temporal — solo vive dentro de una ejecucion.              │
│                                                                         │
│  persist: cycle                                                         │
│  ──────────────                                                         │
│  Live: ciclo N persiste → ciclo N+1 lee. Al stop/play, resetea.       │
│  Managed: equivale a "none" (una sola ejecucion por sesion).           │
│                                                                         │
│  persist: execution                                                     │
│  ─────────────────                                                      │
│  Sobrevive todo: ciclos, stop/play, ejecuciones independientes.        │
│  Solo se resetea con clear_memory explicito.                           │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────┐          │
│  │              persist: none                                │          │
│  │  cycle 1: init → run → [forget]                          │          │
│  │  cycle 2: init → run → [forget]                          │          │
│  ├──────────────────────────────────────────────────────────┤          │
│  │              persist: cycle                               │          │
│  │  play:                                                    │          │
│  │    cycle 1: init → run → save ──┐                        │          │
│  │    cycle 2: load → run → save ──┘ (acumula)              │          │
│  │  stop → play:                                             │          │
│  │    cycle 1: init → run → save  (reset, arranca de cero)  │          │
│  ├──────────────────────────────────────────────────────────┤          │
│  │              persist: execution                           │          │
│  │  play:                                                    │          │
│  │    cycle 1: init → run → save ──┐                        │          │
│  │    cycle 2: load → run → save ──┘                        │          │
│  │  stop → play:                                             │          │
│  │    cycle 3: load → run → save  (continua desde donde     │          │
│  │                                  quedo, no resetea)       │          │
│  └──────────────────────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: feature
- **Resumen**: Los dos agent types (managed/live) estan definidos como enum pero se comportan identicamente. No hay ejecucion ciclica, no hay scheduling conectado, no hay memoria persistente entre ejecuciones. El Scheduler existe desconectado del server.
- **Actores**: Engine (runtime), Host App (HTTP), Developer (YAML/CLI)
- **Flujos tocados**: AgentSpec parsing, agent execution (CLI + HTTP), agent lifecycle, scheduler
- **Que cambia**:
  - HOY: `agent_type` es cosmético — todos los agentes ejecutan una vez y terminan. Scheduler existe pero no conectado. Cada ejecucion arranca con estado limpio.
  - DESPUES: managed ejecuta por trigger externo (una vez), live ejecuta en ciclo continuo (play/stop). Ambos pueden tener memoria persistente entre ejecuciones, configurada por el grafo.

---

## Actores y Permisos

| Actor | Capacidad | Accion | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | disenar_agente | Define agent_type, schedule, graph.memory en YAML | Spec completo |
| Host App | controlar_agente | Play/stop live agents via HTTP | Estado + ciclos + memoria |
| Host App | ejecutar_agente | Execute managed agents via HTTP (sin cambio) | Resultado de ejecucion |
| Engine | ciclar_agente | Ejecuta ciclos automaticamente segun schedule | Interno |
| Engine | persistir_memoria | Lee/escribe memoria entre ejecuciones | Interno |

---

## Entidades

### AgentScheduleSpec (nueva)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| interval_seconds | numero | no | Segundos entre ciclos. Mutuamente exclusivo con cron |
| cron | texto | no | Expresion cron (5 campos). Mutuamente exclusivo con interval_seconds |
| max_cycles | numero | no | Maximo de ciclos antes de auto-stop. null = infinito |
| on_cycle_error | enum [continue, stop] | no (default: continue) | Comportamiento al fallar un ciclo |

**Restricciones**:
- Exactamente uno de `interval_seconds` o `cron` debe estar presente
- `interval_seconds` minimo: 1 segundo
- `max_cycles` null o >= 1

### AgentMemorySpec (nueva — declaracion de memoria en graph)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| persist | enum [none, cycle, execution] | no (default: cycle) | Nivel de persistencia entre ciclos/ejecuciones |
| keys | mapa de (texto → valor) | si | Keys persistentes con valores iniciales |

**Semantica de persist**:

| Modo | Entre ciclos (live) | Entre ejecuciones (stop→play o execute→execute) |
|------|--------------------|-------------------------------------------------|
| none | Resetea cada ciclo | Resetea cada ejecucion |
| cycle | Acumula ciclo a ciclo | Resetea al iniciar nueva sesion (play) |
| execution | Acumula ciclo a ciclo | Acumula entre sesiones. Solo clear_memory resetea |

**Para managed agents**:
- `none` = cada execute arranca con iniciales
- `cycle` = equivale a `none` (managed no tiene ciclos, una ejecucion = una sesion)
- `execution` = estado persiste entre llamadas a execute separadas

### AgentGraphSpec (modificada)

Campos nuevos:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| memory | AgentMemorySpec | no | Declaracion de memoria persistente |

```
AgentGraphSpec:
  nodes: Vec<AgentNodeSpec>
  edges: Vec<AgentEdgeSpec>
  memory: Option<AgentMemorySpec>        ◀── NUEVO
```

**Backward compat**: `memory` usa `Option` con `#[serde(default)]`. Grafos sin memory → None → sin persistencia (identico a hoy).

### AgentSpec (modificada)

Campo nuevo:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| schedule | AgentScheduleSpec | no | Configuracion de ciclo. Solo significativo cuando agent_type = live |

```
AgentSpec:
  ...campos existentes...
  schedule: Option<AgentScheduleSpec>    ◀── NUEVO
  ...
```

### CycleRecord (nueva — runtime)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| cycle_id | id | si | Identificador unico del ciclo |
| agent_id | referencia → Agent | si | Agente que ejecuto |
| cycle_number | numero | si | Numero secuencial del ciclo (1-based) |
| started_at | fecha-hora | si | Inicio del ciclo |
| completed_at | fecha-hora | no | Fin del ciclo (null si en progreso) |
| status | enum [running, completed, failed] | si | Resultado del ciclo |
| duration_ms | numero | no | Duracion en milisegundos |
| error | texto | no | Mensaje de error si failed |
| trace_summary | json | no | Resumen del trace (nodos ejecutados, tiempos) |

**Relaciones**: Agent 1 → N CycleRecord

### MemoryStore (nueva — per-agent persistent state)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| agent_id | referencia → Agent | si | Agente dueno de esta memoria |
| data | mapa de (texto → valor) | si | Key-value de memoria persistida |
| updated_at | fecha-hora | si | Ultima escritura |

**Restricciones de acceso**: solo el Engine lee/escribe. Host App puede leer via API.
**Unicidad**: un MemoryStore por agent_id (1:1).

### RuntimeAgentStatus (modificada)

Estados actuales: Enabled | Disabled | Error

Nuevo estado:

| Estado | Descripcion |
|--------|-------------|
| Playing | Live agent activamente ciclando |

```
RuntimeAgentStatus:
  Enabled      ← registrado, idle (managed + live)
  Disabled     ← deshabilitado manualmente
  Playing      ← live: ciclando activamente       ◀── NUEVO
  Error(msg)   ← error ocurrio
```

---

## Ciclos de Vida

### LiveAgent

Estados: ENABLED | PLAYING | DISABLED | ERROR

| Desde | Hacia | Actor | Guard | Efecto |
|-------|-------|-------|-------|--------|
| ENABLED | PLAYING | Host App | agent_type = live AND schedule definido | Scheduler registra ciclo. Emite AgentPlaying |
| PLAYING | ENABLED | Host App | — | Scheduler cancela ciclo. Emite AgentStopped |
| PLAYING | ERROR | Engine | on_cycle_error = stop AND ciclo falla | Scheduler cancela ciclo. Registra error |
| PLAYING | PLAYING | Engine | on_cycle_error = continue AND ciclo falla | Log error, continua al siguiente ciclo |
| ERROR | ENABLED | Host App | — | Limpia error. Listo para play |
| ENABLED | DISABLED | Host App | — | No puede hacer play hasta re-enable |
| DISABLED | ENABLED | Host App | — | Listo para play |

```
                        ┌──────────┐
                ┌──────▶│ DISABLED │◀──────┐
                │       └──────────┘       │
           disable                     disable
                │                          │
                │       ┌──────────┐       │
                ├───────│ ENABLED  │───────┤
                │       └─────┬────┘       │
                │             │            │
                │       play (live only)   │
                │             │            │
                │             ▼            │
                │       ┌──────────┐       │
                │       │ PLAYING  │───────┘
                │       └─────┬────┘
                │          │      │
                │        stop   error
                │          │   (on_cycle_error=stop)
                │          ▼      │
                │       ENABLED   ▼
                │              ┌───────┐
                └──────────────│ ERROR │
                               └───────┘
```

### Cycle (dentro de un LiveAgent playing)

Estados: STARTED | COMPLETED | FAILED

| Desde | Hacia | Actor | Guard | Efecto |
|-------|-------|-------|-------|--------|
| STARTED | COMPLETED | Engine | grafo termina exitosamente | Registra CycleRecord. Persiste memoria. Emite CycleCompleted |
| STARTED | FAILED | Engine | grafo falla | Registra CycleRecord. NO persiste memoria. Emite CycleFailed |

```
          ┌─────────┐
   ──────▶│ STARTED │
          └────┬────┘
               │
          run graph
               │
          ok?
         ╱      ╲
       Si        No
       │          │
       ▼          ▼
┌───────────┐ ┌────────┐
│ COMPLETED │ │ FAILED │
└───────────┘ └────────┘
       │          │
  persist     no persist
  memory      (state safe)
```

### ManagedAgent (sin cambio funcional)

La ejecucion de managed agents no cambia. Si el grafo tiene `graph.memory`, la memoria se inyecta al inicio y se persiste al final — igual que un live agent pero sin ciclo.

---

## Reglas de Negocio

### schedule-requiere-live

- **Invariante**: Si `schedule` esta definido, `agent_type` DEBE ser `live`. Si `agent_type = live`, `schedule` DEBE estar definido.
- **Cuando se verifica**: al parsear AgentSpec (validate)
- **Si se viola**: error: "live agents require a schedule section" o "schedule is only valid for live agents"

### schedule-mutuamente-exclusivo

- **Invariante**: `interval_seconds` y `cron` son mutuamente exclusivos. Exactamente uno debe estar presente en schedule.
- **Cuando se verifica**: al parsear AgentSpec (validate)
- **Si se viola**: error: "schedule must have exactly one of: interval_seconds, cron"

### memory-keys-declarados

- **Invariante**: El nodo `state/memory` solo puede persistir keys que estan declarados en `graph.memory`. Keys no declarados se ignoran silenciosamente.
- **Cuando se verifica**: en state/memory tool.execute()
- **Si se viola**: key no declarado se ignora (no error, safe default). Log warning.

### memory-inyeccion-segun-persist

- **Invariante**: Al iniciar ejecucion/ciclo, el runner inyecta memoria en SharedState segun el modo persist:
  - `none`: siempre inyecta valores iniciales de graph.memory.keys
  - `cycle`: inyecta valores del ciclo anterior (dentro de la misma sesion play) o iniciales si primer ciclo de la sesion
  - `execution`: inyecta valores del MemoryStore (o iniciales si primera vez global)
- **Cuando se verifica**: en GraphRunner.run(), pre-walk
- **Si se viola**: N/A — por construccion del runner

### persist-cycle-resetea-entre-sesiones

- **Invariante**: Cuando persist = cycle, al hacer play() en un live agent, la memoria de ciclos se resetea a valores iniciales. El primer ciclo de cada sesion play arranca limpio.
- **Cuando se verifica**: en play_agent, antes de iniciar scheduler
- **Si se viola**: N/A — por construccion

### ciclo-fallo-no-persiste

- **Invariante**: Si un ciclo/ejecucion falla (status = Failed), la memoria NO se actualiza (para persist = cycle y execution). El siguiente ciclo/ejecucion arranca con la memoria del ultimo exito.
- **Cuando se verifica**: en el cycle loop del Scheduler / en run_agent_spec
- **Si se viola**: N/A — por construccion

### play-solo-live

- **Invariante**: Solo agentes con `agent_type = live` pueden recibir operacion play/stop. Managed agents no tienen esta operacion.
- **Cuando se verifica**: en handler de play/stop
- **Si se viola**: error: "only live agents support play/stop"

### play-requiere-enabled

- **Invariante**: Un agente debe estar en estado ENABLED para hacer play. No se puede hacer play desde DISABLED o ERROR directamente.
- **Cuando se verifica**: en operacion play_agent
- **Si se viola**: error: "agent must be enabled before playing"

---

## Patrones de Diseno

### State → Live Agent lifecycle

- **Aplica a**: RuntimeAgentStatus con transiciones dependientes de agent_type
- **Por que**: El comportamiento del agente (acepta play/stop, cicla automaticamente) depende de su estado actual. Un agente Playing rechaza play, un agente Disabled rechaza play.
- **Participantes**: AgentRuntime (orquesta transiciones), Scheduler (ejecuta ciclos), RuntimeAgentStatus (estado)

### Observer → Cycle events

- **Aplica a**: Emision de eventos durante ciclos de vida de ciclos
- **Por que**: El Host App necesita reaccionar a ciclos completados/fallidos en tiempo real (SSE, logging, alertas)
- **Participantes**: EventEmitter (emisor), Scheduler (produce eventos), Host App (suscriptor via SSE)

### Template Method → Execution con memory injection

- **Aplica a**: GraphRunner.run() con pasos pre/post para memoria
- **Por que**: El flujo base (validate → walk → result) se extiende con inject_memory al inicio y persist_memory al final, sin cambiar el algoritmo core
- **Participantes**: GraphRunner (template), MemoryStore (pre/post steps), state/memory tool (write step)

```
┌─ Template Method: run() ──────────────────────────┐
│                                                     │
│  1. inject_memory(graph.memory, store) ◀── NUEVO   │
│  2. validate(graph)                                 │
│  3. walk(graph, state)          ← sin cambio       │
│  4. build_result()                                  │
│                                                     │
│  state/memory tool en el grafo persiste al store    │
│  durante step 3 (walk), como cualquier otro nodo    │
│                                                     │
└─────────────────────────────────────────────────────┘
```

---

## Operaciones

### play_agent

- **Actor**: Host App
- **Input**:
  - agent_id (texto, requerido) — ID del agente registrado
- **Output exitoso**: confirmacion con agent_id, status = playing, schedule info
- **Errores posibles**:
  - Agente no encontrado → "agent not found: {id}"
  - Agente no es live → "only live agents support play/stop"
  - Agente disabled → "agent must be enabled before playing"
  - Ya playing → "agent is already playing"
- **Efectos secundarios**: Scheduler registra ciclo. Emite AgentPlaying event.

### stop_agent

- **Actor**: Host App
- **Input**:
  - agent_id (texto, requerido) — ID del agente
- **Output exitoso**: confirmacion con agent_id, status = enabled, cycles_completed
- **Errores posibles**:
  - Agente no encontrado → "agent not found: {id}"
  - Agente no es live → "only live agents support play/stop"
  - No esta playing → "agent is not playing"
- **Efectos secundarios**: Scheduler cancela ciclo. Emite AgentStopped event. Ciclo en progreso se deja terminar (graceful).

### get_agent_cycles

- **Actor**: Host App
- **Input**:
  - agent_id (texto, requerido)
  - limit (numero, opcional, default: 50) — ultimos N ciclos
- **Output exitoso**: lista de CycleRecord ordenados por cycle_number desc
- **Errores posibles**:
  - Agente no encontrado → "agent not found: {id}"

### get_agent_memory

- **Actor**: Host App / Developer
- **Input**:
  - agent_id (texto, requerido)
- **Output exitoso**: mapa de key-value de la memoria actual
- **Errores posibles**:
  - Agente no encontrado → "agent not found: {id}"
  - Sin memoria declarada → respuesta vacia {}

### clear_agent_memory

- **Actor**: Host App
- **Input**:
  - agent_id (texto, requerido)
- **Output exitoso**: confirmacion, memoria reseteada a valores iniciales de graph.memory
- **Errores posibles**:
  - Agente no encontrado → "agent not found: {id}"
  - Agente playing → "cannot clear memory while agent is playing" (must stop first)
- **Efectos secundarios**: MemoryStore reseteado a initial values.

### execute_cycle (interna)

- **Actor**: Engine (Scheduler)
- **Input**:
  - agent_id (texto) — agente live que esta playing
  - cycle_number (numero) — secuencial
  - is_first_cycle_of_session (booleano)
- **Logica**:
  1. Crear CycleRecord con status = running
  2. Determinar memoria a inyectar segun persist mode:
     - none → valores iniciales siempre
     - cycle → cycle_memory del ciclo anterior (o iniciales si is_first_cycle_of_session)
     - execution → MemoryStore (o iniciales si primera vez)
  3. Inyectar memoria en SharedState como nodo virtual "memory"
  4. Inyectar cycle_number + cycle_id en trigger config
  5. Ejecutar grafo via run_agent_spec (mismo flujo que managed)
  6. Si OK:
     - CycleRecord.status = completed
     - state/memory dentro del grafo ya persisto segun modo (cycle_memory o MemoryStore)
  7. Si Error:
     - CycleRecord.status = failed
     - Memoria NO se actualiza (ni cycle_memory ni MemoryStore)
  8. Emitir CycleCompleted o CycleFailed event
  9. Si max_cycles alcanzado → auto-stop

### inject_memory (interna)

- **Actor**: Engine (GraphRunner)
- **Input**:
  - graph.memory.persist (modo de persistencia)
  - graph.memory.keys (mapa de keys + initial values)
  - stored_memory (mapa de keys + persisted values, puede ser null)
  - cycle_memory (mapa de keys + valores del ciclo anterior, puede ser null)
  - is_first_cycle_of_session (booleano)
- **Logica segun persist mode**:

  **persist = none**:
  1. Siempre usar valores iniciales de graph.memory.keys
  2. Ignorar stored_memory y cycle_memory

  **persist = cycle**:
  1. Si is_first_cycle_of_session = true → usar valores iniciales
  2. Si is_first_cycle_of_session = false → usar cycle_memory (del ciclo anterior)
  3. Ignorar stored_memory (no se lee del MemoryStore)

  **persist = execution**:
  1. Si stored_memory tiene el key → usar valor persistido
  2. Si no → usar valor inicial de graph.memory.keys

  **Comun a todos**:
  3. Insertar en SharedState bajo nodo virtual "memory"
  4. Las expressions `${memory.key}` resuelven contra este nodo
- **Output**: SharedState con memoria inyectada

### persist_memory (interna — via state/memory tool)

- **Actor**: Engine (state/memory tool dentro del grafo)
- **Input**: lo que llega via data_map al nodo state/memory + graph.memory.persist
- **Logica segun persist mode**:

  **persist = none**:
  1. No-op. El nodo ejecuta pero no persiste nada. Los valores mueren al terminar el ciclo/ejecucion.

  **persist = cycle**:
  1. Para cada input key declarado en graph.memory.keys → escribir en cycle_memory (in-memory, no MemoryStore)
  2. cycle_memory vive mientras el scheduler esta activo (play session)
  3. Al stop → cycle_memory se descarta

  **persist = execution**:
  1. Para cada input key declarado en graph.memory.keys → escribir en MemoryStore (persistente)
  2. Actualizar MemoryStore.updated_at

  **Comun a todos**:
  - Keys no declarados en graph.memory.keys → ignorar + log warning
- **Output**: confirmacion de keys procesados + modo de persistencia usado

---

## Interfaces

> Engine no tiene frontend. Las interfaces son CLI y HTTP API.

### CLI: mirai play (nueva)

- **Proposito**: Iniciar ciclos de un live agent
- **Uso**: `mirai play agent.yaml`
- **Comportamiento**:
  1. Cargar spec, validar que es live
  2. Arrancar servidor embebido (si no esta corriendo)
  3. Registrar agente + play
  4. Mostrar logs de ciclos en stdout (streaming)
  5. Ctrl+C → stop graceful
- **Output**:
  ```
  Agent: news-monitor (live)
  Schedule: every 300s
  Memory keys: last_id, count
  Status: playing

  [cycle 1] started
  [cycle 1] completed (1.2s) — memory: {last_id: "abc", count: 1}
  [cycle 2] started
  ...
  ^C
  Stopping... waiting for current cycle to finish
  Agent stopped after 12 cycles
  ```

### CLI: mirai run (modificada)

- **Cambio**: Si el agente tiene `graph.memory`, inyectar memoria al inicio
- **Para managed**: ejecutar una vez como hoy, pero con memoria si declarada
- **Para live**: error: "live agents must be started with `mirai play`, not `mirai run`"

### HTTP: POST /api/v1/agents/{id}/play (nueva)

- **Proposito**: Iniciar ciclos de un live agent registrado
- **Request body**: (vacio o opcional config overrides)
- **Response exitosa** (200):
  ```json
  {
    "agent_id": "abc123",
    "status": "playing",
    "schedule": {"interval_seconds": 300},
    "memory_keys": ["last_id", "count"]
  }
  ```
- **Errores**: 404 not found, 409 already playing, 422 not a live agent

### HTTP: POST /api/v1/agents/{id}/stop (nueva)

- **Proposito**: Detener ciclos de un live agent
- **Request body**: (vacio)
- **Response exitosa** (200):
  ```json
  {
    "agent_id": "abc123",
    "status": "enabled",
    "cycles_completed": 42,
    "last_cycle_at": "2026-05-28T12:00:00Z"
  }
  ```
- **Errores**: 404 not found, 409 not playing

### HTTP: GET /api/v1/agents/{id}/cycles (nueva)

- **Proposito**: Historial de ciclos de un live agent
- **Query params**: `?limit=50`
- **Response** (200):
  ```json
  {
    "agent_id": "abc123",
    "total_cycles": 42,
    "cycles": [
      {
        "cycle_id": "cyc-001",
        "cycle_number": 42,
        "status": "completed",
        "started_at": "...",
        "completed_at": "...",
        "duration_ms": 1200
      }
    ]
  }
  ```

### HTTP: GET /api/v1/agents/{id}/memory (nueva)

- **Proposito**: Leer memoria actual del agente
- **Response** (200):
  ```json
  {
    "agent_id": "abc123",
    "memory": {"last_id": "article-99", "count": 42},
    "updated_at": "2026-05-28T12:00:00Z"
  }
  ```

### HTTP: DELETE /api/v1/agents/{id}/memory (nueva)

- **Proposito**: Resetear memoria a valores iniciales
- **Response** (200):
  ```json
  {
    "agent_id": "abc123",
    "memory": {"last_id": null, "count": 0},
    "reset_to": "initial_values"
  }
  ```
- **Errores**: 409 agent is playing (must stop first)

### HTTP: POST /api/v1/agents/{id}/execute (modificada)

- **Cambio**: Si el agente tiene graph.memory, inyectar memoria al inicio
- **Managed**: funciona como hoy + memoria
- **Live**: error 422: "live agents are controlled via play/stop, not execute"

### HTTP: GET /api/v1/agents/{id} (mejorada)

- **Cambio**: Response incluye campos nuevos
  ```json
  {
    "id": "abc123",
    "name": "news-monitor",
    "agent_type": "live",
    "status": "playing",
    "schedule": {"interval_seconds": 300, "on_cycle_error": "continue"},
    "memory_keys": ["last_id", "count"],
    "current_cycle": 42,
    "last_cycle_at": "..."
  }
  ```

### Agent YAML (formato actualizado)

**Managed con memoria entre ejecuciones**:
```yaml
name: conversation-bot
version: v1
agent_type: managed

graph:
  memory:
    persist: execution          # recuerda entre llamadas a execute
    keys:
      history: []

  nodes:
    - id: trigger
      tool_type: trigger/manual
    - id: llm
      tool_type: ai/llm_call
      config:
        prompt: "History: ${memory.history}\nUser: ${trigger.payload.message}"
    - id: save
      tool_type: state/memory
  edges:
    - source: trigger
      target: llm
    - source: llm
      target: save
      data_map:
        history: llm.updated_history
```

**Managed sin memoria** (cada ejecucion independiente):
```yaml
name: qa-bot
version: v1
agent_type: managed

graph:
  memory:
    persist: none               # cada execute arranca limpio
    keys:
      scratch: null             # workspace temporal, no sobrevive

  nodes: [...]
```

**Live con memoria entre ciclos** (resetea entre play sessions):
```yaml
name: news-monitor
version: v1
agent_type: live

schedule:
  interval_seconds: 300
  on_cycle_error: continue

graph:
  memory:
    persist: cycle              # acumula ciclo a ciclo, resetea al re-play
    keys:
      last_processed_id: null
      batch_count: 0

  nodes:
    - id: trigger
      tool_type: trigger/schedule
    - id: fetch
      tool_type: data/web_scrape
      config:
        url: "https://api.news/latest?after=${memory.last_processed_id}"
    - id: analyze
      tool_type: ai/llm_call
      config:
        prompt: "Summarize: ${fetch.content}"
    - id: save
      tool_type: state/memory
  edges:
    - source: trigger
      target: fetch
    - source: fetch
      target: analyze
    - source: analyze
      target: save
      data_map:
        last_processed_id: fetch.article_id
        batch_count: fetch.count
```

**Live con memoria permanente** (sobrevive stop/play):
```yaml
name: data-collector
version: v1
agent_type: live

schedule:
  interval_seconds: 3600
  on_cycle_error: continue

graph:
  memory:
    persist: execution          # NUNCA resetea, acumula para siempre
    keys:
      total_records: 0
      last_sync: null
      error_count: 0

  nodes: [...]
```

**Live sin memoria** (cada ciclo independiente):
```yaml
name: health-checker
version: v1
agent_type: live

schedule:
  interval_seconds: 60

graph:
  memory:
    persist: none               # cada ciclo es independiente
    keys:
      check_result: null        # solo vive dentro del ciclo

  nodes: [...]
```

### Events (nuevos)

| EventType | Cuando | Data |
|-----------|--------|------|
| AgentPlaying | play_agent exitoso | agent_id, schedule |
| AgentStopped | stop_agent exitoso | agent_id, cycles_completed |
| CycleStarted | inicio de un ciclo | agent_id, cycle_id, cycle_number |
| CycleCompleted | ciclo termina OK | agent_id, cycle_id, cycle_number, duration_ms |
| CycleFailed | ciclo falla | agent_id, cycle_id, cycle_number, error |

---

## Matriz de Permutaciones

### Live Agent Lifecycle

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| play_agent | Happy: live agent enabled | Host App | Agent status → playing, scheduler activo |
| play_agent | Agente no encontrado | Host App | Error 404 |
| play_agent | Agente es managed | Host App | Error 422: "only live agents support play/stop" |
| play_agent | Agente disabled | Host App | Error 422: "agent must be enabled before playing" |
| play_agent | Ya esta playing | Host App | Error 409: "agent is already playing" |
| stop_agent | Happy: live agent playing | Host App | Agent status → enabled, scheduler cancela |
| stop_agent | No esta playing | Host App | Error 409: "agent is not playing" |
| stop_agent | Ciclo en progreso al momento de stop | Host App | Ciclo termina, luego stop (graceful) |

### Cycle Execution

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| execute_cycle | Happy: grafo completa, memoria persiste | Engine | CycleRecord completed, memoria actualizada |
| execute_cycle | Grafo falla, on_cycle_error = continue | Engine | CycleRecord failed, memoria NO actualizada, siguiente ciclo continua |
| execute_cycle | Grafo falla, on_cycle_error = stop | Engine | CycleRecord failed, agent status → error, scheduler cancela |
| execute_cycle | max_cycles alcanzado | Engine | Auto-stop, agent status → enabled |
| execute_cycle | Grafo con timeout | Engine | CycleRecord failed (timeout), apply on_cycle_error |

### Memory — persist: none

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| inject (none) | Cualquier ejecucion/ciclo | Engine | Siempre valores iniciales de graph.memory.keys |
| persist (none) | state/memory ejecuta | Engine | No-op, valores no se guardan |
| live (none) | Ciclo 1 → ciclo 2 | Engine | Ciclo 2 arranca con iniciales (sin carry) |

### Memory — persist: cycle

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| inject (cycle) | Primer ciclo de sesion play | Engine | Valores iniciales |
| inject (cycle) | Ciclo N+1 dentro de sesion | Engine | Valores del ciclo N (acumulado) |
| inject (cycle) | Primer ciclo despues de stop→play | Engine | Valores iniciales (reset) |
| persist (cycle) | state/memory ejecuta | Engine | Guarda en cycle_memory (in-memory) |
| persist (cycle) | Ciclo falla | Engine | cycle_memory no se actualiza |
| managed (cycle) | Execute 1 → execute 2 | Engine | Equivale a none: cada execute arranca limpio |

### Memory — persist: execution

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| inject (execution) | Primera vez global (sin MemoryStore) | Engine | Valores iniciales |
| inject (execution) | Ejecucion N (store existe) | Engine | Valores del store |
| inject (execution) | Despues de stop→play | Engine | Valores del store (NO resetea) |
| persist (execution) | state/memory ejecuta | Engine | Guarda en MemoryStore (persistente) |
| persist (execution) | Ciclo/ejecucion falla | Engine | MemoryStore no se actualiza |
| managed (execution) | Execute 1 persiste → execute 2 | Engine | Execute 2 lee estado de execute 1 |

### Memory — comun

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| inject | Grafo sin graph.memory | Engine | Sin inyeccion (backward compat) |
| persist | Key no declarado en graph.memory.keys | Engine | Ignorado + warning log |
| get_agent_memory | Agente con memoria (execution) | Host App | Mapa key-value del MemoryStore |
| get_agent_memory | Agente con memoria (cycle) | Host App | Mapa key-value del cycle_memory activo |
| get_agent_memory | Agente con memoria (none) | Host App | Valores iniciales (nada persistido) |
| get_agent_memory | Agente sin graph.memory | Host App | Respuesta vacia {} |
| clear_agent_memory | Agente stopped | Host App | MemoryStore/cycle_memory reseteados a iniciales |
| clear_agent_memory | Agente playing | Host App | Error 409: "cannot clear while playing" |

### Managed Agent (backward compat)

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| execute (managed sin memory) | Identico a hoy | Host App | Sin cambio |
| execute (managed con memory) | Primera ejecucion | Host App | Memoria inyectada (initials), persiste al final |
| execute (managed con memory) | Ejecucion N | Host App | Memoria inyectada (stored), persiste al final |
| execute (live agent) | Intento execute en live | Host App | Error 422: "use play/stop" |
| run CLI (managed) | Con graph.memory | Developer | Memoria inyectada + persistida |
| run CLI (live) | Intento mirai run en live | Developer | Error: "use mirai play" |

---

## Escenarios GWT

### Journey: Host App — Play/Stop Live Agent

TEST-068: Play live agent — happy path
  Given: Live agent registrado con status enabled, schedule.interval_seconds = 10
  When: Host App hace POST /api/v1/agents/{id}/play
  Then: Status → playing. Scheduler inicia ciclo. CycleStarted event emitido al primer ciclo.

TEST-069: Stop live agent — happy path
  Given: Live agent con status playing, 5 ciclos completados
  When: Host App hace POST /api/v1/agents/{id}/stop
  Then: Status → enabled. Scheduler cancela. AgentStopped event emitido con cycles_completed = 5.

TEST-070: Play managed agent — error
  Given: Managed agent registrado con status enabled
  When: Host App hace POST /api/v1/agents/{id}/play
  Then: 422 error: "only live agents support play/stop"

TEST-071: Play disabled live agent — error
  Given: Live agent con status disabled
  When: Host App hace POST /api/v1/agents/{id}/play
  Then: 422 error: "agent must be enabled before playing"

TEST-072: Play already playing agent — error
  Given: Live agent con status playing
  When: Host App hace POST /api/v1/agents/{id}/play
  Then: 409 error: "agent is already playing"

TEST-073: Stop graceful — ciclo en progreso
  Given: Live agent playing, ciclo 6 en progreso (mid-execution)
  When: Host App hace POST /api/v1/agents/{id}/stop
  Then: Ciclo 6 termina normalmente. Luego status → enabled. No se inicia ciclo 7.

### Journey: Engine — Cycle Execution

TEST-074: Ciclo completa exitosamente
  Given: Live agent playing, graph.memory con last_id: null
  When: Scheduler dispara ciclo 1
  Then: Grafo ejecuta. state/memory persiste last_id = "abc". CycleRecord status = completed.

TEST-075: Ciclo falla, on_cycle_error = continue
  Given: Live agent playing con on_cycle_error = continue
  When: Ciclo 3 falla (error en nodo LLM)
  Then: CycleRecord status = failed. Memoria NO cambia. Ciclo 4 arranca normalmente.

TEST-076: Ciclo falla, on_cycle_error = stop
  Given: Live agent playing con on_cycle_error = stop
  When: Ciclo 3 falla
  Then: CycleRecord status = failed. Agent status → error. No mas ciclos.

TEST-077: max_cycles alcanzado
  Given: Live agent playing con max_cycles = 10
  When: Ciclo 10 completa exitosamente
  Then: CycleRecord status = completed. Agent auto-stop → status = enabled. AgentStopped event.

### Journey: Engine — Memory persist: none

TEST-078: persist none — cada ciclo arranca limpio
  Given: Live agent con graph.memory: {persist: none, keys: {count: 0}}. Ciclo 1 ejecuta y state/memory recibe count = 5.
  When: Ciclo 2 arranca
  Then: memory.count = 0 (valor inicial). El 5 del ciclo 1 no se propago.

TEST-079: persist none — state/memory es no-op
  Given: Agent con persist: none. Nodo state/memory recibe data_map con count = 10.
  When: state/memory ejecuta
  Then: Nodo completa OK (no error) pero NO persiste. Log indica "persist: none — skipping write".

### Journey: Engine — Memory persist: cycle

TEST-080: persist cycle — acumula entre ciclos
  Given: Live agent playing con persist: cycle, keys: {count: 0}. Ciclo 1 persiste count = 5.
  When: Ciclo 2 arranca
  Then: memory.count = 5 (del ciclo anterior). Ciclo 2 puede acumular.

TEST-081: persist cycle — resetea al re-play
  Given: Live agent que hizo play→(10 ciclos, count = 50)→stop. Ahora se hace play de nuevo.
  When: Primer ciclo de la nueva sesion play arranca
  Then: memory.count = 0 (valor inicial). Los 50 del play anterior no sobreviven.

TEST-082: persist cycle — managed equivale a none
  Given: Managed agent con persist: cycle, keys: {data: null}. Execute 1 persiste data = "hello".
  When: Execute 2 arranca (nueva llamada a /execute)
  Then: memory.data = null (valor inicial). No hay "sesion" en managed, cada execute es independiente.

### Journey: Engine — Memory persist: execution

TEST-083: persist execution — acumula entre ciclos
  Given: Live agent playing con persist: execution, keys: {total: 0}. Ciclo 1 persiste total = 10.
  When: Ciclo 2 arranca
  Then: memory.total = 10 (del MemoryStore).

TEST-084: persist execution — sobrevive stop/play
  Given: Live agent que hizo play→(5 ciclos, total = 50)→stop. Ahora se hace play de nuevo.
  When: Primer ciclo de la nueva sesion arranca
  Then: memory.total = 50 (del MemoryStore). NO resetea.

TEST-085: persist execution — managed entre ejecuciones
  Given: Managed agent con persist: execution, keys: {history: []}. Execute 1 persiste history = ["msg1"].
  When: Execute 2 arranca (nueva llamada)
  Then: memory.history = ["msg1"]. Nodo LLM lee historial previo.

### Journey: Engine — Memory comun

TEST-086: state/memory persiste solo keys declarados
  Given: graph.memory.keys declara {count: 0}. Nodo state/memory recibe data_map con count y extra_key.
  When: state/memory ejecuta
  Then: count se procesa. extra_key se ignora. Warning en log.

TEST-087: Fallo no persiste memoria (cycle y execution)
  Given: Agent con persist: execution, memory total = 5. Grafo falla antes de llegar a state/memory.
  When: Siguiente ciclo/ejecucion
  Then: memory.total = 5 (sin cambio, valor del ultimo exito).

TEST-088: clear_agent_memory resetea a iniciales
  Given: Agent con graph.memory.keys: {count: 0}. MemoryStore tiene {count: 42}.
  When: Host App hace DELETE /api/v1/agents/{id}/memory
  Then: MemoryStore reseteado a {count: 0}. Siguiente ejecucion arranca con count = 0.

TEST-089: Grafo sin graph.memory — backward compat total
  Given: YAML existente sin seccion graph.memory
  When: Parsear y ejecutar
  Then: Funciona identico a hoy. Sin inyeccion, sin persistencia, sin nodo virtual memory.

### Journey: Developer — YAML Validation

TEST-090: Live agent sin schedule — error
  Given: YAML con agent_type: live, sin seccion schedule
  When: mirai validate agent.yaml
  Then: Error: "live agents require a schedule section"

TEST-091: Managed agent con schedule — error
  Given: YAML con agent_type: managed, con seccion schedule
  When: mirai validate agent.yaml
  Then: Error: "schedule is only valid for live agents"

TEST-092: Schedule con ambos interval_seconds y cron — error
  Given: YAML con schedule.interval_seconds = 60 Y schedule.cron = "* * * * *"
  When: mirai validate agent.yaml
  Then: Error: "schedule must have exactly one of: interval_seconds, cron"

TEST-093: YAML sin graph.memory — backward compat
  Given: YAML existente (v0.4.x) sin seccion graph.memory
  When: Parsear y ejecutar
  Then: Funciona identico a hoy. Sin inyeccion de memoria, sin persistencia.

TEST-094: Live agent YAML round-trip
  Given: YAML con agent_type, schedule, graph.memory (con persist y keys)
  When: from_yaml → to_yaml → from_yaml
  Then: Spec identico. schedule, memory.persist y memory.keys preservados.

---

## Fuera de Alcance

- **Cron parsing library**: v1 soporta solo `interval_seconds`. Cron se deja declarado en el spec pero su parsing se implementa en un PRD futuro. Validation: si cron present e interval_seconds ausente → error con mensaje "cron support coming soon, use interval_seconds".
- **Memory persistence a disco/DB**: v1 almacena MemoryStore en memoria (HashMap per agent_id). Si el server se reinicia, la memoria se pierde. Persistencia a SQLite/Postgres es PRD futuro.
- **Memory encryption**: los valores se almacenan en claro. Encryption at rest es PRD futuro.
- **Memory size limits**: v1 no limita el tamano del MemoryStore. Limits son PRD futuro.
- **Distributed scheduling**: v1 es single-process. Si hay multiples instancias del server, cada una tiene su propio scheduler. Distributed locking es PRD futuro.
- **state/memory tool avanzado**: v1 es write-only (persiste lo que recibe). Operaciones avanzadas (increment, append, merge) son PRD futuro.
- **Webhook-triggered live agents**: v1 live agents solo se disparan por schedule (interval). Trigger por evento externo es PRD futuro.

---

## Dependencias

- **No depende de PRD-006** (refactoring): los cambios son a AgentSpec, runner pre/post steps, Scheduler integration, y nuevos endpoints. Se puede implementar sobre la estructura actual.
- **No depende de PRD-007** (claude_code tool): tool type independiente.
- **Reutiliza Scheduler existente**: `runtime/scheduler.rs` se conecta al server y se extiende con cycle tracking.
- **Reutiliza trigger/schedule tool**: el tool ya produce triggered_at y run_count. Se le inyecta cycle_number via config.
- **Reutiliza EventEmitter**: se agregan nuevos EventType variants.

### Orden de implementacion recomendado

```
Fase 1 (foundation):
  1. AgentScheduleSpec struct + serde
  2. graph.memory en AgentGraphSpec + serde
  3. AgentSpec.schedule field + validation (schedule↔live rules)
  4. MemoryStore (in-memory HashMap per agent_id)
  5. Tests de parseo YAML round-trip

Fase 2 (memory):
  6. inject_memory en GraphRunner.run() (pre-walk)
  7. state/memory tool (persiste keys declarados)
  8. ${memory.key} resolution en resolve_expression
  9. Tests de memoria: inject, persist, round-trip, fallo

Fase 3 (cycle loop):
  10. RuntimeAgentStatus::Playing
  11. CycleRecord struct
  12. Scheduler ← connect to server AppState
  13. Cycle loop: inject memory → run graph → persist if ok → sleep → repeat
  14. play_agent / stop_agent en AgentRuntime
  15. Nuevos EventType: AgentPlaying, AgentStopped, CycleStarted/Completed/Failed
  16. Tests de ciclo: play, stop, error modes, max_cycles

Fase 4 (API + CLI):
  17. HTTP endpoints: play, stop, cycles, memory, delete memory
  18. GET /api/v1/agents/{id} enhanced response
  19. POST /api/v1/agents/{id}/execute → reject live agents
  20. CLI: mirai play
  21. CLI: mirai run → reject live agents
  22. Integration tests end-to-end
```
