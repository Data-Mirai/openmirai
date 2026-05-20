# FEAT-027 — Energy Metering + Execution Lifecycle

**Estado**: Draft
**Fecha**: 2026-05-18
**Reemplaza**: FEAT-015 (Cost Control & Budgets) — obsoleta
**Absorbe**: FEAT-022 §22.1 (Live View) — concepto absorbido en session/run detail

---

## Problem Statement

**Tipo**: Feature nueva (sistema de metering universal + control de ejecución)
**Actor**: Usuario local — persona que opera Data Mirai Engine y necesita visibilidad + control sobre consumo de recursos y ejecuciones activas.

Hoy Data Mirai Engine ejecuta operaciones (LLM calls, MCP calls, herramientas, compute) sin registro centralizado de consumo. FEAT-015 proponía tracking en USD solo para LLM calls — insuficiente. Necesitamos:

1. **Metering universal**: toda operación que consume recursos genera un registro atómico de consumo
2. **Energía como moneda**: abstracción propia que centraliza todos los costos en una sola unidad. Gap entre costo real y precio energía = margen
3. **3 capas de costo**: infra interna (lo que Datamirai hostea), servicios externos (LLM APIs, MCP providers), margen plataforma (orquestación)
4. **Control de ejecución**: stop/pause/resume de sessions y runs con UX embebida en detail pages
5. **Live View absorbida**: ejecución en vivo no es página separada — es estado natural del session/run detail cuando activo

---

## Objetivo

Cuando esté implementado:
1. Toda operación que consume recursos genera `energy_event` atómico (receipt facturable)
2. Session/run detail muestra ejecución en vivo con controles stop/pause/resume cuando activo
3. Tracking de energía consumida visible en tiempo real durante ejecución
4. Trazabilidad completa: cada unidad de energía → agente, session, nodo, operación, capa de costo
5. CEO puede ver por cliente: qué se pagó en infra, qué se pagó a terceros, cuánto queda de margen
6. En Cloud (futuro): balance de energía, compra de packs, sin energía = no opera

---

## Catálogo de Operaciones

Solo se mide lo que cuesta dinero a alguien:

| Operación | Capa de costo | Qué se mide | Costo real para Datamirai |
|---|---|---|---|
| `LLM_CALL` | EXTERNAL_SERVICE | tokens_in + tokens_out × rate del provider | Factura Anthropic/OpenAI/Google |
| `MCP_CALL` | EXTERNAL_SERVICE | por invocación a servicio tercero | Factura del provider MCP |
| `TOOL_EXEC` | INTERNAL_INFRA | por invocación de tool builtin | Compute del servidor |
| `COMPUTE_TIME` | INTERNAL_INFRA | segundos de proceso vivo (session/run activo) | Costo servidor donde corre |
| `STORAGE_OP` | INTERNAL_INFRA | bytes almacenados + operaciones I/O | Costo S3/R2/disco |
| `DB_OP` | INTERNAL_INFRA | queries + almacenamiento de datos del universo | Costo Postgres/hosting |
| — | PLATFORM_FEE | fee de orquestación (configurable) | Ganancia Datamirai |

**Fórmula energía por operación**: `energy = actual_cost(infra + external) + platform_margin`
**Fórmula energía por session**: `session_energy = Σ energy_events de todos los nodos ejecutados`

---

## Actores y Permisos

| Actor | Capability | Acción | Visibilidad |
|---|---|---|---|
| OWNER | `manage_energy_rates` | Configurar rates de conversión, ver breakdown 3 capas | Rates, balance, desglose infra/externo/margen, todos agentes |
| ADMIN | `view_energy_analytics` | Ver analytics de consumo | Balance, consumo por agente, breakdown sin margen |
| EDITOR | `view_energy_usage` | Ver consumo de sus ejecuciones | Energía de sus sessions/runs, sin breakdown de capa |
| VIEWER | — | Ver resúmenes | Total consumido, sin detalle |

**Capabilities nuevas**: `manage_energy_rates`, `view_energy_analytics`, `view_energy_usage`

En local (single-user): sin restricciones, todo visible.

---

## Entidades

### energy_event

Registro atómico de cada operación que consume recursos. Inmutable post-creación.

