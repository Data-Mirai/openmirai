# FEAT-012 — Agent Testing & Evaluation

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-030

---

## Problem Statement

**Tipo**: Feature nueva (framework de testing para agentes)
**Actor**: Usuario local — persona que desarrolla y opera agentes en Data Mirai Engine.

Hoy no existe forma sistematica de validar que un agente funciona correctamente antes de desplegarlo. El usuario ejecuta el agente manualmente, revisa el output visualmente y decide si "se ve bien". Esto tiene tres problemas criticos:

1. **Sin testing determinístico**: Los agentes usan LLMs que son no-determinísticos. Si el usuario cambia un prompt, un data_map o una configuracion de nodo, no tiene forma de saber si rompio algo. No hay regression testing. No hay forma de ejecutar el grafo con datos conocidos y validar que el output cumple expectativas.

2. **Sin metricas de evaluacion**: No hay medicion objetiva de calidad. El usuario no sabe cuantos pasos toma su agente, cuantos tokens consume, cuanto tarda, ni si la calidad del output mejoro o empeoro entre versiones. No hay baseline ni comparacion historica.

3. **Sin mocking de dependencias**: Para testear un agente hay que ejecutar LLMs reales (costo, latencia, no-determinismo), hacer llamadas reales a DB/storage, y depender de servicios externos. No hay forma de aislar el grafo de sus dependencias para testing rapido y determinístico.

Sin testing formal, los agentes son cajas negras que el usuario opera con fe. Esto es inaceptable para produccion.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Definir test cases con inputs y outputs esperados para cualquier agente
2. Ejecutar test suites que corren el grafo completo con datos mock y reportan pass/fail
3. Ver metricas de evaluacion por ejecucion: steps, tokens, latencia, success rate
4. Comparar runs contra baselines para detectar regresiones
5. Usar mock providers para testing determinístico sin costo de LLM
6. Gestionar tests desde la UI en el tab "Tests" de cada agente

---

## Features

### 12.1 — Test Runner

**Problema**: No hay infraestructura para ejecutar un agente con datos controlados y validar su output. El unico camino es ejecucion manual con datos reales, que es lento, costoso y no-reproducible.

**Solucion**: Test runner que ejecuta el grafo completo (o parcial) con inputs definidos por el usuario, inyectando mocks donde se configure, y valida outputs contra assertions definidas. Integrado con el GraphRunner existente del framework.

**Arquitectura**:
- `TestCase` dataclass:
  - `id: str` — UUID
  - `agent_id: str` — agente bajo test
  - `name: str` — nombre descriptivo
  - `description: str` — que valida este test
  - `input_data: dict` — datos del trigger (reemplazan trigger_data real)
  - `mock_config: dict` — configuracion de mocks por nodo (ver 12.3)
  - `assertions: list[Assertion]` — validaciones sobre el output
  - `max_duration_ms: int | None` — timeout para el test (None = sin limite)
  - `tags: list[str]` — para filtrar y agrupar
- `Assertion` dataclass:
  - `target: str` — path al valor a validar (ej: `output.summary`, `state.node_3.result`)
  - `operator: str` — tipo de validacion: `equals`, `contains`, `matches_regex`, `matches_schema`, `is_type`, `greater_than`, `less_than`, `is_not_empty`
  - `expected: Any` — valor esperado o schema JSON
- `TestSuite` dataclass:
  - `id: str` — UUID
  - `agent_id: str` — agente
  - `name: str` — nombre de la suite
  - `description: str`
  - `test_case_ids: list[str]` — IDs de test cases incluidos
  - `run_mode: str` — `sequential` (default) o `parallel`
- `TestRun` — resultado de ejecutar una suite o test individual:
  - `id: str` — UUID
  - `suite_id: str | None` — NULL si es un test case individual
  - `agent_id: str`
  - `status: str` — `running`, `passed`, `failed`, `error`
  - `started_at, finished_at, duration_ms`
  - `results: list[TestResult]`
