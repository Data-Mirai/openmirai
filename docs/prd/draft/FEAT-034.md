# API Simplification & DX Overhaul

| Campo | Valor |
|-------|-------|
| **ID** | FEAT-034 |
| **Fecha** | 2026-05-25 |
| **Estado** | draft |
| **Branch** | — |

---

## Diagrama General

```
                        Authoring (cómo diseñas)
                ┌───────────────────────────────────────┐
                │                                       │
  ┌─────────────┤  Python SDK    Visual Editor    YAML  │
  │             │  (AgentSpec)   (n8n-like)       (CLI) │
  │             └───────────────────────────────────────┘
  │                              │
  │                    ┌─────────▼─────────┐
  │                    │  agent.json       │  ← artefacto canónico
  │                    │  (JSON Schema     │    portable entre lenguajes
  │                    │   validated)      │
  │                    └─────────┬─────────┘
  │                              │
  │             ┌────────────────▼────────────────┐
  │             │  Motor (Rust binary)            │
  │             │  FFI / WASM / CLI               │
  │             └────────────────┬────────────────┘
  │                              │
  │             ┌────────────────▼────────────────┐
  │             │  Host (Python / Swift / Go)     │
  │             │  Backend que abraza motor+agent │
  │             └─────────────────────────────────┘
```

---

## Problema

- **Tipo**: mejora
- **Resumen**: La API pública del Engine es 2-3x más verbosa que LangGraph para casos equivalentes. El rediseño aplica a ambos runtimes (Python SDK + Rust core) porque el motor es el binario Rust y Python/Swift/Go/WASM son consumers.
- **Actores**: Developer (SDK user), EDITOR
- **Flujos tocados**: Diseñar grafo → Desplegar como agente (existente), Authoring programático (nuevo), Distribución 3 capas (clarificado)
- **Qué cambia**: Hoy el SDK requiere 5-6 líneas de setup ceremonial, tools de 15+ líneas, strings sueltos como configuración, edge IDs manuales, data_map obligatorio en cadenas lineales, JSON Schema manual para structured output → después: 3 líneas para caso simple, enums tipados, auto-gen de IDs, default passthrough, Pydantic support, convenience factories

## Actores y Permisos

| Actor | Capacidad | Acción | Visibilidad |
|-------|-----------|--------|-------------|
| Developer (SDK user) | `define_agent` | Crear configuración de agente (JSON/YAML) via SDK de cualquier lenguaje | JSON/YAML completo, enums tipados, schema validable |
| Developer (SDK user) | `run_agent` | Ejecutar agente contra el motor (FFI/WASM/CLI) | ExecutionResult con status, state, trace, transcript |
| Developer (SDK user) | `define_tool` | Crear herramientas custom (@tool decorator o BaseTool) | ToolSpec, inputs, outputs, config |
| Developer (SDK user) | `configure_context` | Inyectar recursos (LLM, DB, vector, storage) | Recursos configurados en el contexto |
| EDITOR | `design_graph` | Diseñar grafos en editor visual (existente) | Editor visual |
| EDITOR | `export_agent` | Serializar grafo a JSON/YAML (nuevo) | Archivo descargable |
| EDITOR | `import_agent` | Cargar agente desde JSON/YAML (nuevo) | Grafo reconstruido en editor |

**Nota**: Developer SDK no es un rol del runtime (no vive en DOMINIO.md). Es el consumidor del paquete. Sus "permisos" son las funciones que la API pública expone.

**Arquitectura de distribución (3 capas)**:

```
┌─────────────────────────────────┐
│  Agente = configuración (JSON)  │  ← portable, lenguaje-agnóstico
├─────────────────────────────────┤
│  Motor = binario Rust compilado │  ← FFI, WASM, o CLI
├─────────────────────────────────┤
│  Host = app que los abraza      │  ← Python backend, Swift app, Go service
└─────────────────────────────────┘
```

**3 momentos de validación del agente**:
1. **Authoring time** — IDE + tipos del lenguaje (si usa SDK)
2. **Build time** — CLI `mirai validate` o `agent.validate()`
3. **Runtime** — motor Rust valida JSON antes de ejecutar

## Entidades

No se crean entidades nuevas. Se refactorizan las existentes:

### AgentSpec (modificada)