| Columna | Tipo | Default | Constraint |
|---|---|---|---|
| id | TEXT | — | PK |
| agent_id | TEXT | — | FK agents(id) ON DELETE CASCADE, NOT NULL |
| session_id | TEXT | NULL | FK sessions(id) ON DELETE CASCADE |
| run_id | TEXT | NULL | FK agent_runs(id) ON DELETE CASCADE |
| cycle_id | TEXT | NULL | FK agent_cycles(id) ON DELETE CASCADE |
| node_id | TEXT | NULL | Nodo que generó la operación |
| operation_type | TEXT | — | NOT NULL |
| cost_category | TEXT | — | NOT NULL |
| provider | TEXT | NULL | Nombre del provider (anthropic, openai, aws, etc.) |
| model | TEXT | NULL | Modelo/servicio específico |
| quantity | REAL | 0 | NOT NULL |
| quantity_unit | TEXT | — | NOT NULL |
| actual_cost_usd | REAL | 0 | NOT NULL — costo real para Datamirai |
| energy_charged | REAL | 0 | NOT NULL — energía cobrada al usuario |
| metadata | TEXT | '{}' | JSON con detalles específicos de la operación |
| created_at | TEXT | — | NOT NULL |

**Enums**:
- `operation_type`: LLM_CALL | MCP_CALL | TOOL_EXEC | COMPUTE_TIME | STORAGE_OP | DB_OP
- `cost_category`: INTERNAL_INFRA | EXTERNAL_SERVICE | PLATFORM_FEE
- `quantity_unit`: TOKENS_IN | TOKENS_OUT | BYTES | SECONDS | INVOCATIONS

**CHECK**: al menos uno de session_id, run_id, cycle_id debe ser NOT NULL

**Índices**:
- `idx_energy_event_agent` ON (agent_id)
- `idx_energy_event_session` ON (session_id)
- `idx_energy_event_run` ON (run_id)
- `idx_energy_event_agent_date` ON (agent_id, created_at)

---

### energy_rate

Rates de conversión por tipo de operación. Determinan cuánta energía se cobra por unidad consumida.

| Columna | Tipo | Default | Constraint |
|---|---|---|---|
| id | TEXT | — | PK |
| operation_type | TEXT | — | NOT NULL |
| provider | TEXT | NULL | NULL = default para tipo |
| model_pattern | TEXT | NULL | Pattern matching (claude-sonnet-*) |
| cost_per_unit_usd | REAL | 0 | NOT NULL — qué paga Datamirai por unidad |
| energy_per_unit | REAL | 0 | NOT NULL — qué se cobra al usuario por unidad |
| quantity_unit | TEXT | — | NOT NULL |
| is_active | INTEGER | 1 | NOT NULL |
| created_at | TEXT | — | NOT NULL |
| updated_at | TEXT | — | NOT NULL |

**UNIQUE**: (operation_type, provider, model_pattern, quantity_unit)
**Índice**: `idx_energy_rate_lookup` ON (operation_type, provider, is_active)

**Seed data**: rates para modelos populares (Claude Sonnet/Haiku/Opus, GPT-4o/4o-mini, Gemini Pro/Flash, Ollama=0 cost)

---

### energy_balance

Balance de energía del universo. Cloud = enforcement real. Local = informacional.

| Columna | Tipo | Default | Constraint |
|---|---|---|---|
| id | TEXT | — | PK |
| universe_id | TEXT | — | FK universes(id), NOT NULL, UNIQUE |
| available | REAL | 0 | NOT NULL — energía disponible |
| total_consumed | REAL | 0 | NOT NULL — consumo lifetime |
| total_purchased | REAL | 0 | NOT NULL — compras lifetime |
| updated_at | TEXT | — | NOT NULL |

En local: available = ∞ (sin enforcement).

---

### Tablas obsoletas (de FEAT-015, nunca implementadas)

- `cost_entry` → reemplazada por `energy_event`
- `model_pricing` → reemplazada por `energy_rate`
- `budget` → reemplazada por `energy_balance`
- `degradation_rule` → fuera de scope (optimización futura independiente)

---

## Ciclos de Vida

### Energy Balance (Cloud)

Estados: ACTIVE | LOW | DEPLETED | SUSPENDED

| From | To | Guard | Side-effect |
|---|---|---|---|
| ACTIVE | LOW | available < threshold (default 20%) | Emitir `energy.warning` via EventBus |
| LOW | DEPLETED | available = 0 | Emitir `energy.depleted`. Bloquear nuevas sessions/runs. Activas terminan nodo actual → pause |
| DEPLETED | ACTIVE | Compra exitosa | Emitir `energy.recharged`. Desbloquear ejecuciones |
| any | SUSPENDED | Admin action | Bloquear todo. Emitir `energy.suspended` |

En local: no aplica.

