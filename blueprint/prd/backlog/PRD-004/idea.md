# Agent Contract — Input/Output Schema + Runner Estricto

| Campo | Valor |
|-------|-------|
| **ID** | PRD-004 |
| **Fecha** | 2026-05-27 |
| **Estado** | in_progress |
| **Branch** | prd/PRD-004 |
| **Target** | v0.3.0 |

---

## Diagrama General

```
┌─────────────────────────────────────────────────────────────────────┐
│                    PRD-004 — DOS CAPAS DE VALIDACION                │
│                                                                     │
│  CAPA 1: AgentSpec boundary (client → engine)                      │
│  ─────────────────────────────────────────────                     │
│                                                                     │
│  Client payload                                                     │
│       │                                                             │
│       ▼                                                             │
│  ┌──────────────────────────────────────────┐                      │
│  │ validate_agent_inputs(payload, spec)     │                      │
│  │                                          │                      │
│  │  spec.inputs:                            │                      │
│  │    question: { type: text, required }    │                      │
│  │    context:  { type: text, optional }    │                      │
│  │                                          │                      │
│  │  ✓ required present?                     │                      │
│  │  ✓ types match?                          │                      │
│  │  ✓ defaults applied?                     │                      │
│  │                                          │                      │
│  │  FAIL → "missing required input: X"      │                      │
│  └──────────────┬───────────────────────────┘                      │
│                 │ OK                                                │
│                 ▼                                                   │
│  inject → trigger.config["payload"]                                │
│                 │                                                   │
│                 ▼                                                   │
│  trigger.execute() → output:                                       │
│    { payload: {question, context}, triggered_by, timestamp }       │
│                                                                     │
│  CAPA 2: Node boundary (nodo → nodo, dentro del runner)            │
│  ─────────────────────────────────────────────────────             │
│                                                                     │
│  ┌──────────────────────────────────────────┐                      │
│  │ POR CADA NODO:                           │                      │
│  │                                          │                      │
│  │ 1. resolve_inputs(data_map, state)       │                      │
│  │    → nested traversal: trigger.payload.q │                      │
│  │                                          │                      │
│  │ 2. merge node.config defaults            │                      │
│  │                                          │                      │
│  │ 3. validate_node_inputs(inputs, ToolSpec)│                      │
│  │    ✓ required ToolFields present?        │                      │
│  │    ✓ FieldTypes match?                   │                      │
│  │    ✓ defaults from ToolField.default?    │                      │
│  │    FAIL → "Node 'llm': missing 'prompt'" │                      │
│  │                                          │                      │
│  │ 4. catch_unwind { tool.execute() }       │                      │
│  │    PANIC → ToolError (no process crash)  │                      │
│  │                                          │                      │
│  │ 5. state.set(node_id, output)            │                      │
│  └──────────────────────────────────────────┘                      │
│                                                                     │
│  YAML-ONLY PARA AGENT SPECS                                        │
│  ───────────────────────────                                       │
│  YAML = unico formato para specs. JSON eliminado.                  │
│  from_file() → siempre serde_yaml (parsea YAML y JSON syntax).    │
│  Eliminar from_json(), to_json() publicos.                         │
│  HTTP API body → sigue JSON (es HTTP standard).                    │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: feature + hardening
- **Resumen**: El trigger acepta cualquier payload sin validacion. El runner no valida inputs de nodos contra ToolSpec. Expressions que no resuelven fallan silenciosamente (`None` → input omitido → tool recibe datos incompletos → panic o garbage). No existe contrato formal entre client y agente. YAML es soportado pero no canonico.
- **Actores**: Developer (diseña agent specs), Host App (envia datos al engine), Engine (valida y ejecuta)
- **Flujos tocados**: AgentSpec parsing, trigger injection, data_map resolution, runner execute loop, CLI --input, HTTP execute
- **Que cambia**:
  - HOY: payload entra sin validacion → trigger lo pasa crudo → data_map falla silenciosamente → tools crashean con unwrap
  - DESPUES: payload validado contra spec.inputs → trigger output estandarizado → data_map con nested traversal → node inputs validados contra ToolSpec → tool.execute() envuelto en catch_unwind → zero crashes

---

## Actores y Permisos

| Actor | Capacidad | Accion | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | disenar_spec | Define inputs/outputs en AgentSpec YAML | Contrato completo |
| Developer | inspeccionar_contrato | `mirai describe agent.yaml` muestra inputs/outputs | Schema del agente |
| Host App | enviar_datos | Envia payload que cumple con spec.inputs | Errores de validacion claros |
| Host App | leer_contrato | `GET /api/agents/{id}/schema` retorna inputs/outputs | Schema como JSON |
| Engine | validar_inputs | Valida payload contra spec.inputs antes de ejecutar | N/A (interno) |
| Engine | validar_nodos | Valida inputs resueltos contra ToolSpec antes de tool.execute() | N/A (interno) |

---

## Entidades

### InputFieldSpec (nueva — campo de spec.inputs)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| type | enum [text, number, boolean, json, file] | si | Tipo esperado del valor |
| required | booleano | no (default false) | Si el campo es obligatorio |
| description | texto | no | Descripcion para documentacion y SDKs |
| default | valor | no | Valor por defecto si no se provee |

**Restricciones**: `default` solo aplica si `required = false`. Si `required = true` y hay `default`, el default se ignora (el client DEBE proveerlo).

### OutputFieldSpec (nueva — campo de spec.outputs)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| type | enum [text, number, boolean, json, file] | si | Tipo esperado del valor |
| description | texto | no | Descripcion para documentacion y SDKs |

**Nota**: outputs son declarativos en v1. Describen que produce el agente para que clientes/SDKs sepan que esperar. No se validan en runtime (el engine no fuerza que el output node produzca exactamente estos campos).

### InputType (nuevo enum — tipos user-facing para YAML)

| Valor | Mapea a serde_json | Validacion |
|-------|-------------------|------------|
| text | Value::String | `value.is_string()` |
| number | Value::Number | `value.is_number()` |
| boolean | Value::Bool | `value.is_boolean()` |
| json | Value::Object o Value::Array | `value.is_object() \|\| value.is_array()` |
| file | Value::String | `value.is_string()` (path, URL, o base64) |

**Relacion con FieldType existente**: InputType es el enum user-facing (YAML). FieldType (tools/base.rs) es el enum interno. Mapping:

| InputType | FieldType |
|-----------|-----------|
| text | String |
| number | Number |
| boolean | Boolean |
| json | Object |
| file | String |

### ValidationError (nueva — runtime)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| field | texto | si | Nombre del campo que fallo |
| error_type | enum [missing_required, type_mismatch, unresolved_expression] | si | Tipo de error |
| message | texto | si | Mensaje legible para el developer |
| node_id | texto | no | Nodo donde ocurrio (solo Capa 2) |
| edge_info | texto | no | Edge que transporta el dato (solo Capa 2) |

### AgentSpec (modificada — agregar inputs/outputs)

Campos nuevos:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| inputs | mapa de InputFieldSpec | no | Schema de inputs del agente. Si ausente → sin validacion (backward compat) |
| outputs | mapa de OutputFieldSpec | no | Schema de outputs del agente. Declarativo — no validado en runtime v1 |

```
AgentSpec struct actualizado:

  name: String
  description: String
  version: String
  agent_type: AgentType
  system_prompt: Option<String>
  soul: Option<String>
  inputs: Option<Map<String, InputFieldSpec>>      ◀── NUEVO
  outputs: Option<Map<String, OutputFieldSpec>>     ◀── NUEVO
  graph: AgentGraphSpec
  triggers: Vec<AgentTriggerSpec>
  config: AgentConfig
  resources: Vec<AgentResourceRef>
  metadata: HashMap<String, Value>