| Campo | Tipo lógico | Cambio |
|-------|-------------|--------|
| format | enum [json, yaml] | Formato canónico JSON, YAML como convenience |
| triggers[].type | enum [webhook, schedule, event, manual, agent_call] | Era string libre → ahora TriggerType enum |
| config.retry.backoff | enum [none, linear, exponential] | Era string → ahora Backoff enum |
| config.retry.on_failure | enum [stop, skip, route_to_error] | Era string → ahora OnFailure enum |

### GraphDef (modificada)

| Campo | Tipo lógico | Cambio |
|-------|-------------|--------|
| edges[].id | texto (auto-gen) | Opcional, auto-generado como `{source}__{target}` si no se provee |
| edges[].data_map | json (opcional) | Default passthrough cuando edge es único saliente sin condición |

### EdgeDef (modificada)

| Campo | Tipo lógico | Cambio |
|-------|-------------|--------|
| id | texto (auto-gen) | Opcional |
| condition.op | enum [eq, neq, gt, lt, gte, lte, in, contains] | Era string → ahora Op enum |
| data_map | json (opcional) | Default passthrough para cadenas lineales |

### NodeDef (modificada)

| Campo | Tipo lógico | Cambio |
|-------|-------------|--------|
| tool_type | texto (validado por registry) | Se mantiene string pero validado contra registry al construir |

### ToolSpec (modificada)

| Campo | Tipo lógico | Cambio |
|-------|-------------|--------|
| inputs[].type | enum [string, number, boolean, object, array, any] | Era string → ahora DataType enum |
| outputs[].type | enum [string, number, boolean, object, array, any] | Era string → ahora DataType enum |
| config[].type | enum [string, number, boolean, select, slider, object] | Era string → ahora ConfigFieldType enum |

### Enums nuevos (value objects)

| Enum | Valores |
|------|---------|
| Op | EQ, NEQ, GT, LT, GTE, LTE, IN, CONTAINS |
| Backoff | NONE, LINEAR, EXPONENTIAL |
| OnFailure | STOP, SKIP, ROUTE_TO_ERROR |
| DataType | STRING, NUMBER, BOOLEAN, OBJECT, ARRAY, ANY |
| TriggerType | WEBHOOK, SCHEDULE, EVENT, MANUAL, AGENT_CALL |
| SessionStatus | PENDING, RUNNING, COMPLETED, FAILED, TIMEOUT, INTERRUPTED |
| ConfigFieldType | STRING, NUMBER, BOOLEAN, SELECT, SLIDER, OBJECT |

**Relaciones**: No cambian. La estructura grafo (nodes→edges→graph→agent) se mantiene.

## Ciclos de Vida

No se agregan máquinas de estado nuevas. Las existentes (Agent Lifecycle, Session, Graph Version) no cambian.

Ciclo del agente como artefacto (no es máquina de estado del runtime):

```
authored → validated → loaded → executed
```

## Reglas de Negocio

### API-01: enums-obligatorios
- **Invariante**: Todo parámetro de configuración con conjunto finito de valores DEBE ser enum tipado, nunca string libre
- **Cuándo se verifica**: En compilación (Rust) / en definición (Python)
- **Si se viola**: `TypeError` si se pasa string donde se espera enum

### API-02: edge-id-auto
- **Invariante**: EdgeDef.id es auto-generado si no se provee. Formato: `{source}__{target}` (o `{source}__{target}__{n}` si hay múltiples)
- **Cuándo se verifica**: En construcción del GraphDef
- **Si se viola**: N/A (transparente)

### API-03: data-map-passthrough
- **Invariante**: EdgeDef.data_map default = passthrough completo del output del nodo source cuando no se especifica y el edge es el único saliente sin condición
- **Cuándo se verifica**: En resolución de inputs (runner)
- **Si se viola**: N/A (transparente)

### API-04: json-canonico
- **Invariante**: JSON es el formato canónico de AgentSpec. YAML es convenience layer con round-trip lossless
- **Cuándo se verifica**: En serialización
- **Si se viola**: `ValidationError` si el YAML produce JSON inválido

### API-05: tool-decorator-sugar
- **Invariante**: El decorator `@tool` produce internamente un BaseTool+ToolSpec. Es sugar, no un camino alterno
- **Cuándo se verifica**: En registro
- **Si se viola**: Mismas validaciones que BaseTool

### API-06: factory-equivalence
- **Invariante**: Convenience factories (`GraphRunner.default()`, `ToolRegistry.default()`) son equivalentes a la configuración manual completa
- **Cuándo se verifica**: Por diseño
- **Si se viola**: N/A