### Session lifecycle — side-effects de energía (sin cambios estructurales)

| Transición | Side-effect energía |
|---|---|
| pending → running | Registrar start_time para compute_time metering |
| running → completed | Crear energy_event(COMPUTE_TIME) con duración total. Emitir energy.session_total via WS |
| running → failed | Igual que completed (metering se cierra) |
| running → interrupted | Crear energy_event(COMPUTE_TIME) parcial hasta pause. Pausar metering |
| interrupted → running | Registrar nuevo start_time para reanudar metering |

### Agent Run — side-effects de energía

| Transición | Side-effect energía |
|---|---|
| started → running | Registrar start_time compute_time |
| running → completed | energy_event(COMPUTE_TIME) con duración total. Si stop_reason="user_stopped", incluir en metadata |
| running → failed | Igual |

Cada nodo que ejecuta (LLM, MCP, tool) genera su propio energy_event durante ejecución. COMPUTE_TIME es adicional.

---

## Pantallas

### SessionDetail (`/sessions/[id]`) — MODIFICADA

**Datos nuevos**: `useReactive<EnergyEvent[]>("/api/energy/events?session_id={id}", "energy:{id}")`
**Acciones nuevas**: pause, resume, stop

| Estado session | Controles visibles | Energy counter | Nodos |
|---|---|---|---|
| running | Stop + Pause | Ticking (real-time) | Progreso nodo-a-nodo con pulsing |
| interrupted | Resume + Stop | Congelado | Último nodo completado highlighted |
| completed | Ninguno | Total final | Todos con check verde |
| failed | Ninguno | Total hasta falla | Nodo fallido con X roja |

**Secciones nuevas**:
- **Header**: progress bar (completados/total), energy counter (número grande), botones control
- **Execution timeline**: nodo-a-nodo en tiempo real. Input → Processing → Output por nodo. Sin truncar
- **Energy breakdown**: tabla expandible: nodo, operation_type, provider, energy_charged, cost_category

**Compute time real-time**: frontend calcula `Σ(energy_events) + (elapsed_seconds × compute_rate)`. Rate disponible desde energy_rate config.

### RunDetail (`/agents/[id]/runs/[runId]`) — MODIFICADA

**Datos nuevos**: `useReactive<EnergyEvent[]>("/api/energy/events?run_id={runId}", "energy:run:{runId}")`
**Acciones nuevas**: stop

| Estado run | Controles | Energy |
|---|---|---|
| running | Stop | Ticking, breakdown por cycle |
| completed | Ninguno | Total final |
| failed | Ninguno | Total hasta falla |

**Secciones nuevas**:
- Tabla de cycles con columna "Energía" por cycle
- Energy total del run
- Clic en cycle → detalle nodo-a-nodo

### AgentDetail (`/agents/[id]`) — MODIFICADA

**Datos nuevos**: `useReactive<EnergySummary>("/api/agents/{id}/energy", "energy:agent:{id}")`

**Sección nueva "Energía"**:
- Total consumido (período: hoy/semana/mes)
- Bar chart: consumo diario últimos 30 días
- Breakdown por operation_type
- Breakdown por cost_category (3 capas)
- Top 5 sessions/runs por energía

### SettingsEnergyRates (`/settings/energy`) — NUEVA

- **Datos**: `useReactive<EnergyRate[]>("/api/energy/rates", "energy_rates")`
- **Acciones**: updateRate, createRate, deleteRate, resetDefaults
- **Estados UI**: skeleton → ready | empty (sin rates, botón "Load Defaults")
- **Componentes**: Tabla editable (operation_type, provider, model_pattern, cost_per_unit_usd, energy_per_unit, quantity_unit) + botón "Reset to Defaults"

### ELIMINADA: Live View

Live View como página/componente separado → eliminada. Ejecución en vivo = estado natural de SessionDetail/RunDetail cuando status=running.

---

## Contratos de API

### Energy Events

**GET /api/energy/events**
- **Query params**: session_id, run_id, cycle_id, agent_id, operation_type, period (day|week|month)
- **Response 200**: `{ events: EnergyEvent[], total_energy: float, total_cost_usd: float }`

**GET /api/energy/summary**
- **Query params**: agent_id (opcional), period (day|week|month)
- **Response 200**:
  ```
  {
    total_energy: float,
    total_cost_usd: float,
    by_operation_type: [{ type: str, energy: float, cost_usd: float }],
    by_cost_category: [{ category: str, energy: float, cost_usd: float }],
    by_agent: [{ agent_id: str, name: str, energy: float }],
    trend: [{ date: str, energy: float }]
  }
  ```