- `TestResult` — resultado individual por test case:
  - `id: str` — UUID
  - `test_run_id: str`
  - `test_case_id: str`
  - `status: str` — `passed`, `failed`, `error`, `skipped`
  - `assertion_results: list[AssertionResult]`
  - `session_id: str | None` — session creada para la ejecucion (para ver trace)
  - `duration_ms: int`
  - `error_message: str | None`
- `AssertionResult`:
  - `assertion: Assertion` — la assertion evaluada
  - `passed: bool`
  - `actual_value: Any` — valor real encontrado
  - `message: str` — descripcion del resultado
- `TestRunnerService`:
  - `run_test_case(test_case, agent) -> TestResult` — ejecuta un test case individual
  - `run_suite(suite, agent) -> TestRun` — ejecuta todos los test cases de una suite
  - Internamente: crea una session real del agente con trigger_data del test case, inyecta mocks via mock_config, ejecuta el grafo, evalua assertions contra el state final
  - Cada ejecucion crea una session real (visible en historial) con tag `source: test`

**Entidades nuevas**:

Tabla `test_case` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| input_data | TEXT | JSON, NOT NULL DEFAULT '{}' |
| mock_config | TEXT | JSON, DEFAULT '{}' |
| assertions | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| max_duration_ms | INTEGER | NULL |
| tags | TEXT | JSON array, DEFAULT '[]' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Tabla `test_suite` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| test_case_ids | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| run_mode | TEXT | NOT NULL DEFAULT 'sequential' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Tabla `test_run` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| suite_id | TEXT | FK test_suite(id) ON DELETE SET NULL, NULL |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| status | TEXT | NOT NULL DEFAULT 'running' |
| total_cases | INTEGER | NOT NULL DEFAULT 0 |
| passed_cases | INTEGER | NOT NULL DEFAULT 0 |
| failed_cases | INTEGER | NOT NULL DEFAULT 0 |
| error_cases | INTEGER | NOT NULL DEFAULT 0 |
| started_at | TEXT | NOT NULL |
| finished_at | TEXT | NULL |
| duration_ms | INTEGER | NULL |

Tabla `test_result` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| test_run_id | TEXT | FK test_run(id) ON DELETE CASCADE, NOT NULL |
| test_case_id | TEXT | FK test_case(id) ON DELETE SET NULL, NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| status | TEXT | NOT NULL DEFAULT 'running' |
| assertion_results | TEXT | JSON array, DEFAULT '[]' |
| duration_ms | INTEGER | NULL |
| error_message | TEXT | NULL |
| created_at | TEXT | NOT NULL |

CREATE INDEX idx_test_case_agent ON test_case(agent_id);
CREATE INDEX idx_test_suite_agent ON test_suite(agent_id);
CREATE INDEX idx_test_run_agent ON test_run(agent_id);
CREATE INDEX idx_test_result_run ON test_result(test_run_id);

**Contratos API**:
- `GET /api/agents/{id}/tests/cases` — listar test cases del agente. Query params: `tags`, `limit`, `offset`. Response: `{ cases: TestCase[], total: int }`
- `POST /api/agents/{id}/tests/cases` — crear test case. Body: `{ name, description?, input_data, mock_config?, assertions, max_duration_ms?, tags? }`. Response: `{ case: TestCase }`
- `GET /api/agents/{id}/tests/cases/{caseId}` — detalle de test case. Response: `{ case: TestCase }`
- `PATCH /api/agents/{id}/tests/cases/{caseId}` — actualizar test case. Body: campos parciales. Response: `{ case: TestCase }`
- `DELETE /api/agents/{id}/tests/cases/{caseId}` — eliminar test case. Response: `204`
- `GET /api/agents/{id}/tests/suites` — listar test suites. Response: `{ suites: TestSuite[] }`
- `POST /api/agents/{id}/tests/suites` — crear test suite. Body: `{ name, description?, test_case_ids, run_mode? }`. Response: `{ suite: TestSuite }`
- `PATCH /api/agents/{id}/tests/suites/{suiteId}` — actualizar suite. Response: `{ suite: TestSuite }`
- `DELETE /api/agents/{id}/tests/suites/{suiteId}` — eliminar suite. Response: `204`
- `POST /api/agents/{id}/tests/cases/{caseId}/run` — ejecutar un test case individual. Response: `{ run: TestRun }` (con status `running`, polling o WS para resultado)
- `POST /api/agents/{id}/tests/suites/{suiteId}/run` — ejecutar suite completa. Response: `{ run: TestRun }`
- `GET /api/agents/{id}/tests/runs` — listar test runs del agente. Query params: `limit`, `offset`. Response: `{ runs: TestRun[], total: int }`
- `GET /api/agents/{id}/tests/runs/{runId}` — detalle de un test run con resultados. Response: `{ run: TestRun, results: TestResult[] }`

