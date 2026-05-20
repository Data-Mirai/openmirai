# FEAT-015 — Cost Control & Budgets

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-033

---

## Problem Statement

**Tipo**: Feature nueva (control de costos y presupuestos)
**Actor**: Usuario local — persona que opera agentes que consumen LLMs de pago y necesita controlar gasto.

Hoy Data Mirai Engine ejecuta LLM calls sin ningun control de costos. El nodo `ai/llm_call` dispara requests a Claude, GPT, Gemini o cualquier provider configurado, consume tokens, y el costo se acumula en la cuenta del provider sin que el usuario tenga visibilidad desde Data Mirai. Esto tiene tres problemas:

1. **Sin visibilidad de costos**: La tabla `execution_spans` tiene `cost_estimate` y `tokens_input/output`, pero estos datos no se agregan ni se muestran de forma util. El usuario tiene que ir al dashboard del provider (Anthropic Console, OpenAI Usage, etc.) para saber cuanto gasto. No hay vista consolidada por agente, por sesion o por periodo.

2. **Sin limites**: Un agente con un loop mal configurado o un prompt muy largo puede consumir cientos de dolares en minutos. No hay budgets, no hay circuit breakers, no hay forma de decir "este agente no puede gastar mas de $5/dia". El usuario descubre el gasto despues del hecho.

3. **Sin optimizacion automatica**: Cuando el costo se acerca al limite, el unico recurso es apagar el agente manualmente. No hay degradacion automatica (cambiar a modelo mas barato), no hay alertas, no hay forma de optimizar costo sin sacrificar disponibilidad.

Para agentes en produccion, el control de costos es tan critico como el control de errores.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Ver el costo exacto de cada session, cada agente y de toda la instancia, desglosado por provider/modelo
2. Definir budgets por agente (diario, mensual) y globales (mensual)
3. Recibir alertas cuando el gasto se acerca al limite (80%)
4. Configurar model degradation automatica cuando el budget esta en riesgo
5. Pausar agentes automaticamente cuando exceden su budget

---

## Features

### 15.1 — Cost Tracking

**Problema**: Los datos de costo existen (execution_spans.cost_estimate, tokens_input, tokens_output) pero no se agregan, no se indexan por agente/periodo, y no se muestran de forma util. El usuario no tiene forma rapida de responder "cuanto gaste esta semana con el agente X".

**Solucion**: Tabla dedicada `cost_entry` que registra cada llamada LLM con su costo calculado, mas funciones de agregacion por periodo. El costo se calcula al momento de la llamada usando precios configurables por modelo.

**Arquitectura**:
- `CostEntry` dataclass:
  - `id: str` — UUID
  - `session_id: str` — session donde ocurrio
  - `agent_id: str` — agente que hizo la llamada
  - `node_id: str` — nodo del grafo que ejecuto
  - `provider: str` — ollama, claude, openai, gemini, groq, openrouter
  - `model: str` — modelo especifico (claude-sonnet-4-20250514, gpt-4o, etc)
  - `tokens_in: int` — tokens de input
  - `tokens_out: int` — tokens de output
  - `cost_usd: float` — costo calculado en USD
  - `created_at: str` — timestamp
- `ModelPricing` dataclass:
  - `model_pattern: str` — pattern para match de modelo (ej: `claude-sonnet-*`, `gpt-4o*`)
  - `input_price_per_1m: float` — precio por 1M tokens de input en USD
  - `output_price_per_1m: float` — precio por 1M tokens de output en USD
  - `updated_at: str` — cuando se actualizo el precio
- `CostCalculator`:
  - `calculate(provider, model, tokens_in, tokens_out) -> float` — calcula costo en USD
  - Lee precios de tabla `model_pricing` con match por pattern
  - Si no hay precio para el modelo, usa precio generico del provider o 0 (con warning)
  - Ollama local: costo siempre 0 (no hay pricing)
- `CostAggregator`:
  - `by_agent(agent_id, period_start, period_end) -> AgentCostSummary`
  - `by_session(session_id) -> SessionCostSummary`
  - `global_summary(period_start, period_end) -> GlobalCostSummary`
  - `by_model(period_start, period_end) -> list[ModelCostSummary]`
- Integracion: el LLM adapter (FEAT-002) llama a CostCalculator despues de cada LLM call exitosa y persiste el CostEntry

**Entidades nuevas**:

Tabla `cost_entry` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK sessions(id) ON DELETE CASCADE, NOT NULL |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| node_id | TEXT | NOT NULL |
| provider | TEXT | NOT NULL |
| model | TEXT | NOT NULL |
| tokens_in | INTEGER | NOT NULL DEFAULT 0 |
| tokens_out | INTEGER | NOT NULL DEFAULT 0 |
| cost_usd | REAL | NOT NULL DEFAULT 0.0 |
| created_at | TEXT | NOT NULL |

CREATE INDEX idx_cost_entry_agent ON cost_entry(agent_id);
CREATE INDEX idx_cost_entry_session ON cost_entry(session_id);
CREATE INDEX idx_cost_entry_agent_date ON cost_entry(agent_id, created_at);

Tabla `model_pricing` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| provider | TEXT | NOT NULL |
| model_pattern | TEXT | NOT NULL |
| input_price_per_1m | REAL | NOT NULL |
| output_price_per_1m | REAL | NOT NULL |
| updated_at | TEXT | NOT NULL |
| UNIQUE(provider, model_pattern) |

**Contratos API**:
- `GET /api/costs/summary?period=day|week|month&agent_id={id?}` — resumen de costos. Response: `{ total_usd: float, by_provider: { provider: str, cost_usd: float }[], by_model: { model: str, cost_usd: float }[], period_start: str, period_end: str }`
- `GET /api/agents/{id}/costs?period=day|week|month` — costos del agente. Response: `{ total_usd: float, by_session: { session_id: str, cost_usd: float, tokens_total: int }[], by_model: { model: str, cost_usd: float }[], trend: { date: str, cost_usd: float }[] }`
- `GET /api/sessions/{id}/costs` — costos de una session. Response: `{ total_usd: float, entries: CostEntry[], by_node: { node_id: str, cost_usd: float, tokens: int }[] }`
- `GET /api/costs/pricing` — listar precios configurados. Response: `{ pricing: ModelPricing[] }`
- `PUT /api/costs/pricing` — actualizar precios. Body: `{ pricing: { provider, model_pattern, input_price_per_1m, output_price_per_1m }[] }`. Response: `{ pricing: ModelPricing[] }`

**Pantallas**:
- **Session detail → seccion "Costo"**:
  - Total USD de la session
  - Breakdown por nodo: tabla con node_id, tool_type, tokens_in, tokens_out, cost_usd
  - Breakdown por modelo
- **AgentDetail → tab "Costos"**:
  - Grafico de gasto diario (bar chart, ultimos 30 dias)
  - Total del periodo seleccionado
  - Top sessions por costo
  - Breakdown por modelo