```

**Backward compat**: `inputs` y `outputs` usan `Option` con `#[serde(default)]`. Specs sin estos campos → `None` → cero validacion, funciona identico a hoy.

### ManualTriggerTool output (modificada)

| Campo output | Antes | Despues |
|-------------|-------|---------|
| Datos del client | `user_input` | `payload` |
| Quien triggeo | `triggered_by` | `triggered_by` (sin cambio) |
| Timestamp | `timestamp` | `timestamp` (sin cambio) |

**Injection key**: `config["mock_payload"]` → `config["payload"]` (con fallback a `mock_payload` para backward compat).

### SharedState.get_field (modificada — nested traversal)

| Antes | Despues |
|-------|---------|
| `get_field("trigger", "user_input")` → busca key exacta | `get_field("trigger", "payload.question")` → traversa nested JSON |
| Solo 1 nivel de profundidad | N niveles: `a.b.c.d` |
| `trigger.user_input.question` → None (key "user_input.question" no existe) | `trigger.payload.question` → Some(value) |

```
Algoritmo:
  field_path = "payload.question"
  parts = split('.') → ["payload", "question"]
  current = node_output.get("payload") → Value::Object({question: "..."})
  current = current.get("question") → Value::String("...")
  return Some(current)
```

**Backward compat**: paths sin dots (ej: `trigger.triggered_by`) funcionan identico — un solo part, sin traversal.