**Pantallas**:
- **AgentDetail → tab "Tests"** (`/agents/[id]` con tab activo):
  - Seccion "Test Cases": lista de test cases con nombre, tags como badges, ultimo resultado (pass/fail/never run). Boton "New Test Case" abre formulario. Boton "Run" por test case individual.
  - Seccion "Test Suites": lista de suites con nombre, cantidad de cases, ultimo resultado. Boton "New Suite" abre formulario. Boton "Run Suite".
  - Seccion "Test Runs" (historial): lista de runs ordenados por fecha, con status badge (passed verde, failed rojo, running spinner), duracion, breakdown de passed/failed/error. Click en run expande resultados individuales.
- **Test Case Form** (modal o panel lateral):
  - Campo nombre + descripcion
  - Editor JSON para input_data (con syntax highlighting)
  - Editor de assertions: selector de target (dropdown con nodos del grafo), selector de operator, campo de expected value
  - Editor JSON para mock_config (opcional, avanzado)
  - Campo max_duration_ms
  - Tags (input con chips)
- **Test Result Detail** (expandido dentro del run):
  - Status por assertion con checkmark/cross
  - Actual vs expected side by side
  - Link a la session creada para ver trace completo

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-200 | Cada test case execution crea una session real con tag `source: test`. La session es visible en historial y tiene trace completo | TestRunnerService crea session via API interna |
| REGLA-201 | Los test cases no modifican el agente ni sus memorias. Las sessions de test son read-only respecto al estado del agente | Flag `test_mode: true` en session que previene escritura a agent_memory |
| REGLA-202 | Si un test case excede max_duration_ms, el status es `error` con mensaje `Timeout exceeded: {max_duration_ms}ms`. La session se cancela | asyncio.wait_for con timeout en TestRunnerService |
| REGLA-203 | Las assertions se evaluan sobre el SharedState final del grafo. El target usa dot notation para navegar el state (ej: `nodes.llm_1.output.response`) | Assertion evaluator con resolver de paths |
| REGLA-204 | Un TestRun es `passed` solo si TODOS los TestResults son `passed`. Si uno falla, el run es `failed`. Si uno tiene error, el run es `error` | Logica de agregacion en TestRunnerService |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/testing/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/testing/models.py` — TestCase, TestSuite, Assertion, TestRun, TestResult, AssertionResult dataclasses
- CREAR `framework/src/datamirai_engine/testing/runner.py` — TestRunnerService
- CREAR `framework/src/datamirai_engine/testing/assertions.py` — evaluador de assertions con todos los operators
- CREAR `app/server/datamirai_app/routes/tests.py` — endpoints CRUD + run
- MODIFICAR `app/server/datamirai_app/database.py` — tablas test_case, test_suite, test_run, test_result + migracion
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de tests
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de tests
- CREAR `app/web/src/app/agents/[id]/tests/page.tsx` — pagina UI del tab Tests (o integrar como tab en agent detail)
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab Tests

---

### 12.2 — Evaluation Metrics

**Problema**: Cuando un agente ejecuta, se pierde informacion valiosa sobre la calidad de la ejecucion. El usuario ve si "funciono" o "fallo" pero no tiene metricas cuantitativas: cuantos pasos tomo, cuantos tokens consumio, que tan rapido fue, si la calidad del output mejoro o empeoro vs ejecuciones anteriores. Sin metricas no hay optimizacion posible.

**Solucion**: Sistema de metricas que se calcula automaticamente por cada test run usando datos de la session (trace, execution_spans). Incluye metricas de trayectoria (como llego al resultado), metricas de outcome (que tan bueno es el resultado), y deteccion de regresiones comparando contra baseline.

**Arquitectura**:
- `TrajectoryMetrics` dataclass:
  - `total_steps: int` — nodos ejecutados
  - `unique_tools_used: list[str]` — tool_types unicos ejecutados
  - `retry_count: int` — nodos que hicieron retry
  - `branch_decisions: list[dict]` — decisiones en nodos condition/switch
  - `total_tokens_input: int` — suma de tokens input de todos los spans LLM
  - `total_tokens_output: int` — suma de tokens output
  - `total_cost_estimate: float` — suma de cost_estimate de spans
- `OutcomeMetrics` dataclass:
  - `success: bool` — session termino en completed
  - `duration_ms: int` — duracion total
  - `output_size: int` — tamano del output en caracteres
  - `assertions_passed: int` — de las assertions del test
  - `assertions_total: int`
  - `quality_score: float | None` — score de LLM judge (0-1), None si no se configuro
- `RegressionReport` dataclass:
  - `baseline_run_id: str` — run contra el que se compara
  - `deltas: dict` — diferencias en metricas clave (duration_ms, total_tokens, cost, assertions_passed)
  - `regression_detected: bool` — True si alguna metrica empeoro significativamente (>10% degradacion)
  - `improvements: list[str]` — metricas que mejoraron
  - `regressions: list[str]` — metricas que empeoraron
- `MetricsCalculator`:
  - `calculate_trajectory(session) -> TrajectoryMetrics` — lee execution_spans y trace de la session
  - `calculate_outcome(session, test_result) -> OutcomeMetrics` — combina session status con assertion results
  - `compare_with_baseline(current_run, baseline_run) -> RegressionReport` — compara metricas
  - `evaluate_quality(output, criteria, llm_adapter) -> float` — usa LLM judge para evaluar calidad (opcional)
- Las metricas se almacenan como JSON en el campo `metrics` de `test_run` y `test_result`

**Entidades nuevas**:

Columna adicional en `test_run`:

| Columna | Tipo | Constraint |
|---|---|---|
| metrics | TEXT | JSON, DEFAULT '{}' |
| baseline_run_id | TEXT | FK test_run(id), NULL |

Columna adicional en `test_result`:

| Columna | Tipo | Constraint |
|---|---|---|
| trajectory_metrics | TEXT | JSON, DEFAULT '{}' |
| outcome_metrics | TEXT | JSON, DEFAULT '{}' |

**Contratos API**:
- `GET /api/agents/{id}/tests/runs/{runId}/metrics` — metricas agregadas del run. Response: `{ trajectory: TrajectoryMetrics, outcome: OutcomeMetrics, regression: RegressionReport | null }`
- `POST /api/agents/{id}/tests/runs/{runId}/set-baseline` — marcar este run como baseline para comparaciones futuras. Response: `{ baseline_run_id: str }`
- `GET /api/agents/{id}/tests/metrics/trend` — tendencia de metricas en los ultimos N runs. Query params: `limit` (default 20). Response: `{ runs: { run_id, date, duration_ms, tokens_total, cost, pass_rate }[] }`

**Pantallas**:
- **Test Run Detail → seccion Metrics**:
  - Cards con metricas clave: duration, tokens, cost, pass rate
  - Trajectory breakdown: tabla de steps con herramienta usada, duracion, tokens
  - Regression indicators: flecha verde (mejora) o roja (regresion) vs baseline
  - Boton "Set as Baseline"
- **Agent Tests → Trends** (sub-seccion):
  - Grafico de linea: pass rate over time
  - Grafico de linea: duration/tokens/cost over time
  - Indicador de tendencia: mejorando / estable / degradando

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-205 | TrajectoryMetrics se calcula exclusivamente de execution_spans y trace. No ejecuta nada adicional | MetricsCalculator lee datos existentes |
| REGLA-206 | Quality score via LLM judge es opt-in (requiere configuracion explicita). No se ejecuta por defecto — tiene costo | Flag en test_case.mock_config |
| REGLA-207 | Regression se detecta con threshold configurable (default 10%). Solo se reporta si hay baseline definido | Comparacion relativa en MetricsCalculator |
| REGLA-208 | Las metricas son immutables. Una vez calculadas para un run, no cambian | Se calculan al finalizar el run y se persisten en JSON |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/testing/metrics.py` — MetricsCalculator, TrajectoryMetrics, OutcomeMetrics, RegressionReport dataclasses
- MODIFICAR `framework/src/datamirai_engine/testing/runner.py` — integrar MetricsCalculator al finalizar cada run
- MODIFICAR `app/server/datamirai_app/routes/tests.py` — agregar endpoints de metrics y trend
- MODIFICAR `app/server/datamirai_app/database.py` — columnas metrics, baseline_run_id en test_run; trajectory_metrics, outcome_metrics en test_result