### API-07: json-portabilidad
- **Invariante**: El JSON de un agente producido por cualquier lenguaje DEBE ser ejecutable por el motor Rust sin transformación
- **Cuándo se verifica**: En validación de JSON Schema
- **Si se viola**: `SchemaValidationError`

## Patrones de Diseño

### Builder → GraphDef, AgentSpec
- **Aplica a**: Construcción de grafos y specs de agente
- **Por qué**: Construcción fluida paso a paso como alternativa al constructor directo
- **Participantes**: GraphDef.build() → NodeSpec → EdgeSpec → GraphDef validado

```
┌─ Builder ──────────────────────────────────┐
│  Graph.build()                             │
│    .node("t", ToolType.WEBHOOK)            │
│    .node("p", ToolType.LLM_CALL, config)   │
│    .edge("t", "p")                         │
│    .done()  ← valida y retorna GraphDef    │
└────────────────────────────────────────────┘
```

### Factory Method → GraphRunner.default(), ToolRegistry.default()
- **Aplica a**: Setup del runtime
- **Por qué**: Encapsular el setup ceremonial (registry+executor+runner) en un solo call
- **Participantes**: Factory crea registry con builtins → executor → runner pre-wired

### Decorator (structural) → @tool
- **Aplica a**: Definición de tools simples
- **Por qué**: Wrap función Python en BaseTool+ToolSpec sin escribir la clase completa
- **Participantes**: Función decorada → ToolSpec inferido de type hints → BaseTool generado

### Strategy → Backoff, OnFailure
- **Aplica a**: Retry policy y manejo de errores
- **Por qué**: Enums tipados reemplazan strings como selectores de estrategia
- **Participantes**: Enum selecciona implementación concreta en runtime

### Adapter → YAML↔JSON
- **Aplica a**: Serialización de AgentSpec
- **Por qué**: YAML como adapter sobre formato canónico JSON. Round-trip sin pérdida
- **Participantes**: YAML parser → JSON canonical → YAML serializer

## Operaciones

### Agent.from_yaml
- **Actor**: Developer
- **Capacidad requerida**: `define_agent`
- **Input**: yaml_source (texto, requerido) — string YAML o path a archivo
- **Output exitoso**: AgentSpec validado y listo para ejecutar
- **Errores**: ValidationError (schema inválido), FileNotFoundError (path no existe)

### Agent.from_json
- **Actor**: Developer
- **Capacidad requerida**: `define_agent`
- **Input**: json_source (texto, requerido) — string JSON o path a archivo
- **Output exitoso**: AgentSpec validado
- **Errores**: ValidationError, FileNotFoundError

### agent.run
- **Actor**: Developer
- **Capacidad requerida**: `run_agent`
- **Input**: inputs (json, requerido), context (ExecutionContext, opcional — usa default si no se provee)
- **Output exitoso**: ExecutionResult con status, state, trace, transcript
- **Errores**: ExecutionError, TimeoutError

### agent.to_json / agent.to_yaml
- **Actor**: Developer
- **Capacidad requerida**: `define_agent`
- **Input**: path (texto, opcional — si no se provee retorna string)
- **Output exitoso**: JSON/YAML string o archivo escrito
- **Errores**: N/A

### @tool decorator
- **Actor**: Developer
- **Capacidad requerida**: `define_tool`
- **Input**: función Python con type hints
- **Output exitoso**: Clase registrable en ToolRegistry con ToolSpec inferido
- **Errores**: TypeError (hints incompletos o tipos no soportados)

### GraphRunner.default()
- **Actor**: Developer
- **Capacidad requerida**: `configure_context`
- **Input**: ninguno
- **Output exitoso**: Runner con registry (builtins pre-registrados) + executor pre-wired
- **Errores**: N/A

### Graph.build()
- **Actor**: Developer
- **Capacidad requerida**: `define_agent`
- **Input**: fluent API calls (.node, .edge, .done)
- **Output exitoso**: GraphDef validado
- **Errores**: ValidationError al .done() si grafo inválido

## Interfaces

No hay interfaces de usuario nuevas. El editor visual es indirectamente afectado: cuando el formato JSON cambie (enums en lugar de strings), el editor debe emitir el nuevo formato. Cambio de serialización, no de UI.

## Matriz de Permutaciones