---

## Ciclos de Vida

### Payload Validation Pipeline

```
                    ┌────────────┐
                    │  RECEIVED  │  payload llega (CLI --input o HTTP trigger_data)
                    └─────┬──────┘
                          │
                    spec.inputs?
                   ╱             ╲
                 Si               No (None)
                 │                 │
                 ▼                 ▼
          ┌────────────┐   ┌──────────────┐
          │ VALIDATING │   │ PASS-THROUGH │  sin validacion, backward compat
          └─────┬──────┘   └──────┬───────┘
                │                 │
          valid?                  │
         ╱      ╲                │
       Si        No              │
       │          │              │
       ▼          ▼              │
 ┌──────────┐ ┌──────────┐     │
 │ ACCEPTED │ │ REJECTED │     │
 └────┬─────┘ └──────────┘     │
      │       (error claro)     │
      │                         │
      └─────────┬───────────────┘
                │
                ▼
         ┌────────────┐
         │  INJECTED  │  payload en trigger.config["payload"]
         └────────────┘
```

### Node Execution Pipeline (con validacion)

```
  ┌─────────────────┐
  │ RESOLVE_INPUTS  │  data_map expressions → nested traversal
  └───────┬─────────┘
          │
          ▼
  ┌─────────────────┐
  │ MERGE_DEFAULTS  │  node.config como fallback
  └───────┬─────────┘
          │
          ▼
  ┌─────────────────┐
  │ VALIDATE_NODE   │  contra ToolSpec: required + types
  └───────┬─────────┘
          │
     valid?
    ╱      ╲
  Si        No
  │          │
  ▼          ▼
┌──────┐  ┌──────────────────┐
│EXEC  │  │ VALIDATION_ERROR │  → apply failure_mode (Stop/Skip/Route)
└──┬───┘  └──────────────────┘
   │
   │ catch_unwind
   │
  ok?
 ╱    ╲
Si     Panic
│       │
▼       ▼
STORE  TOOL_ERROR  → apply retry/failure_mode
```

---

## Reglas de Negocio

### R1: inputs opcionales, outputs declarativos

- **Invariante**: `spec.inputs = None` → cero validacion. `spec.inputs = Some(map)` → validacion estricta. `spec.outputs` → solo informativo, sin enforcement en runtime v1.
- **Cuando se verifica**: al recibir payload (CLI o HTTP)
- **Si se viola**: N/A — es por diseño

### R2: required sin default

- **Invariante**: si un input tiene `required: true`, el client DEBE proveerlo. Si tambien tiene `default`, el default se ignora — required manda.
- **Cuando se verifica**: validate_agent_inputs()
- **Si se viola**: error: "missing required input: {name}"

### R3: lenient con extras

- **Invariante**: campos en el payload que NO estan declarados en spec.inputs son permitidos y pasan sin validacion. Solo se validan los campos declarados.
- **Cuando se verifica**: validate_agent_inputs()
- **Si se viola**: N/A — es por diseño (forward compat, migracion gradual)

### R4: node validation obligatoria

- **Invariante**: ANTES de llamar tool.execute(), el runner DEBE validar inputs resueltos contra ToolSpec.inputs del tool. Required fields ausentes → error controlado, no panic.
- **Cuando se verifica**: runner main loop, pre-execute
- **Si se viola**: error: "Node '{id}': missing required input '{field}'"

### R5: catch_unwind en tool.execute()

- **Invariante**: toda llamada a tool.execute() esta envuelta en catch_unwind. Si el tool paniquea → se trata como ToolError, no como crash del proceso.
- **Cuando se verifica**: execute_with_retry
- **Si se viola**: N/A — catch_unwind previene crash por construccion

### R6: nested traversal seguro

- **Invariante**: get_field con path `a.b.c` traversa JSON nested. Si cualquier nivel no existe o no es object → retorna None, nunca panic.
- **Cuando se verifica**: resolve_expression
- **Si se viola**: N/A — cada `.get()` retorna Option

### R7: trigger output estandar

- **Invariante**: ManualTriggerTool y WebhookTriggerTool producen campo `payload` (no `user_input` ni `body`). El payload contiene exactamente los datos del client validados.
- **Cuando se verifica**: trigger.execute()
- **Si se viola**: data_map expressions con `trigger.payload.x` no resuelven

### R8: YAML-only para agent specs

