# PRD: Crates.io Quality Refactor

| Campo | Valor |
|-------|-------|
| **ID** | FEAT-033 |
| **Fecha** | 2026-05-23 |
| **Estado** | draft |
| **Fecha cierre** | — |

---

## Problema

**Tipo**: mejora (refactor arquitectonico)

**Resumen**: `datamirai-engine` (crate Rust) tiene defectos de API que bloquean su publicacion en crates.io como libreria de calidad produccion: fat traits sin defaults, tipos duplicados, strings magicos donde deberian haber enums, allocaciones innecesarias en hot paths, y ~70% de API publica sin documentacion.

**Actores afectados**:
- Desarrolladores que consumen el crate (futuros usuarios de crates.io)
- Datamirai Local (consumidor principal actual)
- Contributors del engine

**Flujos afectados**: Session (ejecucion), Agent Lifecycle (validacion), Graph Version (builder)

**Contexto**: El crate tiene 542 tests pasando y es funcional. Pero la API publica viola multiples guidelines oficiales de Rust (API Guidelines checklist): C-GOOD-ERR (errores pierden source), C-EXAMPLE (~70% sin docs), C-CRATE-DOC inexistente, traits sin defaults. 5 problemas criticos, 8 importantes, 8 mejoras.

**Objetivo**: Al terminar, el crate pasa: (1) cargo semver-checks sin violaciones, (2) cargo doc con deny(missing_docs), (3) cargo clippy limpio, (4) 542 tests verdes, (5) API sigue Rust API Guidelines, (6) Datamirai Local compila.

## Actores y Permisos

| Actor | Capability | Accion | Visibilidad |
|-------|-----------|--------|-------------|
| Desarrollador consumidor | Implementar traits | HookHandler, ToolExecutor, ExecutionContext | API publica, docs.rs |
| Datamirai Local | Consumir crate | GraphRunner, tools, ejecucion | API publica + features |
| Contributor | Modificar internals | Runner, context, tools | pub(crate) |

Sin capabilities nuevas — refactor interno.

## Entidades Afectadas

No hay tablas SQL. Tipos Rust publicos afectados:

### Tipos modificados

| Tipo | Archivo | Cambio |
|------|---------|--------|
| HookHandler (trait) | core/runner.rs:73 | Agregar default impls a 7 metodos |
| TokenUsage (duplicado) | core/context.rs:53 + llm/adapter.rs:16 | Consolidar en core/context.rs |
| ToolField::field_type | tools/base.rs:13 | String → FieldType enum |
| TraceEntry::status | core/runner.rs:246 | String → TraceStatus enum |
| GraphRunner (struct) | core/runner.rs:294 | Box<dyn> → Arc<dyn> para Clone |
| SharedState::get() | core/state.rs:63 | Agregar get_ref() sin clone |
| ToolFactory::create() | tools/registry.rs:38 | Box<dyn Tool> → Arc<dyn Tool> |
| Tool::execute() | tools/registry.rs:22 | config owned → &HashMap |
| AgentRetryConfig | core/agent_spec.rs:96 | String → BackoffStrategy/FailureMode |
| RunnerError::ExecutionFailed | core/runner.rs:30 | Agregar #[source] |
| ExecutionContext (trait) | core/context.rs:163 | Evaluar Extensions/TypeMap |
| AgentSpec::validate() | core/agent_spec.rs:299 | Delegar a GraphDef::validate() |

### Tipos nuevos

| Tipo | Ubicacion | Proposito |
|------|-----------|-----------|
| FieldType enum | tools/base.rs | Reemplaza field_type: String |
| TraceStatus enum | core/runner.rs | Reemplaza status: String |

### Funciones modificadas

| Funcion | Archivo | Cambio |
|---------|---------|--------|
| resolve_expression() | core/runner.rs:1061 | Regex::new() → static LazyLock |
| node_index() | core/runner.rs:1273 | Linear O(n) → HashMap O(1) |
| default_version() | graph.rs + agent_spec.rs | Dedup — consolidar en uno |

## Ciclos de Vida

N/A — no se modifican maquinas de estado. Solo se mejora la representacion interna (String → enum).

## Pantallas

N/A — refactor de library crate sin UI.

## Contratos de API

No hay endpoints REST. Cambios en API publica Rust:

### Breaking changes