| Flujo | Permutación | Actor | Resultado esperado |
|---|---|---|---|
| from_yaml | Happy path: YAML válido | Developer | AgentSpec cargado |
| from_yaml | Input inválido: YAML malformado | Developer | ValidationError |
| from_yaml | Input inválido: schema incorrecto | Developer | ValidationError |
| from_yaml | No encontrado: path inexistente | Developer | FileNotFoundError |
| agent.run | Happy path: agente simple | Developer | ExecutionResult completed |
| agent.run | Happy path: con context custom | Developer | ExecutionResult completed |
| agent.run | Timeout: agente excede tiempo | Developer | TimeoutError |
| @tool | Happy path: función con hints completos | Developer | Tool registrable |
| @tool | Input inválido: hints incompletos | Developer | TypeError |
| Graph.build | Happy path: grafo válido | Developer | GraphDef |
| Graph.build | Input inválido: edge a nodo inexistente | Developer | ValidationError |
| Graph.build | Edge case: múltiples edges mismo source→target | Developer | IDs auto-gen con sufijo |
| enum validation | Input inválido: valor fuera de enum | Developer | TypeError |
| passthrough | Happy path: edge lineal sin data_map | Developer | Inputs resueltos automáticamente |
| portabilidad | Happy path: JSON de Python ejecutado en Rust | Developer | Ejecución idéntica |
| round-trip | Happy path: YAML→JSON→YAML | Developer | Output == input |
| backward compat | Edge case: strings legacy en lugar de enums | Developer | Funciona con deprecation warning |

## Escenarios GWT

| test_id | Escenario | Given | When | Then |
|---------|-----------|-------|------|------|
| TEST-153 | Quick path: agente desde YAML en 3 líneas | Un archivo agent.yaml válido con grafo de 2 nodos | `agent = Agent.from_yaml("agent.yaml")` y `result = await agent.run({"query": "hola"})` | result.status == completed y output accesible |
| TEST-154 | Full control: setup manual equivalente | Registry custom + executor + runner + context explícito | Ejecutar mismo grafo que TEST-153 con runner.run() | Mismo resultado que TEST-153 |
| TEST-155 | Enum validation: operator inválido | EdgeDef con condition op="invalid" | Construir GraphDef | TypeError o ValueError en construcción, no en runtime |
| TEST-156 | Auto-gen edge ID | EdgeDef sin id, source="a", target="b" | Construir GraphDef | edge.id == "a__b" |
| TEST-157 | Auto-gen edge ID con colisión | 2 edges classify→handle con condiciones distintas, sin ID | Construir GraphDef | IDs: classify__handle__1, classify__handle__2 |
| TEST-158 | Default passthrough data_map | Edge único sin condición y sin data_map | Runner resuelve inputs del target | Target recibe todo el output del source |
| TEST-159 | @tool decorator: tool simple | Función def weather(city: str) -> str decorada con @tool | registry.register(weather) | Tool registrado con ToolSpec inferido |
| TEST-160 | Portabilidad JSON: Python → Rust | AgentSpec creado en Python, serializado a JSON | Motor Rust parsea y ejecuta el mismo JSON | Ejecución exitosa, zero-transform |
| TEST-161 | YAML↔JSON round-trip lossless | AgentSpec cargado desde YAML | .to_json() → from_json() → .to_yaml() | YAML output == YAML input |
| TEST-162 | Structured output con Pydantic | NodeDef ai/llm_call con output_schema=PydanticModel | Ejecutar LLM call | Output validado contra el model, retry si falla |
| TEST-163 | Convenience factory equivalence | GraphRunner.default() | Ejecutar mismo grafo que con setup manual | Resultado idéntico |
| TEST-164 | Backward compat: strings legacy aceptados | EdgeDef con condition {"op": "eq"} string | Construir GraphDef | Funciona con deprecation warning, internamente Op.EQ |

## Fuera de Alcance

- Cambios en la lógica interna del GraphRunner (REGLA-01 a REGLA-38 no cambian)
- Nuevos tipos de bloques/tools builtin
- Cambios en el editor visual (solo serialización)
- CLI `mirai validate` (futuro, no en este PRD)
- FFI/WASM bindings del motor Rust (FEAT-032 cubre eso)

## Dependencias

- FEAT-032 (Engine Rust) debe existir para TEST-160 (portabilidad JSON Python→Rust)
- FEAT-033 (Crates.io Quality) idealmente completo antes — ya está done