- **Invariante**: YAML es el UNICO formato para AgentSpecs. `from_json()` y `to_json()` eliminados como metodos publicos. `from_file()` usa `serde_yaml` para todo (YAML es superset de JSON — si alguien pasa JSON, serde_yaml lo parsea igual). `to_file()` siempre produce YAML. HTTP API bodies siguen siendo JSON (estandar HTTP).
- **Cuando se verifica**: al parsear specs, al serializar
- **Si se viola**: error de compilacion (metodos eliminados)

---

## Patrones de Diseno

### Validator Gate → pre-execution validation

- **Aplica a**: Capa 1 (AgentSpec inputs) y Capa 2 (Node inputs)
- **Por que**: validar ANTES de ejecutar previene 90% de crashes runtime. El costo es O(n) checks pre-execute, el beneficio es zero panics.
- **Participantes**: InputValidator (Capa 1), NodeValidator (Capa 2), Runner (orquesta)

```
┌──────────────────────────────────────────────────┐
│  Validator Gate Pattern                          │
│                                                  │
│  data ──▶ validate(data, schema) ──▶ execute()   │
│                │                                 │
│                ▼                                 │
│           valid? ─No─▶ Error (controlado)        │
│                │                                 │
│               Si                                 │
│                │                                 │
│                ▼                                 │
│           execute() ──▶ result                   │
└──────────────────────────────────────────────────┘
```

### Defensive Execution → catch_unwind

- **Aplica a**: tool.execute() wrapper
- **Por que**: tools son codigo de terceros (o codigo propio con bugs). Un unwrap() en un tool no debe crashear todo el engine.
- **Participantes**: Runner (wrapper), Tool (ejecutado)

### Path Traversal → nested field access

- **Aplica a**: SharedState.get_field, resolve_expression
- **Por que**: data_map necesita acceder sub-campos de objetos JSON: `trigger.payload.question`. Sin traversal, solo se puede mapear el objeto entero.
- **Participantes**: SharedState (traversal), Runner (resolution)

---

## Operaciones

### O1: validate_agent_inputs (Capa 1)

- **Actor**: Engine (interno)
- **Input**:
  - payload (json) — datos del client
  - inputs_schema (mapa de InputFieldSpec) — de spec.inputs
- **Output exitoso**: payload validado (con defaults aplicados para opcionales)
- **Errores posibles**:
  - Campo required ausente → `"missing required input: {name} ({description})"`
  - Tipo incorrecto → `"input '{name}': expected {type}, got {actual_type}"`
  - Multiples errores → todos reportados en una lista (no fail-fast)
- **Logica**:
  1. Para cada field en inputs_schema:
     - Si required y ausente en payload → error
     - Si presente: validar tipo (InputType.matches(value))
     - Si no required y ausente y tiene default → insertar default en payload
  2. Campos extras en payload (no en schema) → permitidos, pasan sin tocar
  3. Retornar payload enriquecido con defaults

### O2: inject_validated_payload (Capa 1)

- **Actor**: Engine (interno)
- **Input**:
  - graph (mutable) — grafo del agente
  - validated_payload (json) — payload ya validado
- **Output exitoso**: trigger node con `config["payload"]` inyectado
- **Logica**:
  1. Encontrar primer nodo con `tool_type.starts_with("trigger/")`
  2. Insertar `config["payload"] = validated_payload`
  3. Si no hay trigger → error: "no trigger node found in graph"
- **Reemplaza**: la inyeccion actual via `config["mock_payload"]`

### O3: extract_agent_outputs (Capa 1)

- **Actor**: Engine (interno, post-execution)
- **Input**:
  - state (SharedState) — estado final del grafo
  - outputs_schema (mapa de OutputFieldSpec) — de spec.outputs
  - graph (GraphDef) — para encontrar nodo terminal
- **Output exitoso**: mapa de outputs extraidos
- **Logica**:
  1. Encontrar nodo terminal (sin edges salientes)
  2. Para cada campo en outputs_schema: buscar en output del nodo terminal
  3. Si campo no encontrado en terminal → buscar en todos los nodos (ultimo que lo tenga)
  4. Retornar mapa con campos encontrados (best-effort, sin error si falta)
- **Nota**: v1 es best-effort. Futuro: campo `source: "node.field"` para mapping explicito.

### O4: describe_agent (Capa 1)

- **Actor**: Developer / Host App
- **Input**: AgentSpec (cargado desde archivo o registrado)
- **Output exitoso**: contrato portable:
  ```yaml
  name: qa-assistant
  version: v1
  inputs:
    question: { type: text, required: true, description: "..." }
    context: { type: text, required: false, description: "..." }
  outputs:
    answer: { type: text, description: "..." }
  ```