- **Settings → seccion "Pricing"**:
  - Tabla editable de model_pricing
  - Precios pre-cargados para modelos populares (Claude Sonnet, GPT-4o, Gemini Pro, etc)
  - Boton "Reset to Defaults"

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-260 | Cada LLM call que retorna tokens DEBE generar un cost_entry. Sin excepciones | Hook post-LLM-call en adapter |
| REGLA-261 | Si no hay pricing para un modelo, cost_usd = 0 y se loguea warning. No se bloquea la ejecucion | Fallback en CostCalculator |
| REGLA-262 | Ollama (local) siempre cost_usd = 0. Se registra el cost_entry para tracking de tokens, pero costo es cero | Check de provider en calculator |
| REGLA-263 | Los precios son configurables y actualizables. Se proveen defaults para modelos populares en seed data | Seed en migracion + API de update |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/costs/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/costs/calculator.py` — CostCalculator
- CREAR `framework/src/datamirai_engine/costs/models.py` — CostEntry, ModelPricing dataclasses
- CREAR `app/server/datamirai_app/routes/costs.py` — endpoints de costos
- MODIFICAR `app/server/datamirai_app/database.py` — tablas cost_entry, model_pricing + seed data
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de costs
- MODIFICAR `framework/src/datamirai_engine/llm/adapter.py` — hook post-call para registrar cost_entry (o integracion con CostCalculator)
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de costs
- CREAR `app/web/src/components/costs/CostChart.tsx` — grafico de costos
- CREAR `app/web/src/components/costs/CostBreakdown.tsx` — breakdown tabla
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab Costos
- CREAR `app/web/src/app/settings/pricing/page.tsx` — pagina de pricing

---

### 15.2 — Budgets

**Problema**: Sin limites de gasto, un agente puede consumir ilimitadamente. El usuario descubre el exceso cuando le llega la factura del provider.

**Solucion**: Sistema de budgets configurables por agente y globales, con enforcement automatico. Al exceder el budget, el agente se pausa (soft limit) o se bloquea la ejecucion (hard limit).

**Arquitectura**:
- `Budget` dataclass:
  - `id: str` — UUID
  - `scope: str` — `agent` o `global`
  - `agent_id: str | None` — NULL para global
  - `limit_usd: float` — limite en USD
  - `period: str` — `daily` o `monthly`
  - `enforcement: str` — `soft` (pausa agente, permite override manual) o `hard` (bloquea ejecucion, no override)
  - `degradation_threshold: float` — porcentaje (0-1) en el que se activa model degradation (default 0.8)
  - `is_active: bool`
- `BudgetEnforcer`:
  - `check_budget(agent_id) -> BudgetStatus` — verifica estado del budget antes de ejecutar
  - `BudgetStatus`: `{ within_budget: bool, usage_usd: float, limit_usd: float, usage_percent: float, action: str }` donde action es `allow`, `degrade`, `pause`, `block`
  - Se ejecuta ANTES de cada LLM call (pre-hook en adapter)
  - Flujo de decision:
    1. Calcular gasto actual del periodo (query a cost_entry)
    2. Si < threshold → `allow`
    3. Si >= threshold pero < limit → `degrade` (cambiar a modelo mas barato)
    4. Si >= limit y enforcement=soft → `pause` (pausa agente, session actual termina)
    5. Si >= limit y enforcement=hard → `block` (cancela la session inmediatamente)
- `BudgetAlertService`:
  - Emite eventos via EventBus cuando se cruzan umbrales: 50%, 80%, 95%, 100%
  - Canal: `budgets:{agent_id}` o `budgets:global`
  - Eventos: `budget.warning`, `budget.critical`, `budget.exceeded`

**Entidades nuevas**:

Tabla `budget` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| scope | TEXT | NOT NULL (agent / global) |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NULL |
| limit_usd | REAL | NOT NULL |
| period | TEXT | NOT NULL (daily / monthly) |
| enforcement | TEXT | NOT NULL DEFAULT 'soft' |
| degradation_threshold | REAL | NOT NULL DEFAULT 0.8 |
| is_active | INTEGER | NOT NULL DEFAULT 1 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

CREATE INDEX idx_budget_agent ON budget(agent_id);
UNIQUE(scope, agent_id, period) — solo 1 budget por scope+agente+periodo

**Contratos API**:
- `GET /api/budgets` — listar todos los budgets. Response: `{ budgets: Budget[] }`
- `POST /api/budgets` — crear budget. Body: `{ scope, agent_id?, limit_usd, period, enforcement?, degradation_threshold? }`. Response: `{ budget: Budget }`
- `PATCH /api/budgets/{id}` — actualizar budget. Body: campos parciales. Response: `{ budget: Budget }`
- `DELETE /api/budgets/{id}` — eliminar budget. Response: `204`
- `GET /api/budgets/{id}/status` — estado actual del budget. Response: `{ budget: Budget, usage_usd: float, usage_percent: float, remaining_usd: float, period_start: str, period_end: str }`
- `GET /api/agents/{id}/budget-status` — estado de budgets del agente (convenience). Response: `{ budgets: { budget: Budget, usage_usd: float, usage_percent: float }[] }`

**Pantallas**:
- **AgentDetail → tab "Costos" → seccion "Budget"**:
  - Card con budget activo: barra de progreso con porcentaje de uso, color verde→amarillo→rojo
  - Formulario inline para crear/editar budget del agente
  - Indicador de enforcement (soft/hard badge)
  - Historial de alertas: lista de eventos budget.warning, budget.exceeded
- **Settings → seccion "Global Budget"**:
  - Config de budget global mensual
  - Barra de progreso con uso actual vs limite
  - Lista de agentes que mas contribuyen al gasto

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-264 | BudgetEnforcer se ejecuta ANTES de cada LLM call. Si retorna block, la session se cancela con error descriptivo | Pre-hook en LLM adapter |
| REGLA-265 | Budget check usa datos de cost_entry del periodo actual. No cache — siempre query fresco para precision | Query directo a cost_entry con rango de fechas |
| REGLA-266 | Solo 1 budget activo por (scope, agent_id, period). Crear uno nuevo desactiva el anterior | UNIQUE constraint + service logic |
| REGLA-267 | Enforcement soft: pausa el agente (status → paused). El usuario puede reactivar manualmente. Enforcement hard: bloquea ejecucion, el agente no puede ejecutar hasta nuevo periodo | Agent status update para soft, session cancel para hard |
| REGLA-268 | Alertas de budget se emiten via EventBus. No se almacenan en tabla aparte — se registran como eventos transitorios | EventBus emit |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/costs/budget.py` — Budget dataclass, BudgetEnforcer, BudgetAlertService
- CREAR `app/server/datamirai_app/routes/budgets.py` — endpoints CRUD + status
- MODIFICAR `app/server/datamirai_app/database.py` — tabla budget
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de budgets
- MODIFICAR `framework/src/datamirai_engine/llm/adapter.py` — pre-hook de BudgetEnforcer antes de LLM call
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de budgets
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — seccion budget en tab Costos