**GET /api/agents/{id}/energy**
- **Query params**: period (day|week|month)
- **Response 200**:
  ```
  {
    total_energy: float,
    avg_per_session: float,
    by_operation_type: [...],
    by_cost_category: [...],
    trend: [{ date: str, energy: float }],
    top_sessions: [{ session_id: str, energy: float, date: str }]
  }
  ```

### Energy Rates

**GET /api/energy/rates**
- **Response 200**: `{ rates: EnergyRate[] }`

**PUT /api/energy/rates**
- **Body**: `{ rates: [{ operation_type, provider?, model_pattern?, cost_per_unit_usd, energy_per_unit, quantity_unit }] }`
- **Response 200**: `{ rates: EnergyRate[] }`

**POST /api/energy/rates/reset**
- **Response 200**: `{ rates: EnergyRate[] }` — seed defaults restaurados

### Session Control

**POST /api/sessions/{id}/pause**
- **Guard**: session.status = running
- **Response 200**: `{ session: Session }` (status=interrupted)
- **Error 409**: session no está running

**POST /api/sessions/{id}/resume**
- **Guard**: session.status = interrupted
- **Response 200**: `{ session: Session }` (status=running)
- **Error 409**: session no está interrupted

**POST /api/sessions/{id}/stop**
- **Guard**: session.status ∈ (running, interrupted)
- **Response 200**: `{ session: Session }` (status=failed, error="cancelled_by_user")
- **Error 409**: session ya terminó

### Run Control

**POST /api/agent-runs/{id}/stop**
- **Guard**: run.status = running
- **Response 200**: `{ run: AgentRun }` (status=completed, stop_reason="user_stopped")
- **Error 409**: run no está running

### WebSocket — Canales nuevos

| Canal | Eventos |
|---|---|
| `energy:{session_id}` | energy.recorded, energy.total_updated |
| `energy:run:{run_id}` | energy.recorded, energy.cycle_total, energy.run_total |
| `energy:agent:{agent_id}` | energy.summary_updated |

Payload `energy.recorded`:
```json
{
  "event_id": "ee-123",
  "operation_type": "LLM_CALL",
  "energy_charged": 0.45,
  "node_id": "node-2",
  "session_total": 2.35
}
```

---

## Reglas de Negocio

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-39 | Toda operación que consume recursos externos/infra DEBE generar energy_event. Sin excepción | Hook post-operación en adapter/executor |
| REGLA-40 | energy_events son inmutables. Nunca UPDATE ni DELETE | Solo INSERT en capa de servicio |
| REGLA-41 | Cambio de energy_rate aplica solo a eventos futuros. Eventos pasados mantienen energy_charged original | Rate se resuelve al crear event |
| REGLA-42 | Sin energy_rate para operación → energy_charged=0 + warning en logs | Fallback en EnergyCalculator |
| REGLA-43 | Ollama (local) → actual_cost_usd=0 siempre. energy_charged según rate | Provider check en calculator |
| REGLA-44 | Pause congela compute_time metering. Resume reanuda | Timestamp tracking en session controller |
| REGLA-45 | Energy summary = SUM query sobre energy_events. Sin totales cacheados | Query directo |
| REGLA-46 | Cada energy_event DEBE referenciar al menos session_id, run_id, o cycle_id | CHECK constraint DB |
| REGLA-47 | energy_balance enforcement solo en Cloud. Local = sin límite | Config flag |
| REGLA-48 | Stop/pause espera que nodo actual complete. No interrumpe mid-execution | Guard en GraphRunner (REGLA-14) |
| REGLA-49 | Energy events se emiten al EventBus para display real-time via WebSocket | Emit post-INSERT |
| REGLA-50 | Stop session = failed con error "cancelled_by_user". Energía registrada hasta stop | Session controller |
| REGLA-51 | Stop run = completed con stop_reason "user_stopped". Cycle actual termina primero | Run controller |
| REGLA-52 | energy_rate seed data incluye rates para modelos populares. Actualizable por usuario | Migration seed |
| REGLA-53 | Energy se deduce del balance al COMPLETAR cada nodo, no al final de session. Deducción atómica inmediata | Post-node hook en GraphRunner |
| REGLA-54 | Pre-node check: si balance <= 0 → auto-interrupt (energy_depleted). Checkpoint guardado. Nodo siguiente NO inicia | Pre-node guard en GraphRunner |
| REGLA-55 | Resume de energy_depleted requiere balance > 0. Si balance=0, Resume bloqueado | Guard en resume endpoint |