---

### 12.3 — Mock Data Providers

**Problema**: Para testear un agente hay que llamar LLMs reales (costoso, lento, no-determinístico), consultar DBs reales (requieren datos), y acceder a storage real. Esto hace que los tests sean fragiles, caros y no-reproducibles. Sin mocking, testing determinístico es imposible.

**Solucion**: Sistema de mock providers que reemplazan los recursos reales durante testing. El usuario define mocks por nodo en el test case. Incluye mock LLM (respuestas fijas), mock DB (datos predefinidos), y snapshot replay (capturar ejecucion real y reproducirla).

**Arquitectura**:
- `MockProvider` (ABC):
  - `get_response(node_id, config) -> Any` — retorna la respuesta mock para un nodo
- `MockLLMProvider(MockProvider)`:
  - Recibe mapa de `node_id -> response_text`
  - Cuando el GraphRunner ejecuta un nodo `ai/llm_call`, intercepta y retorna el texto fijo en vez de llamar al LLM real
  - Soporta respuestas multiples para mismo nodo (para loops): lista de responses que se consumen en orden
- `MockDBProvider(MockProvider)`:
  - Recibe mapa de `node_id -> { rows: list[dict] }` para db_read, `node_id -> { affected: int }` para db_write
  - Intercepta nodos `data/db_read` y `data/db_write`
