# ROADMAP: Paridad competitiva Mirai Engine vs LangGraph

> Cada item aquí es un gap donde LangGraph tiene ventaja HOY.
> Mirai tiene la arquitectura para resolverlos todos — y mejor.
> Prioridad: lo que más impacta adopción y producción primero.

---

## P0 — Crítico para adopción (sin esto no compites)

### GAP-001: Ecosystem de integraciones (LLM providers reales)
- **LangGraph tiene**: ChatOpenAI, ChatAnthropic, ChatOllama, ChatGoogle, 20+ providers con 1 línea
- **Mirai tiene**: OllamaAdapter, OpenAIAdapter, ClaudeAdapter, GeminiAdapter, GroqAdapter
- **Gap real**: los adapters existen pero el CLI usa MockLLM por defecto. Falta configuración de LLM provider desde el agent spec o config del engine
- **Solución**: flag `--llm-provider ollama --llm-url http://localhost:11434` en el CLI, o bloque `resources:` en el agent JSON que el engine lea y configure el adapter real
- **Impacto**: sin esto, los demos muestran mock responses — mata la demo en vivo

### GAP-002: Structured Output con validación real
- **LangGraph tiene**: `with_structured_output(PydanticModel)` — el LLM responde JSON validado
- **Mirai tiene**: `output_schema` en config del nodo `ai/llm_call` — el engine inyecta instrucciones + auto-retry
- **Gap real**: funciona con LLM real pero necesita el GAP-001 resuelto primero
- **Solución**: ya diseñado (schema_retries + re-prompt automático). Solo necesita LLM real conectado
- **Impacto**: alto — structured output es el feature #1 que usan los developers

### GAP-003: Observabilidad (equivalente a LangSmith)
- **LangGraph tiene**: LangSmith — tracing visual, evaluaciones, debugging, replay
- **Mirai tiene**: trace + transcript (JSON plano)
- **Gap real**: no hay UI para inspeccionar ejecuciones, no hay tracing anidado
- **Solución**:
  - Fase 1: endpoint GET /sessions/{id}/trace con formato OpenTelemetry-compatible
  - Fase 2: UI web simple (timeline de nodos + inputs/outputs + duración)
  - Fase 3: integración con Grafana/Datadog via OTLP export
- **Impacto**: sin observabilidad, producción es a ciegas

---

## P1 — Importante para producción (te diferencia de "juguete")

### GAP-004: Fan-out dinámico (equivalente a Send())
- **LangGraph tiene**: `Send("worker", {"item": x}) for x in items` — N instancias paralelas en runtime
- **Mirai tiene**: `logic/parallel` experimental, o múltiples sesiones desde el host
- **Gap real**: no hay fan-out nativo dentro del grafo donde N es dinámico
- **Solución**: nodo `logic/fan_out` que:
  - Recibe un array de items
  - Crea N ejecuciones paralelas de un sub-grafo
  - Consolida resultados en un nodo `logic/fan_in`
  - Tokio hace el parallelismo real (ventaja Rust)
- **Impacto**: map-reduce es patrón fundamental en pipelines de datos

### GAP-005: Cross-thread memory (Store con semantic search)
- **LangGraph tiene**: `Store` con `store.put()` / `store.search()` — memoria entre conversaciones con búsqueda semántica
- **Mirai tiene**: `LongTermMemory` (pattern/error_correction/feedback) — sin semantic search
- **Gap real**: no hay forma de que un agente "recuerde" entre sesiones con búsqueda por similitud
- **Solución**:
  - Extender `VectorResource` para que sea accesible como memoria del agente
  - Nuevo nodo `memory/store` y `memory/search`
  - Config en agent spec: `memory: { provider: "pgvector", namespace: "user_{id}" }`
- **Impacto**: chatbots y agentes conversacionales lo necesitan