**interrupt_type nuevo**: `energy_depleted` (se suma a `session_control` y `human_input_block` existentes)

**energy_balance enforcement**: configurable via flag. Local = off por default. Cloud = on. Tests pueden activarlo para simular depletion.

**Reglas obsoletas**: REGLA-260 a REGLA-274 de FEAT-015 → reemplazadas por REGLA-39 a REGLA-55.

---

## Escenarios GWT

### Journey: Energy Metering — Managed Session

TEST-086: Energy event por LLM call
  Given: Agente managed con nodo ai/llm_call configurado (Claude Sonnet)
  When: Session ejecuta y nodo LLM completa con 500 tokens in + 200 tokens out
  Then: Se crea energy_event con operation_type=LLM_CALL, provider=anthropic, cost_category=EXTERNAL_SERVICE
  And: energy_charged calculado según energy_rate activo para claude-sonnet-*

TEST-087: Energy event por MCP call
  Given: Agente con nodo MCP (WhatsApp send)
  When: Nodo MCP ejecuta exitosamente
  Then: Se crea energy_event con operation_type=MCP_CALL, cost_category=EXTERNAL_SERVICE, quantity=1, quantity_unit=INVOCATIONS

TEST-088: Energy event por compute_time
  Given: Session en status running
  When: Session completa (duración 45 segundos)
  Then: Se crea energy_event con operation_type=COMPUTE_TIME, quantity=45, quantity_unit=SECONDS, cost_category=INTERNAL_INFRA

TEST-089: Energía total de session = Σ energy_events
  Given: Session completada con 3 nodos (LLM + MCP + db_write)
  When: Usuario consulta energía de la session
  Then: Total = suma de todos los energy_events de esa session
  And: Desglose visible por operation_type y cost_category

TEST-090: Múltiples energy_events por nodo con loop
  Given: Agente con nodo LLM dentro de loop (max_iterations=3)
  When: Loop ejecuta 3 iteraciones
  Then: Se crean 3 energy_events independientes (uno por iteración)
  And: Total del nodo = suma de las 3

### Journey: Energy Metering — Live Agent Run/Cycles

TEST-091: Energy tracking por cycle
  Given: Live agent en ejecución, run activo
  When: Cycle #1 completa con 2 nodos (LLM + tool)
  Then: Se crean energy_events con cycle_id=cycle_1 y run_id=run_1

TEST-092: Energy total de run = Σ cycles
  Given: Run con 5 cycles completados
  When: Usuario consulta energía del run
  Then: Total = suma de energy_events de todos los cycles del run

TEST-093: Stop de run registra compute_time final
  Given: Live agent running, 3 cycles completados
  When: Usuario detiene el run
  Then: Se crea energy_event(COMPUTE_TIME) con duración desde start hasta stop
  And: Run status=completed, stop_reason="user_stopped"

### Journey: Execution Control — Session

TEST-094: Controles visibles en session detail cuando running
  Given: Session en status=running
  When: Usuario navega a session detail page
  Then: Botones Stop y Pause visibles en header
  And: Contador de energía consumida visible y actualizándose

TEST-095: Pause session → controles cambian
  Given: Session running, usuario en session detail
  When: Usuario hace clic en Pause
  Then: Session status → interrupted. Botón cambia a Resume + Stop
  And: Contador de energía se congela

TEST-096: Resume session → ejecución continúa
  Given: Session interrupted
  When: Usuario hace clic en Resume
  Then: Session status → running. Ejecución continúa desde checkpoint
  And: Contador de energía reanuda

TEST-097: Stop session → ejecución cancela
  Given: Session running
  When: Usuario hace clic en Stop
  Then: Session espera nodo actual → status=failed, error="cancelled_by_user"
  And: Energy events finales registrados

TEST-098: Session completada → controles desaparecen
  Given: Session en status=completed
  When: Usuario navega a session detail
  Then: No hay botones Stop/Pause/Resume
  And: Total de energía consumida como dato final

### Journey: Execution Control — Run

TEST-099: Controles visibles en run detail cuando running
  Given: Live agent con run activo (status=running)
  When: Usuario navega a run detail page
  Then: Botón Stop visible. Energía acumulada visible por cycle

TEST-100: Stop run → run finaliza
  Given: Run activo ejecutando cycle #4
  When: Usuario hace clic en Stop
  Then: Cycle actual termina → run status=completed, stop_reason="user_stopped"