- `MockStorageProvider(MockProvider)`:
  - Recibe mapa de `node_id -> { content: str }` para storage_read, `node_id -> { url: str }` para storage_write
- `SnapshotCapture`:
  - Captura inputs/outputs de CADA nodo durante una ejecucion real
  - Genera un `MockSnapshot` que puede usarse como mock_config completo de un test case
  - Permite "grabar" una ejecucion exitosa y "reproducirla" como test de regresion
- `MockSnapshot`: `{ captures: { [node_id]: { inputs: dict, output: Any, duration_ms: int } } }`
- `MockInjector`:
  - Se integra con el ExecutionContext
  - Antes de ejecutar cada nodo, verifica si hay mock configurado para ese nodo
  - Si hay mock, retorna el mock response sin ejecutar el bloque real
  - Si no hay mock, ejecuta normalmente

**Entidades nuevas**:

Tabla `test_snapshot` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| captures | TEXT | JSON, NOT NULL |
| created_at | TEXT | NOT NULL |

CREATE INDEX idx_test_snapshot_agent ON test_snapshot(agent_id);

**Contratos API**:
- `POST /api/agents/{id}/tests/snapshots/capture` — capturar snapshot de una session existente. Body: `{ session_id, name, description? }`. Response: `{ snapshot: TestSnapshot }`
- `GET /api/agents/{id}/tests/snapshots` — listar snapshots del agente. Response: `{ snapshots: TestSnapshot[] }`
- `GET /api/agents/{id}/tests/snapshots/{snapshotId}` — detalle de snapshot. Response: `{ snapshot: TestSnapshot }`
- `DELETE /api/agents/{id}/tests/snapshots/{snapshotId}` — eliminar snapshot. Response: `204`
- `POST /api/agents/{id}/tests/snapshots/{snapshotId}/to-test-case` — crear test case desde snapshot. Body: `{ name, assertions? }`. Response: `{ case: TestCase }` (con mock_config pre-poblado del snapshot)