- **CLI**: `mirai describe agent.yaml`
- **HTTP**: `GET /api/agents/{id}/schema`
- **Errores posibles**:
  - Archivo no encontrado → "file not found: {path}"
  - YAML invalido → "parse error: {details}"

### O5: validate_node_inputs (Capa 2)

- **Actor**: Engine (interno, per-node en runner loop)
- **Input**:
  - resolved_inputs (HashMap) — inputs resueltos por data_map + config defaults
  - tool_spec (ToolSpec) — spec del tool registrado
  - node_id (texto) — para mensajes de error
- **Output exitoso**: inputs validados (con defaults de ToolField aplicados)
- **Errores posibles**:
  - Required field ausente → `"Node '{id}': missing required input '{field}' (type: {tool_type})"`
  - Tipo incorrecto → `"Node '{id}': input '{field}' expected {type}, got {actual}"`
- **Logica**:
  1. Para cada ToolField en tool_spec.inputs donde required=true:
     - Si ausente en resolved_inputs → error
  2. Para cada ToolField presente en resolved_inputs:
     - Validar FieldType.matches(value)
  3. Para cada ToolField no required con default y ausente:
     - Insertar default
  4. Retornar inputs enriquecidos
- **Integracion con runner**: se llama entre merge(node.config) y tool.execute()

### O6: resolve_expression_nested (Capa 2 — enhancement)

- **Actor**: Engine (interno)
- **Input**: expression string (ej: `"trigger.payload.question"`), SharedState
- **Output exitoso**: Value resuelto
- **Cambio vs actual**:
  - ANTES: `split_once('.')` → `state.get_field(node, field)` → busca key exacta
  - DESPUES: `split_once('.')` → `state.get_field(node, field_path)` → traversa nested JSON con dots
- **Logica de get_field mejorada**:
  ```
  get_field("trigger", "payload.question"):
    parts = ["payload", "question"]
    current = node_output.get("payload") → Value::Object
    current = current.get("question") → Value::String
    return Some(current)
  ```
- **Edge cases**:
  - Path no existe → None (no panic)
  - Path intermedio no es object → None
  - Path vacio → None
  - Un solo nivel (sin dots) → identico a hoy

---

## Interfaces

### CLI: mirai run (modificada)

- **Cambio**: `--input` ahora valida contra spec.inputs antes de ejecutar
- **Antes**: `mirai run agent.yaml --input '{"question":"hola"}'` → inyecta sin validar
- **Despues**: valida → error claro si falla, ejecuta si pasa
- **Error ejemplo**:
  ```
  Error: input validation failed for agent 'qa-assistant':
    - missing required input: question (Pregunta del usuario)
  ```
- **Sin spec.inputs**: comportamiento identico a hoy (sin validacion)

### CLI: mirai describe (nueva)

- **Proposito**: mostrar contrato del agente
- **Actor**: Developer
- **Ejemplo**:
  ```
  $ mirai describe agent.yaml

  Agent: qa-assistant (v1)
  Responde preguntas usando un LLM

  Inputs:
    question  text  required  Pregunta del usuario
    context   text  optional  Contexto adicional opcional

  Outputs:
    answer      text    Respuesta generada por el LLM
    confidence  number  Nivel de confianza (0.0-1.0)
  ```

### CLI: mirai validate (mejorada)

- **Cambio**: ademas de validar grafo (IDs, edges, cycles), valida consistencia de data_map vs ToolSpec inputs
- **Nuevo check**: para cada data_map expression, verificar que el source node existe en el grafo y que el target param es un input valido del tool

### HTTP: POST /api/agents/{id}/execute (modificada)

- **Cambio**: `trigger_data` se valida contra spec.inputs del agente
- **Error response**:
  ```json
  {
    "error": "input validation failed",
    "details": [
      { "field": "question", "error": "missing_required", "message": "missing required input: question" }
    ]
  }
  ```
- **Status code**: 422 Unprocessable Entity (no 400, porque el JSON es valido pero el contenido no cumple)

### HTTP: GET /api/agents/{id}/schema (nueva)

- **Proposito**: retornar contrato del agente como JSON
- **Response**:
  ```json
  {
    "name": "qa-assistant",
    "version": "v1",
    "description": "Responde preguntas usando un LLM",
    "inputs": {
      "question": { "type": "text", "required": true, "description": "..." },
      "context": { "type": "text", "required": false, "description": "..." }
    },
    "outputs": {
      "answer": { "type": "text", "description": "..." }
    }
  }
  ```