### Journey: Energy Visibility en Detail Pages

TEST-101: Session detail muestra energy breakdown
  Given: Session completada con energy_events
  When: Usuario abre session detail
  Then: Sección "Energía" con total, breakdown por nodo, breakdown por cost_category

TEST-102: Energía en tiempo real via WebSocket
  Given: Session running, usuario en session detail
  When: Nodo LLM completa y genera energy_event
  Then: Contador se actualiza vía WebSocket sin refresh

TEST-103: Run detail muestra energía por cycle
  Given: Run con 3 cycles completados
  When: Usuario abre run detail
  Then: Tabla de cycles con columna "Energía" por cycle. Total run = Σ cycles

TEST-104: Agent detail muestra resumen de energía
  Given: Agente con sessions/runs históricos
  When: Usuario abre agent detail
  Then: Resumen: energía total, promedio por session/run, gráfico últimos 30 días

### Journey: Energy Rates Configuration

TEST-105: Ver rates configurados
  Given: Energy rates seed data cargado
  When: Usuario navega a Settings → Energy Rates
  Then: Tabla con rates por operation_type, provider, model_pattern, energy_per_unit

TEST-106: Actualizar rate de LLM
  Given: Rate activo para claude-sonnet-* = 0.003 energy/token
  When: Usuario cambia a 0.005 energy/token y guarda
  Then: Rate actualizado. Futuras LLM calls usan nuevo rate. Events anteriores inmutables

TEST-107: Rate sin configurar → energy = 0 + warning
  Given: Nodo usa modelo sin rate configurado
  When: Nodo ejecuta exitosamente
  Then: Energy_event con energy_charged=0. Warning en logs

### Journey: Edge Cases

TEST-108: Ollama (local) → actual_cost_usd = 0
  Given: Agente con LLM en Ollama (local)
  When: Nodo LLM ejecuta
  Then: Energy_event con actual_cost_usd=0. energy_charged según rate configurado

TEST-109: Session falla mid-execution → energía registrada hasta falla
  Given: Agente con 5 nodos, nodo #3 falla
  When: Session falla
  Then: Energy_events de nodos 1, 2, 3 registrados. 4, 5 sin events. Compute_time hasta falla

TEST-110: Session sin nodos que consumen energía
  Given: Agente solo con nodos logic (conditions, transforms)
  When: Session completa
  Then: Solo energy_event de COMPUTE_TIME. Energía total = solo compute_time

### Journey: Session State Machine — Edge Cases (transiciones inválidas + REGLA-48)

TEST-111: Pause en session pending → no disponible
  Given: Session en status=pending (esperando slot)
  When: Usuario navega a session detail
  Then: Botón Pause no visible. Solo indicador de "pending"

TEST-112: Resume en session running → no disponible
  Given: Session en status=running (no interrupted)
  When: Usuario navega a session detail
  Then: Botón Resume no visible. Solo Pause + Stop visibles

TEST-113: Stop en session completed → no acción
  Given: Session en status=completed
  When: Usuario navega a session detail
  Then: No hay botones de control. Solo datos finales de energía

TEST-114: Stop desde interrupted (pause → cancel)
  Given: Session en status=interrupted (pausada por usuario)
  When: Usuario hace clic en Stop
  Then: Session → failed, error="cancelled_by_user". Energy registrada hasta punto de pausa

TEST-115: Session timeout → energy registrada hasta timeout
  Given: Agente con timeout configurado (30s), nodo LLM tarda más
  When: Session excede timeout
  Then: Session status=timeout. Energy_events registrados hasta timeout. Compute_time parcial

TEST-116: Pause espera nodo complete (REGLA-48)
  Given: Session running, nodo LLM ejecutándose
  When: Usuario hace clic en Pause
  Then: UI muestra estado transitorio "Pausando...". Nodo LLM termina → session interrupted
  And: Energy_event del nodo LLM registrado ANTES de crear interrupt

TEST-117: Stop espera nodo complete (REGLA-48)
  Given: Session running, nodo MCP ejecutándose
  When: Usuario hace clic en Stop
  Then: UI muestra "Deteniendo...". Nodo MCP termina → session failed
  And: Energy_events completos hasta nodo que terminó

TEST-118: Doble pause → solo 1 interrupt (REGLA-16)
  Given: Session running, usuario hace clic en Pause
  When: Rápidamente hace clic en Pause otra vez
  Then: Solo 1 interrupt creado. Segundo clic ignorado o botón deshabilitado