---

### 15.3 — Model Degradation

**Problema**: Cuando el gasto se acerca al limite, el unico recurso actual seria pausar el agente. Pero el agente puede estar haciendo trabajo importante — pausarlo no es ideal. Un approach mas inteligente es degradar a un modelo mas barato que mantenga la funcionalidad con menor costo.

**Solucion**: Configuracion de cadena de fallback por agente: modelo primario → modelo de fallback. Cuando el BudgetEnforcer detecta que se cruzo el threshold de degradacion, automaticamente sustituye el modelo en la siguiente LLM call.

**Arquitectura**:
- `DegradationConfig` dataclass:
  - `agent_id: str`
  - `rules: list[DegradationRule]`
- `DegradationRule`:
  - `primary_model: str` — modelo original (ej: `claude-sonnet-4-20250514`)
  - `fallback_model: str` — modelo mas barato (ej: `claude-haiku-4-20250514`)
  - `primary_provider: str` — provider del modelo original
  - `fallback_provider: str` — provider del fallback (puede ser diferente, ej: de Claude a Groq)
- `DegradationService`:
  - `get_effective_model(agent_id, requested_model, requested_provider) -> (model, provider)` — retorna el modelo a usar
  - Si no hay budget pressure → retorna el modelo original
  - Si budget >= threshold → retorna el fallback correspondiente
  - Si no hay regla de degradacion para el modelo solicitado → retorna el original (no degrada)
  - Log de cada degradacion para auditoria
- Integracion: el LLM adapter consulta DegradationService ANTES de hacer la llamada. Si degrada, la NormalizedResponse incluye `degraded: true, original_model: str`.

**Entidades nuevas**:

Tabla `degradation_rule` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| primary_provider | TEXT | NOT NULL |
| primary_model | TEXT | NOT NULL |
| fallback_provider | TEXT | NOT NULL |
| fallback_model | TEXT | NOT NULL |
| created_at | TEXT | NOT NULL |
| UNIQUE(agent_id, primary_provider, primary_model) |

CREATE INDEX idx_degradation_agent ON degradation_rule(agent_id);

**Contratos API**:
- `GET /api/agents/{id}/degradation` — listar reglas de degradacion del agente. Response: `{ rules: DegradationRule[] }`
- `PUT /api/agents/{id}/degradation` — configurar reglas (replace all). Body: `{ rules: { primary_provider, primary_model, fallback_provider, fallback_model }[] }`. Response: `{ rules: DegradationRule[] }`
- `GET /api/agents/{id}/degradation/log` — historial de degradaciones. Query params: `limit`. Response: `{ entries: { timestamp, session_id, node_id, from_model, to_model, reason }[] }`