### Agent Spec YAML (formato actualizado)

```yaml
name: qa-assistant
version: v1
description: "Responde preguntas usando un LLM"

inputs:
  question:
    type: text
    required: true
    description: "Pregunta del usuario"
  context:
    type: text
    required: false
    description: "Contexto adicional opcional"

outputs:
  answer:
    type: text
    description: "Respuesta generada por el LLM"
  confidence:
    type: number
    description: "Nivel de confianza (0.0-1.0)"

graph:
  nodes:
    - id: trigger
      tool_type: trigger/manual
    - id: llm
      tool_type: ai/llm_call
      config:
        prompt: "Contexto: ${trigger.payload.context}\n\nPregunta: ${trigger.payload.question}"
        model: gemma3
    - id: out
      tool_type: output/response
      config:
        message: "{{llm_response}}"
  edges:
    - source: trigger
      target: llm
      data_map:
        question: trigger.payload.question
        context: trigger.payload.context
    - source: llm
      target: out
      data_map:
        llm_response: llm.text
```

---

## Matriz de Permutaciones

### Capa 1: AgentSpec Input Validation

| Flujo | Permutacion | Actor | Resultado esperado |
|---|---|---|---|
| validate_inputs | Happy: todos los required presentes, tipos correctos | Developer | Payload validado, ejecucion procede |
| validate_inputs | Missing required input | Developer | Error 422: "missing required input: question" |
| validate_inputs | Tipo incorrecto (number donde espera text) | Developer | Error 422: "input 'question': expected text, got number" |
| validate_inputs | Multiples errores simultaneos | Developer | Lista de todos los errores (no fail-fast) |
| validate_inputs | Optional ausente sin default | Developer | OK, campo no presente en payload |
| validate_inputs | Optional ausente con default | Developer | OK, default insertado en payload |
| validate_inputs | Campos extras no declarados | Developer | OK, extras pasan sin tocar |
| validate_inputs | Spec sin inputs (None) | Developer | Sin validacion, backward compat total |
| validate_inputs | Payload vacio con required | Developer | Error: todos los required faltan |
| validate_inputs | Input type=json con objeto valido | Developer | OK |
| validate_inputs | Input type=json con string | Developer | Error: expected json, got text |
| describe_agent | Happy: spec con inputs y outputs | Developer | Contrato portable mostrado |
| describe_agent | Spec sin inputs ni outputs | Developer | Solo name/version/description |
| extract_outputs | Happy: terminal node tiene campos declarados | Engine | Outputs extraidos |
| extract_outputs | Terminal node no tiene campo declarado | Engine | Campo ausente (null), sin error |

### Capa 2: Node Input Validation

| Flujo | Permutacion | Actor | Resultado esperado |
|---|---|---|---|
| validate_node | Happy: todos los required del ToolSpec presentes | Engine | Tool.execute() procede |
| validate_node | Required ToolField ausente | Engine | Error controlado antes de execute |
| validate_node | FieldType mismatch | Engine | Error controlado antes de execute |
| validate_node | Default de ToolField aplicado | Engine | Input con default, execute procede |
| resolve_expr | Nested: trigger.payload.question | Engine | Value resuelto correctamente |
| resolve_expr | Deep nesting: a.b.c.d (4 niveles) | Engine | Value resuelto |
| resolve_expr | Path intermedio no es object | Engine | None retornado, sin panic |
| resolve_expr | Node no ejecutado aun | Engine | None retornado |
| resolve_expr | Template con nested: ${trigger.payload.q} | Engine | Template resuelto |
| catch_unwind | Tool paniquea con unwrap | Engine | ToolError retornado, proceso vivo |
| catch_unwind | Tool retorna error normal | Engine | ToolError propagado normalmente |

### Backward Compatibility

| Flujo | Permutacion | Actor | Resultado esperado |
|---|---|---|---|
| yaml_only | Spec YAML sin inputs/outputs | Developer | Parsea OK, sin validacion |
| yaml_only | Archivo .json pasado a from_file | Developer | Parsea OK (serde_yaml come JSON) |
| yaml_only | from_json() llamado en codigo | Developer | Error de compilacion (metodo eliminado) |
| backward_compat | data_map con 1 nivel (trigger.user_input) | Developer | Funciona (user_input alias) |
| backward_compat | CLI --input sin spec.inputs | Developer | Inyecta sin validar (como hoy) |
| yaml_only | to_file() siempre produce YAML | Developer | Archivo .yaml generado |

---

## Escenarios GWT