TEST-119: Ciclo pause/resume múltiple → compute_time correcto
  Given: Session: running 10s → pause 5s → resume 10s → pause 5s → resume → completa en 10s
  When: Session completa
  Then: Compute_time total = 30s (solo tiempo activo, pausas no cuentan)
  And: Energy de compute = 30s × rate

### Journey: Run State Machine — Edge Cases

TEST-120: Stop en run completado → no disponible
  Given: Run en status=completed
  When: Usuario navega a run detail
  Then: Botón Stop no visible. Solo datos finales

TEST-121: Run falla durante cycle → energy registrada hasta falla
  Given: Live agent running, cycle #3 ejecutando, nodo falla
  When: Cycle falla → run status=failed
  Then: Energy_events de cycles 1, 2 y parcial de 3 registrados

TEST-122: Stop durante cycle → cycle completa primero (REGLA-51)
  Given: Live agent running, cycle #4 en progreso (nodo 2 de 3)
  When: Usuario hace clic en Stop
  Then: Cycle #4 termina completamente → run status=completed, stop_reason="user_stopped"
  And: Energy_events de cycle #4 completos

### Journey: Energy Rate Changes + Isolation

TEST-123: Rate cambia mid-session → events usan rate correcto (REGLA-41)
  Given: Rate claude-sonnet = 0.003 energy/token. Session running
  When: Admin cambia rate a 0.005. Siguiente nodo LLM ejecuta
  Then: Nodos pre-cambio: energy con rate 0.003. Post-cambio: rate 0.005
  And: Events anteriores inmutables

TEST-124: Rate energy_per_unit=0 → event registrado con 0 energy
  Given: Rate para tool_exec con energy_per_unit=0
  When: Nodo tool ejecuta
  Then: Energy_event creado con energy_charged=0. Aparece en breakdown

TEST-125: Múltiples sessions simultáneas → energy aislada
  Given: 2 agentes diferentes ejecutando sessions simultáneamente
  When: Ambas sessions completan
  Then: Energy_events de cada session solo referencian su session_id
  And: Totales independientes, sin contaminación cruzada

### Journey: Energy Visibility — Completeness

TEST-126: Energy breakdown muestra 3 capas de costo
  Given: Session completada con nodos LLM (external) + tool (internal) + platform_fee
  When: Usuario abre session detail → Energy
  Then: Breakdown: INTERNAL_INFRA, EXTERNAL_SERVICE, PLATFORM_FEE. Subtotales suman al total

TEST-127: Session detail histórica → energy final sin controles
  Given: Session completada hace 1 hora
  When: Usuario navega a session detail
  Then: Energy total final (no ticking). Breakdown completo. Sin botones control

TEST-128: WebSocket reconecta → energy counter sincroniza
  Given: Session running, energy counter ticking
  When: WebSocket se desconecta y reconecta
  Then: Energy counter se re-sincroniza via re-fetch. Sin saltos ni datos perdidos

### Journey: Energy Depletion + Resume desde Checkpoint

TEST-129: Energy depletes mid-session → auto-pause
  Given: Balance=2 energía. Agente con 5 nodos (~1 energía cada uno)
  When: Nodo 1 completa (balance→1), nodo 2 completa (balance→0)
  Then: Session auto-interrupts antes de nodo 3. Interrupt reason=energy_depleted
  And: Checkpoint guardado en completación de nodo 2

TEST-130: Recharge → resume desde checkpoint → session completa
  Given: Session interrupted (energy_depleted) en nodo 2 de 5
  When: Usuario recarga 3 energía (balance→3). Click Resume
  Then: Session reanuda desde nodo 3. Ejecuta nodos 3, 4, 5. Session completa
  And: Total energía consumida = 5 (2 pre-pause + 3 post-resume)

TEST-131: UI feedback energy depleted
  Given: Session auto-pausada por energy_depleted
  When: Usuario abre session detail
  Then: Banner "Energía agotada — recarga para continuar"
  And: Botón Resume deshabilitado. Energy counter muestra 0

TEST-132: Resume sin recarga → bloqueado
  Given: Session interrupted (energy_depleted), balance=0
  When: Usuario intenta Resume
  Then: Botón deshabilitado. Tooltip "Sin energía disponible"

TEST-133: Energía alcanza exacto → session completa
  Given: Balance=1 energía. Agente con 1 nodo restante
  When: Nodo ejecuta, consume 1, balance→0
  Then: Session completa exitosamente. No hay nodo siguiente que bloquear