### GAP-006: Subgrafos composables
- **LangGraph tiene**: `parent.add_node("research", research_subgraph)` — grafo dentro de grafo con state mapping
- **Mirai tiene**: `agent/call_agent` — ejecuta otro agente como sub-grafo
- **Gap real**: el mapping de state entre parent y child no es tan fluido. Falta isolation de checkpoints
- **Solución**:
  - `agent/call_agent` ya existe — mejorar el data_map bidireccional (input mapping + output mapping)
  - Namespace isolation para checkpoints del sub-agente
  - Documentar patrones: supervisor → workers, pipeline → sub-pipelines
- **Impacto**: modularidad = reusabilidad de agentes

### GAP-007: Middleware lifecycle (hooks más granulares)
- **LangGraph tiene**: `@before_model`, `@after_model`, `@wrap_tool_call`, `@dynamic_prompt`, agent jumps
- **Mirai tiene**: 7 hooks (pre/post block, pre/post LLM, on_error, graph start/end)
- **Gap real**: Mirai ya tiene hooks pero falta:
  - `@dynamic_prompt` (prompt que cambia según contexto/usuario)
  - Agent jumps (saltar a END desde un hook)
  - Hook de selección de modelo (usar modelo diferente según complejidad de query)
- **Solución**: extender HookHandler con:
  - `on_model_select(node, context) → model_override`
  - `on_prompt_build(node, context) → modified_prompt`
  - HookResult::Jump("node_id") para redireccionar
- **Impacto**: medio — power users lo necesitan, la mayoría no

---

## P2 — Diferenciadores que LangGraph NO tiene (ventaja Mirai)

Estos NO son gaps — son features donde Mirai ya gana. Hay que pulirlos y marketearlos.

### VENTAJA-001: Portabilidad cross-platform
- Ya funciona. Pulir: documentar cómo embedir el binario en iOS/Android/WASM
- Marketing: "same YAML, any platform"

### VENTAJA-002: Determinismo del DAG
- El grafo decide el flujo, no el LLM. Auditable, predecible, testeable
- Pulir: herramienta de visualización de grafos (ya existe en el presenter)
- Marketing: "your agent is data, not code"

### VENTAJA-003: Triggers nativos
- Webhook, schedule, event, manual — integrados en el motor
- LangGraph NO tiene triggers — es solo una librería
- Pulir: más trigger types (Kafka, SQS, pub/sub)

### VENTAJA-004: Vault integrado
- Credential management con encryption at-rest
- LangGraph delega 100% a soluciones externas
- Pulir: integración con AWS Secrets Manager, GCP Secret Manager

### VENTAJA-005: Single binary deployment
- `cargo build --release` → 8MB binario → deploy en cualquier server
- LangGraph necesita: Python runtime + pip install + virtualenv + 200MB de deps
- Marketing: "zero dependency deployment"

---

## Orden de ejecución sugerido

```
Sprint 1 (inmediato):  GAP-001 (LLM providers reales en CLI)
                       → Sin esto no hay demo real

Sprint 2:             GAP-002 (structured output real)
                      GAP-004 (fan-out dinámico)
                       → Features core de producción

Sprint 3:             GAP-003 (observabilidad básica)
                      GAP-005 (cross-thread memory)
                       → Producción seria

Sprint 4:             GAP-006 (subgrafos mejorados)
                      GAP-007 (middleware granular)
                       → Power users

Paralelo:             Pulir VENTAJA-001 a 005
                       → Marketing y documentación
```

---

## Filosofía

LangGraph resolvió estos problemas primero. Eso es valioso — nos ahorra la investigación de "qué necesita el mercado". Pero lo resolvieron dentro de las limitaciones de Python.

Mirai puede resolver CADA UNO de estos gaps **mejor** porque:
1. Rust performance → fan-out real con Tokio, no fake async con GIL
2. Binario portable → observabilidad embebida sin dependencias externas
3. Config declarativa → middleware como config, no como decoradores Python
4. 3-layer architecture → cada capa mejora independientemente

No estamos copiando — estamos reimplementando con una arquitectura superior.