**Pantallas**:
- **AgentDetail → tab "Costos" → seccion "Model Degradation"**:
  - Tabla editable: Primary Model → Fallback Model, con selectores de provider + modelo
  - Boton "+ Add Rule"
  - Indicador de estado: "Active" (si budget > threshold) o "Standby" (budget ok)
  - Historial reciente de degradaciones

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-269 | Degradation es transparente al grafo. El nodo no sabe que se le cambio el modelo. Solo ve el resultado | DegradationService opera en capa de adapter, no de tool |
| REGLA-270 | Cada degradacion se loguea con: timestamp, session_id, node_id, modelo original, modelo fallback, razon (budget X% de Y) | Structured log entry |
| REGLA-271 | Si el modelo fallback tambien esta en budget pressure, NO se degrada a un tercero. Se usa el fallback tal cual — evitar cascading degradation | Max 1 nivel de degradation |
| REGLA-272 | La degradacion se revierte automaticamente al inicio de cada nuevo periodo de budget (diario/mensual). No requiere accion manual | BudgetEnforcer recalcula en cada check |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/costs/degradation.py` — DegradationConfig, DegradationRule, DegradationService
- CREAR `app/server/datamirai_app/routes/degradation.py` — endpoints CRUD + log
- MODIFICAR `app/server/datamirai_app/database.py` — tabla degradation_rule
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de degradation
- MODIFICAR `framework/src/datamirai_engine/llm/adapter.py` — consultar DegradationService antes de LLM call
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para degradation
- CREAR `app/web/src/components/costs/DegradationConfig.tsx` — tabla editable de reglas

---

### 15.4 — UI: Tab "Costos" + Dashboard Global

**Problema**: Sin visualizacion, los datos de costo no se consumen. El usuario necesita ver graficos de gasto, configurar budgets, y monitorear degradacion desde la UI, tanto por agente como globalmente.

**Solucion**: Tab "Costos" en AgentDetail con graficos y config, mas dashboard global en Settings con vision consolidada.

**Pantallas** (detalle completo):
- **AgentDetail → tab "Costos"**:
  - Header: total gastado este mes (numero grande), trend vs mes anterior (flecha + porcentaje)
  - Grafico principal: bar chart de gasto diario (ultimos 30 dias), con hover mostrando desglose por modelo
  - Cards de metricas: gasto hoy, gasto esta semana, gasto este mes, promedio por session
  - Seccion "Budget":
    - Barra de progreso circular: porcentaje de uso del budget
    - Config inline: limit_usd, period, enforcement (toggle soft/hard)
    - Alertas activas (badges amarillo/rojo)
  - Seccion "Model Degradation":
    - Tabla de reglas primary→fallback
    - Estado actual (active/standby)
    - Log de degradaciones recientes
  - Seccion "Top Sessions":
    - Tabla con top 10 sessions por costo: session_id (link), fecha, costo, tokens, modelo
- **Settings → pagina "Costs Dashboard"**:
  - Card grande: gasto total del mes (numero prominente) + budget global (barra de progreso)
  - Grafico: stacked bar chart de gasto diario, stacked por agente (colores diferentes)
  - Ranking de agentes por gasto: tabla con nombre, gasto mes, gasto semana, trend
  - Breakdown por provider: pie chart o donut
  - Breakdown por modelo: tabla con modelo, tokens totales, costo total
  - Config de budget global
  - Config de pricing (link a settings/pricing)

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-273 | Los datos de costos se actualizan en real-time via WebSocket (canal `costs:{agent_id}` y `costs:global`). Cada cost_entry nuevo emite evento | EventBus post cost_entry insert |
| REGLA-274 | El tab Costos solo muestra datos si hay cost_entries. Si el agente nunca ejecuto LLM calls, muestra estado vacio con mensaje explicativo | Conditional render |

**Archivos a crear/modificar**:
- CREAR `app/web/src/components/costs/AgentCostDashboard.tsx` — dashboard completo del tab Costos del agente
- CREAR `app/web/src/components/costs/GlobalCostDashboard.tsx` — dashboard global
- CREAR `app/web/src/components/costs/BudgetProgress.tsx` — barra de progreso circular de budget
- CREAR `app/web/src/components/costs/CostTrendChart.tsx` — grafico de tendencia diario
- CREAR `app/web/src/components/costs/ProviderBreakdown.tsx` — pie/donut chart por provider
- CREAR `app/web/src/app/settings/costs/page.tsx` — pagina de dashboard global
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab Costos (si no esta ya de 15.1)

---

## Dependencias entre features

```
15.1 (Cost Tracking) ← independiente, se implementa primero (base de datos)
15.2 (Budgets) ← depende de 15.1 para calcular gasto actual
15.3 (Model Degradation) ← depende de 15.2 para saber cuando degradar
15.4 (UI) ← depende de 15.1 + 15.2 + 15.3 para mostrar todo
```

Orden de implementacion: 15.1 → 15.2 → 15.3 → 15.4

Dependencia externa: FEAT-002 (Multi-LLM) para LLM adapters donde se integran los hooks de cost tracking y budget enforcement.

---

## Entidades nuevas (resumen consolidado)

### cost_entry
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK sessions(id) ON DELETE CASCADE, NOT NULL |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| node_id | TEXT | NOT NULL |
| provider | TEXT | NOT NULL |
| model | TEXT | NOT NULL |
| tokens_in | INTEGER | NOT NULL DEFAULT 0 |
| tokens_out | INTEGER | NOT NULL DEFAULT 0 |
| cost_usd | REAL | NOT NULL DEFAULT 0.0 |
| created_at | TEXT | NOT NULL |

### model_pricing
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| provider | TEXT | NOT NULL |
| model_pattern | TEXT | NOT NULL |
| input_price_per_1m | REAL | NOT NULL |
| output_price_per_1m | REAL | NOT NULL |
| updated_at | TEXT | NOT NULL |

### budget
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| scope | TEXT | NOT NULL (agent / global) |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NULL |
| limit_usd | REAL | NOT NULL |
| period | TEXT | NOT NULL (daily / monthly) |
| enforcement | TEXT | NOT NULL DEFAULT 'soft' |
| degradation_threshold | REAL | NOT NULL DEFAULT 0.8 |
| is_active | INTEGER | NOT NULL DEFAULT 1 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### degradation_rule
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| primary_provider | TEXT | NOT NULL |
| primary_model | TEXT | NOT NULL |
| fallback_provider | TEXT | NOT NULL |
| fallback_model | TEXT | NOT NULL |
| created_at | TEXT | NOT NULL |

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-260 | Cada LLM call genera cost_entry sin excepcion | Hook post-call |
| REGLA-261 | Sin pricing para modelo → cost_usd=0 + warning | Fallback |
| REGLA-262 | Ollama siempre cost_usd=0 | Provider check |
| REGLA-263 | Precios configurables + defaults para modelos populares | Seed + API |
| REGLA-264 | BudgetEnforcer pre-LLM-call. Block → cancela session | Pre-hook |
| REGLA-265 | Budget check sin cache — query fresco | Direct query |
| REGLA-266 | 1 budget activo por (scope, agent, period) | UNIQUE + service |
| REGLA-267 | Soft → pausa agente. Hard → bloquea ejecucion | Status update / session cancel |
| REGLA-268 | Alertas via EventBus, no tabla | EventBus emit |
| REGLA-269 | Degradation transparente al grafo | Adapter layer |
| REGLA-270 | Cada degradacion se loguea | Structured log |
| REGLA-271 | Max 1 nivel de degradation. No cascading | Guard en service |
| REGLA-272 | Degradation se revierte al inicio de nuevo periodo | Recalculo en check |
| REGLA-273 | Costos actualizados real-time via WS | EventBus |
| REGLA-274 | Tab Costos vacio si no hay cost_entries | Conditional render |

---

## Notas de implementacion

- **execution_spans ya tiene cost_estimate y tokens**. El cost_entry NO reemplaza execution_spans — es una tabla dedicada a costos con mejor indexacion y datos de pricing resueltos. execution_spans se mantiene para tracing, cost_entry para costos.
- **Model pricing seed data**: se incluyen precios actualizados a mayo 2026 para Claude (Sonnet, Haiku, Opus), GPT (4o, 4o-mini, o3-mini), Gemini (Pro, Flash), Groq (Llama). Se marcan como "last updated" y el usuario puede actualizar.
- **Budget check es sincrono y rapido**. Es un SUM query sobre cost_entry con rango de fechas. Con el indice idx_cost_entry_agent_date, esto es O(log n) en SQLite. No deberia agregar latencia perceptible a cada LLM call.
- **Degradation NO cambia la config del nodo**. Es transparente — el adapter sustituye modelo en runtime y lo revierte internamente. El nodo sigue configurado con su modelo primary. Solo el adapter sabe que hubo degradacion.
- **Alertas via WebSocket**. Los canales `budgets:{agent_id}` y `costs:{agent_id}` se agregan al EventBus (ARCHITECTURE.md). La UI usa useReactive para mostrar toasts de warning/critical.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones, EventBus, WebSocket
- `docs/prd/draft/FEAT-002.md` — Multi-LLM adapters (integracion de hooks de cost/budget)