TEST-134: Live run → energy depletes → run pausa
  Given: Live agent running, balance=1. Cycle necesita 2 energía
  When: Primer nodo del cycle completa (balance→0)
  Then: Run auto-pausa. Interrupt reason=energy_depleted

TEST-135: Recarga parcial → resume → depletes de nuevo
  Given: Session en nodo 2 de 5 (energy_depleted). Balance=0
  When: Recarga 1 unidad (balance→1). Resume. Nodo 3 completa (balance→0)
  Then: Session auto-pausa de nuevo antes de nodo 4. Segundo interrupt energy_depleted

TEST-136: Energy depletion muestra nodos completados vs pendientes
  Given: Session interrupted (energy_depleted) en nodo 3 de 5
  When: Usuario abre session detail
  Then: Nodos 1, 2 con check. Nodo 3 con indicador pause/lock. Nodos 4, 5 grayed out
  And: Progreso "2/5 completados — sin energía"

---

## Dependencias

- FEAT-001 (Session Control) — APIs pause/resume diseñadas, se implementan aquí
- FEAT-018 (WebSocket/EventBus) — infra de real-time, canales nuevos
- FEAT-021 (Agent Types, Runs/Cycles) — implementado, tablas agent_runs/agent_cycles existen

## Fuera de Scope

- Billing/compra de packs de energía — Cloud, Capa 3
- Model degradation automática por budget — feature independiente futura
- Comparación energía entre versiones de agente — futuro
- Multi-tenancy / auth en energy endpoints — Cloud

## Archivos a crear/modificar (estimación)

**Backend (Python)**:
- CREAR `framework/src/datamirai_engine/energy/__init__.py`
- CREAR `framework/src/datamirai_engine/energy/models.py` — EnergyEvent, EnergyRate dataclasses
- CREAR `framework/src/datamirai_engine/energy/calculator.py` — EnergyCalculator (rate lookup + cálculo)
- CREAR `framework/src/datamirai_engine/energy/recorder.py` — EnergyRecorder (persiste events + emite EventBus)
- CREAR `app/server/datamirai_app/routes/energy.py` — endpoints energy events, summary, rates
- MODIFICAR `app/server/datamirai_app/database.py` — tablas energy_event, energy_rate, energy_balance + seed data + migration
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint energy + endpoints session/run control
- MODIFICAR `framework/src/datamirai_engine/llm/adapter.py` — hook post-LLM-call → EnergyRecorder
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — hooks compute_time start/stop + energy en pause/resume
- CREAR `app/server/datamirai_app/routes/session_control.py` — pause/resume/stop endpoints
- CREAR `app/server/datamirai_app/routes/run_control.py` — stop endpoint

**Frontend (TypeScript/React)**:
- CREAR `app/web/src/components/energy/EnergyCounter.tsx` — contador real-time
- CREAR `app/web/src/components/energy/EnergyBreakdown.tsx` — tabla breakdown
- CREAR `app/web/src/components/energy/EnergyChart.tsx` — gráfico tendencia
- CREAR `app/web/src/components/execution/ExecutionControls.tsx` — botones stop/pause/resume
- CREAR `app/web/src/components/execution/ExecutionTimeline.tsx` — timeline nodo-a-nodo (reemplaza LiveCanvas concept)
- CREAR `app/web/src/app/settings/energy/page.tsx` — página energy rates
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — agregar energy + controls + timeline
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar sección energía + run detail
- MODIFICAR `app/web/src/lib/api.ts` — tipos + funciones energy

**Tests (Playwright)**:
- CREAR `app/e2e/tests/energy-metering.spec.ts` — TEST-086 a TEST-093
- CREAR `app/e2e/tests/execution-control.spec.ts` — TEST-094 a TEST-100
- CREAR `app/e2e/tests/energy-visibility.spec.ts` — TEST-101 a TEST-107
- CREAR `app/e2e/tests/energy-edge-cases.spec.ts` — TEST-108 a TEST-110

---

## doc_refs

- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, EventBus, WebSocket, convenciones
- `docs/producto/FLUJOS.md` — máquinas de estado, reglas existentes
- `docs/producto/DOMINIO.md` — roles, capabilities
- `docs/backend/API.md` — WebSocket protocol, canales
- `docs/prd/draft/FEAT-001.md` — session control (APIs que se implementan aquí)
- `docs/prd/draft/FEAT-015.md` — OBSOLETA, reemplazada por esta feature
- `docs/prd/draft/FEAT-022.md` — §22.1 Live View absorbida por esta feature