### Journey: Developer — Input Validation via CLI

TEST-058: Happy path — inputs validos via CLI
  Given: Agent YAML con inputs: question (text, required), context (text, optional)
  When: Developer ejecuta `mirai run agent.yaml --input '{"question":"hola","context":"sobre IA"}'`
  Then: Payload validado, agente ejecuta, resultado retornado

TEST-059: Missing required input via CLI
  Given: Agent YAML con inputs: question (text, required)
  When: Developer ejecuta `mirai run agent.yaml --input '{}'`
  Then: Error: "missing required input: question (Pregunta del usuario)"
  And: Agente NO ejecuta

TEST-060: Type mismatch via CLI
  Given: Agent YAML con inputs: question (text, required)
  When: Developer ejecuta `mirai run agent.yaml --input '{"question": 42}'`
  Then: Error: "input 'question': expected text, got number"

TEST-061: Default aplicado para input opcional
  Given: Agent YAML con inputs: context (text, optional, default: "sin contexto")
  When: Developer ejecuta `mirai run agent.yaml --input '{"question":"hola"}'`
  Then: Payload inyectado con context="sin contexto"
  And: Agente ejecuta con el default

TEST-062: Backward compat — spec sin inputs
  Given: Agent YAML sin seccion inputs (formato v0.1.0)
  When: Developer ejecuta `mirai run agent.yaml --input '{"whatever": 123}'`
  Then: Payload inyectado sin validacion (identico a comportamiento actual)
  And: Agente ejecuta normalmente

TEST-063: mirai describe muestra contrato
  Given: Agent YAML con inputs y outputs definidos
  When: Developer ejecuta `mirai describe agent.yaml`
  Then: Muestra nombre, version, inputs con tipos/required, outputs con tipos

### Journey: Host App — Input Validation via HTTP

TEST-064: HTTP execute con inputs validos
  Given: Server corriendo, agente cargado con spec.inputs
  When: Host App hace POST /api/agents/{id}/execute con trigger_data valido
  Then: 200 OK con resultado de ejecucion real

TEST-065: HTTP execute con input invalido
  Given: Server corriendo, agente con spec.inputs
  When: Host App hace POST /api/agents/{id}/execute con trigger_data faltante required
  Then: 422 con error detallado: campo, tipo de error, mensaje
  And: Agente NO ejecuta

TEST-066: GET /api/agents/{id}/schema
  Given: Agente cargado con inputs y outputs
  When: Host App hace GET /api/agents/{id}/schema
  Then: 200 con JSON del contrato (inputs + outputs + metadata)

### Journey: Engine — Node Input Validation (Capa 2)

TEST-067: Node recibe todos los required inputs → ejecuta
  Given: Grafo con trigger → llm (required: prompt)
  When: data_map mapea trigger.payload.question a prompt
  And: Payload incluye question
  Then: validate_node_inputs pasa, llm.execute() se llama

TEST-068: Node missing required input → error controlado
  Given: Grafo con trigger → llm (required: prompt)
  When: data_map NO mapea nada a prompt, y node.config NO tiene default
  Then: Error: "Node 'llm': missing required input 'prompt' (type: ai/llm_call)"
  And: tool.execute() NUNCA se llama
  And: failure_mode del nodo aplica (Stop/Skip/Route)

TEST-069: FieldType mismatch en nodo → error controlado
  Given: Grafo donde data_map envia Number a un campo que espera String
  When: validate_node_inputs corre
  Then: Error: "Node 'llm': input 'prompt' expected String, got Number"

TEST-070: Default de ToolField aplicado
  Given: Tool con input optional que tiene default en ToolField.default
  When: Input no resuelto por data_map ni por node.config
  Then: Default del ToolField insertado, tool.execute() recibe el default

### Journey: Engine — Nested Field Traversal

TEST-071: Nested access trigger.payload.question resuelve
  Given: Trigger output = { payload: { question: "hola" }, triggered_by: "manual" }
  When: data_map expression = "trigger.payload.question"
  Then: resolve_expression retorna Some("hola")

TEST-072: Deep nesting a.b.c.d resuelve
  Given: Node output = { b: { c: { d: "deep" } } }
  When: data_map expression = "a.b.c.d" (donde a es el node_id)
  Then: resolve_expression retorna Some("deep")

TEST-073: Path intermedio no es object → None sin panic
  Given: Node output = { data: "string" }
  When: data_map expression = "node.data.subfield"
  Then: resolve_expression retorna None (data es string, no object)
  And: NO panic