**Pantallas**:
- **Agent Tests → sub-seccion "Snapshots"**:
  - Lista de snapshots con nombre, session de origen, fecha
  - Boton "Capture from Session" → selector de sessions recientes del agente
  - Por snapshot: boton "Create Test Case" que abre formulario pre-poblado
  - Preview del snapshot: arbol colapsable de nodos con sus inputs/outputs capturados

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-209 | MockInjector SOLO opera cuando hay mock_config en el test case. Ejecuciones normales (no-test) nunca usan mocks | Check de flag test_mode en MockInjector |
| REGLA-210 | Si un nodo tiene mock configurado, se SALTA completamente la ejecucion real. No hay ejecucion parcial | MockInjector intercepta antes del execute() del bloque |
| REGLA-211 | Los snapshots capturan inputs/outputs tal cual fueron en la session original. No se modifican. Son inmutables | Captura directa de execution_trace |
| REGLA-212 | Un snapshot convertido a test case hereda TODOS los outputs como mocks y el trigger_data como input_data. El usuario agrega assertions manualmente | Conversion automatica en endpoint |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/testing/mocks.py` — MockProvider ABC, MockLLMProvider, MockDBProvider, MockStorageProvider, MockInjector
- CREAR `framework/src/datamirai_engine/testing/snapshots.py` — SnapshotCapture, MockSnapshot
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — hook point para MockInjector antes de ejecutar cada nodo
- MODIFICAR `app/server/datamirai_app/routes/tests.py` — endpoints de snapshots
- MODIFICAR `app/server/datamirai_app/database.py` — tabla test_snapshot

---

### 12.4 — UI: Tab "Tests" en AgentDetail

**Problema**: Sin UI dedicada, el testing queda relegado a API calls manuales y nunca se adopta. El usuario necesita una interfaz visual para crear, editar, ejecutar y monitorear tests sin salir del contexto del agente.

**Solucion**: Tab "Tests" dentro de la pagina de detalle del agente con tres secciones: Test Cases (CRUD + run), Test Suites (agrupar + run), y History (runs con resultados, metricas, trends).

**Pantallas** (detalle completo):
- **Tab Tests → vista principal**:
  - Header con metricas globales: total test cases, last run result (badge), pass rate trend (sparkline)
  - Tres sub-tabs: "Cases", "Suites", "History"
- **Sub-tab Cases**:
  - Tabla con columnas: Name, Tags (badges), Assertions (count), Last Result (badge pass/fail/never), Last Duration, Actions (run, edit, delete)
  - Boton "+ New Test Case" en header
  - Row click abre detail panel
  - Bulk actions: "Run Selected", "Delete Selected"
- **Sub-tab Suites**:
  - Tabla con columnas: Name, Cases (count), Mode (sequential/parallel badge), Last Result, Actions
  - Boton "+ New Suite" en header
  - Drag-and-drop para reordenar cases dentro de suite
- **Sub-tab History**:
  - Timeline de test runs con: fecha, suite/case name, status badge, duration, pass/fail breakdown bar
  - Click en run expande panel con resultados individuales, metricas, regression report
  - Grafico de trend (sparkline o small line chart) arriba de la timeline
- **Test Case Editor** (panel lateral tipo drawer):
  - Seccion "Input Data": editor JSON con syntax highlighting y validacion
  - Seccion "Assertions": form builder visual — para cada assertion: target (dropdown con nodos del grafo + output paths), operator (dropdown), expected value (input). Boton "+ Add Assertion"
  - Seccion "Mocks" (colapsable, avanzado): editor JSON para mock_config o selector de snapshot existente
  - Seccion "Config": max_duration_ms, tags
  - Boton "Save" + "Save & Run"
- **Test Result Viewer** (panel inline al expandir un run):
  - Lista de assertions con iconos check/cross, con expected vs actual
  - Metricas de trajectory: steps, tokens, cost en cards
  - Link "View Session Trace" que navega al detalle de la session de test

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-213 | El tab Tests solo aparece cuando el agente tiene un grafo asociado (graph_id no null). Sin grafo no hay nada que testear | Conditional render en AgentDetail |
| REGLA-214 | Al ejecutar un test desde la UI, el resultado se actualiza en real-time via WebSocket (canal `tests:{agent_id}`) | Evento test.run.completed emitido al terminar |
| REGLA-215 | El editor de assertions muestra autocompletado de targets basado en los nodos del grafo actual del agente | Lectura del graph_def para extraer node_ids y tool_types |

**Archivos a crear/modificar**:
- CREAR `app/web/src/components/tests/TestCaseEditor.tsx` — formulario completo de test case
- CREAR `app/web/src/components/tests/TestSuiteEditor.tsx` — formulario de suite
- CREAR `app/web/src/components/tests/TestRunViewer.tsx` — visualizacion de resultados
- CREAR `app/web/src/components/tests/TestMetricsCards.tsx` — cards de metricas
- CREAR `app/web/src/components/tests/TestTrendChart.tsx` — grafico de tendencia
- CREAR `app/web/src/components/tests/AssertionBuilder.tsx` — form builder de assertions
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab Tests
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de tests (si no estan ya de 12.1)

---

## Dependencias entre features

```
12.1 (Test Runner) ← independiente, se implementa primero
12.2 (Metrics) ← depende de 12.1 para TestRun/TestResult
12.3 (Mocks) ← depende de 12.1 para TestCase + MockInjector integration
12.4 (UI) ← depende de 12.1 + 12.2 + 12.3 para mostrar todo
```

Orden de implementacion: 12.1 → 12.3 → 12.2 → 12.4

---

## Entidades nuevas (resumen consolidado)

### test_case
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| input_data | TEXT | JSON, NOT NULL DEFAULT '{}' |
| mock_config | TEXT | JSON, DEFAULT '{}' |
| assertions | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| max_duration_ms | INTEGER | NULL |
| tags | TEXT | JSON array, DEFAULT '[]' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### test_suite
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| test_case_ids | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| run_mode | TEXT | NOT NULL DEFAULT 'sequential' |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### test_run
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| suite_id | TEXT | FK test_suite(id) ON DELETE SET NULL, NULL |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| status | TEXT | NOT NULL DEFAULT 'running' |
| total_cases | INTEGER | NOT NULL DEFAULT 0 |
| passed_cases | INTEGER | NOT NULL DEFAULT 0 |
| failed_cases | INTEGER | NOT NULL DEFAULT 0 |
| error_cases | INTEGER | NOT NULL DEFAULT 0 |
| metrics | TEXT | JSON, DEFAULT '{}' |
| baseline_run_id | TEXT | FK test_run(id), NULL |
| started_at | TEXT | NOT NULL |
| finished_at | TEXT | NULL |
| duration_ms | INTEGER | NULL |

### test_result
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| test_run_id | TEXT | FK test_run(id) ON DELETE CASCADE, NOT NULL |
| test_case_id | TEXT | FK test_case(id) ON DELETE SET NULL, NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| status | TEXT | NOT NULL DEFAULT 'running' |
| assertion_results | TEXT | JSON array, DEFAULT '[]' |
| trajectory_metrics | TEXT | JSON, DEFAULT '{}' |
| outcome_metrics | TEXT | JSON, DEFAULT '{}' |
| duration_ms | INTEGER | NULL |
| error_message | TEXT | NULL |
| created_at | TEXT | NOT NULL |

### test_snapshot
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| session_id | TEXT | FK sessions(id) ON DELETE SET NULL, NULL |
| name | TEXT | NOT NULL |
| description | TEXT | DEFAULT '' |
| captures | TEXT | JSON, NOT NULL |
| created_at | TEXT | NOT NULL |

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-200 | Test executions crean sessions reales con tag `source: test` | TestRunnerService |
| REGLA-201 | Test sessions no modifican memorias del agente (test_mode: true) | Flag en session |
| REGLA-202 | Timeout excedido → status error con mensaje descriptivo | asyncio.wait_for |
| REGLA-203 | Assertions evaluan SharedState final con dot notation | Assertion evaluator |
| REGLA-204 | TestRun passed solo si TODOS los results son passed | Agregacion en runner |
| REGLA-205 | TrajectoryMetrics se calcula de execution_spans existentes | MetricsCalculator |
| REGLA-206 | Quality score via LLM judge es opt-in | Flag en mock_config |
| REGLA-207 | Regression threshold configurable (default 10%) | Comparacion relativa |
| REGLA-208 | Metricas immutables post-calculo | Persistencia JSON |
| REGLA-209 | Mocks solo en test_mode. Ejecuciones normales nunca mockean | Check de flag |
| REGLA-210 | Nodo con mock salta ejecucion real completamente | MockInjector intercepta |
| REGLA-211 | Snapshots son inmutables post-captura | Sin endpoint de update |
| REGLA-212 | Snapshot → test case hereda todos los outputs como mocks | Conversion automatica |
| REGLA-213 | Tab Tests solo visible si agente tiene grafo | Conditional render |
| REGLA-214 | Resultados de test se actualizan en real-time via WS | Evento test.run.completed |
| REGLA-215 | Autocompletado de targets basado en nodos del grafo | Lectura de graph_def |

---

## Notas de implementacion

- **El TestRunnerService reutiliza el pipeline de ejecucion existente**. Crea una session real y ejecuta via GraphRunner. La diferencia es que inyecta MockInjector y evalua assertions al final. No se reimplementa ejecucion.
- **MockInjector es un hook en el runner, no un fork del codigo**. Se agrega un punto de intercepcion en GraphRunner.execute_node() que consulta al MockInjector antes de ejecutar el bloque. Si hay mock, retorna el mock. Si no, ejecuta normal.
- **Snapshots se generan desde execution_trace**. La tabla execution_trace (ya existente) tiene inputs_snapshot y output_snapshot por nodo. SnapshotCapture simplemente lee esos datos y los empaqueta como MockSnapshot.
- **LLM Judge para quality score es costoso**. Se usa solo cuando el usuario lo configura explicitamente en el test case. Default: sin LLM judge. Cuando se activa, se usa el adapter del provider default para evaluar el output contra criterios textuales.
- **WebSocket channel para tests**: `tests:{agent_id}` con eventos `test.run.started`, `test.run.completed`, `test.result.completed`. La UI usa useReactive para actualizaciones en real-time.
- **Backward compatibility**: No se modifican tablas existentes (sessions, execution_trace, execution_spans). Solo se agregan tablas nuevas y se lee datos de las existentes.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones, WebSocket, testing
- `docs/prd/draft/FEAT-002.md` — Multi-LLM (LLM judge usa adapters de aca)