| Tipo | Antes | Despues | Mitigacion |
|------|-------|---------|-----------|
| ToolField::field_type | String | FieldType | Serde compatible + From<String> |
| TraceEntry::status | String | TraceStatus | Serde rename_all compatible |
| Tool::execute(config) | HashMap owned | &HashMap | Callers pasan ref |
| ToolFactory::create() | Box<dyn Tool> | Arc<dyn Tool> | Mas flexible |
| AgentRetryConfig::backoff | String | BackoffStrategy | Serde compatible |
| AgentRetryConfig::on_failure | String | FailureMode | Serde compatible |

### Non-breaking

HookHandler defaults, SharedState::get_ref(), GraphRunner Arc internals, resolve_expression LazyLock, node_index HashMap.

## Reglas de Negocio

### regla-trait-defaults-obligatorios
**Invariante**: todo metodo de trait con no-op razonable tiene default impl
**Enforcement**: code review + deny(missing_docs)
**Violacion**: PR rechazado

### regla-no-strings-magicos
**Invariante**: campos con valores finitos usan enum, nunca String
**Enforcement**: clippy + code review
**Violacion**: compile-time

### regla-error-chains-preservados
**Invariante**: errores que wrappean otros usan #[source], nunca String
**Enforcement**: thiserror
**Violacion**: PR rechazado

### regla-zero-alloc-hot-path
**Invariante**: funciones per-nodo no alocan innecesariamente
**Enforcement**: benchmarks
**Violacion**: regression

### regla-tests-verdes-cada-cambio
**Invariante**: 542 tests pasan despues de cada commit
**Enforcement**: cargo test
**Violacion**: revert

### regla-semver-compatible
**Invariante**: breaking changes coordinados con Datamirai Local
**Enforcement**: cargo semver-checks
**Violacion**: bump de version + migracion

## Escenarios GWT

| test_id | Scenario | Given | When | Then |
|---------|----------|-------|------|------|
| TEST-136 | HookHandler defaults — solo on_error | struct con solo on_error | compila como HookHandler | ok, 6 defaults retornan Continue |
| TEST-137 | HookHandler override selectivo | struct overridea 2 metodos | ejecuta grafo | custom invocados, 5 defaults usados |
| TEST-138 | FieldType enum serde | FieldType::String | serializa JSON | "string", deserializa OK |
| TEST-139 | FieldType rechaza typo | JSON "stirng" | deserializa | error |
| TEST-140 | TraceStatus enum serde | TraceStatus::Ok | serializa | "ok", backward-compatible |
| TEST-141 | Regex estatico | grafo 100 nodos con data_map | ejecuta | Regex compila 1 vez |
| TEST-142 | SharedState get_ref | output grande 1MB | get_ref() | referencia sin clone |
| TEST-143 | node_index O(1) | grafo 50 nodos | run() | HashMap pre-computado |
| TEST-144 | RunnerError preserva source | tool falla | runner wrappea | source() retorna ToolError |
| TEST-145 | TokenUsage unificado | import TokenUsage | uso en ambos modulos | mismo tipo |
| TEST-146 | AgentSpec detecta self-loops | edge source==target | validate() | error |
| TEST-147 | AgentSpec detecta grafo vacio | 0 nodos | validate() | error |
| TEST-148 | GraphRunner Clone | runner con executor | clone() | ambos comparten Arc |
| TEST-149 | AgentRetryConfig enums | YAML backoff: exponential | deserializa | BackoffStrategy::Exponential |
| TEST-150 | AgentRetryConfig rechaza invalido | YAML backoff: invalid | deserializa | error |
| TEST-151 | Tool config por referencia | nodo con config | execute() | config como &HashMap |
| TEST-152 | deny(missing_docs) compila | lib.rs con deny | cargo doc | zero warnings |

## Fuera de Alcance

- Reescribir ExecutionContext como TypeMap (evaluacion solo, no implementacion completa)
- Refactorizar modulos server/vault/render (mejora M5 — separar a fase posterior)
- LLMResource vs LLMAdapter unificacion (mejora M2 — evaluar en fase posterior)
- Builder alternativo &mut self (mejora M7 — baja prioridad)

## Dependencias

- Stack skill: `blueprint/stacks/rust-crate-design.md` (creado 2026-05-23)
- Los 542 tests existentes como red de seguridad
- Datamirai Local compilando como baseline

## Tickets en WORKBOARD

> Generados automaticamente — ver workboard.db