TEST-074: Template con nested fields
  Given: Template = "Pregunta: ${trigger.payload.question}"
  When: trigger.payload.question = "hola"
  Then: Template resuelto a "Pregunta: hola"

### Journey: Engine — Crash Prevention

TEST-075: Tool panic → ToolError, proceso vive
  Given: Tool que hace `panic!("bug")` internamente
  When: Runner ejecuta ese nodo
  Then: catch_unwind captura el panic
  And: ToolError retornado con mensaje del panic
  And: Proceso sigue vivo para ejecutar siguientes nodos (segun failure_mode)

TEST-076: Cadena de 3 nodos — datos fluyen end-to-end
  Given: trigger → llm → output con data_map completo
  When: Client envia payload valido
  Then: trigger.payload → llm recibe inputs correctos → output produce resultado
  And: ExecutionResult.state tiene outputs de los 3 nodos
  And: Cero warnings, cero campos faltantes

TEST-077: YAML spec round-trip
  Given: Agent YAML con inputs, outputs, graph completo
  When: from_yaml(yaml_string) → to_yaml() → from_yaml(result)
  Then: Spec original y reparsed son logicamente identicos
  And: inputs/outputs preservados

TEST-078: YAML-only — from_file parsea cualquier sintaxis
  Given: Archivo agent.json con spec en JSON syntax
  When: AgentSpec::from_file("agent.json")
  Then: Parsea OK (serde_yaml es superset de JSON)
  And: Spec identico al equivalente YAML

TEST-079: YAML-only — to_file siempre produce YAML
  Given: AgentSpec cargado en memoria
  When: spec.to_file("output.yaml")
  Then: Archivo generado en sintaxis YAML (no JSON)
  And: Sin llaves, sin comillas innecesarias, con indentacion YAML

---

## Fuera de Alcance

- **Output validation en runtime**: outputs son declarativos en v1. No se valida que el agente realmente produzca los outputs declarados. Futuro PRD.
- **Output source mapping**: campo `source: "node.field"` en OutputFieldSpec para extraccion explicita. Futuro PRD.
- **Schema generation para SDKs**: generar TypeScript interfaces o Python dataclasses desde spec.inputs. Futuro (cuando SDKs existan).
- **Input schema composition**: heredar inputs de otro spec, merge de schemas. Futuro.
- **Validation para edge conditions**: validar que condition.field existe en el output del source node. Candidato para v2 de este PRD.
- **Nuevos tool types**: este PRD no agrega tools nuevos.
- **Cambios a fan-out/fan-in**: la logica de paralelismo no cambia, solo se agrega validacion pre-execute a cada nodo paralelo.

---

## Dependencias

- **No depende de PRD-002**: los cambios son al parser de AgentSpec y al runner core, no al server. Puede implementarse en paralelo.
- **No depende de PRD-003**: MCP tools se benefician de la validacion automaticamente (Capa 2 valida contra ToolSpec de mcp/call).
- **Dependencia interna**: `tools/base.rs` (ToolSpec, FieldType, ToolField) ya existe. Este PRD lo usa, no lo modifica.
- **serde_yaml**: ya es dependencia. Se convierte en el UNICO parser de specs. `serde_json` se mantiene solo para tipos internos (`Value`) y HTTP API.
- **indexmap** (recomendado): para preservar orden de campos en inputs/outputs en YAML. Opcional — HashMap funciona pero no preserva orden.
- **Eliminar**: `AgentSpec::from_json()`, `AgentSpec::to_json()` como metodos publicos. Codigo interno que los use → migrar a `from_yaml()`/`to_yaml()`.


### Orden de implementacion recomendado

```
Fase 1 (foundation):
  1. InputFieldSpec, OutputFieldSpec, InputType → structs + serde
  2. AgentSpec + inputs/outputs → Option fields con #[serde(default)]
  3. SharedState.get_field nested traversal
  4. Tests de parseo YAML round-trip

Fase 2 (Capa 1):
  5. validate_agent_inputs()
  6. inject_validated_payload() (reemplaza mock_payload injection)
  7. Trigger output rename (user_input → payload, con compat)
  8. CLI --input validation integration
  9. HTTP execute validation integration
  10. Tests Capa 1

Fase 3 (Capa 2):
  11. validate_node_inputs() en runner loop
  12. catch_unwind wrapper en execute_with_retry
  13. Tests Capa 2

Fase 4 (interfaces):
  14. mirai describe
  15. GET /api/agents/{id}/schema
  16. mirai validate mejorado
  17. extract_agent_outputs (best-effort)
  18. Tests de integracion end-to-end
```
