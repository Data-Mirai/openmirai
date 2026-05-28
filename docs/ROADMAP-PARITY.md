# ROADMAP: Paridad competitiva Mirai Engine vs LangGraph

> Cada item aqui es un gap donde LangGraph tiene ventaja HOY.
> Mirai tiene la arquitectura para resolverlos todos — y mejor.
> Prioridad: lo que mas impacta adopcion y produccion primero.

---

## P0 — Resueltos

### ~~GAP-001: Ecosystem de integraciones (LLM providers reales)~~ RESUELTO v0.4.0
- **LangGraph tiene**: ChatOpenAI, ChatAnthropic, ChatOllama, ChatGoogle, 20+ providers con 1 linea
- **Mirai tiene**: 7 providers reales (Ollama, Claude, OpenAI, Gemini, Groq, NVIDIA NIM, OpenRouter)
- **Resolucion**: CLI soporta `--provider X --api-key $KEY`. Resolucion: flag > env var > auto-detect > default Ollama. Demos con LLM real funcionan.

### ~~GAP-002: Structured Output con validacion real~~ RESUELTO v0.4.0
- **LangGraph tiene**: `with_structured_output(PydanticModel)` — el LLM responde JSON validado
- **Mirai tiene**: `output_schema` en config de `ai/llm_call` — inyecta instrucciones + auto-retry si JSON invalido
- **Resolucion**: Funciona con todos los providers. Schema enforcement + retry automatico.

---

## P0 — Pendiente

### GAP-003: Observabilidad (equivalente a LangSmith)
- **LangGraph tiene**: LangSmith — tracing visual, evaluaciones, debugging, replay
- **Mirai tiene**: trace + transcript (JSON plano), SSE streaming con eventos por nodo, endpoint `/api/v1/metrics`
- **Gap real**: no hay UI para inspeccionar ejecuciones, no hay tracing anidado ni export OpenTelemetry
- **Solucion**:
  - Fase 1: endpoint GET /api/v1/sessions/{id}/trace con formato OpenTelemetry-compatible
  - Fase 2: UI web simple (timeline de nodos + inputs/outputs + duracion) — esto lo resuelve Mirai Local
  - Fase 3: integracion con Grafana/Datadog via OTLP export
- **Impacto**: sin observabilidad visual, produccion es a ciegas

---

## P1 — Importante para produccion

### GAP-004: Fan-out dinamico (equivalente a Send())
- **LangGraph tiene**: `Send("worker", {"item": x}) for x in items` — N instancias paralelas en runtime
- **Mirai tiene**: Fan-out estatico (multiples edges desde un nodo). `logic/loop` para iteracion.
- **Gap real**: no hay fan-out nativo donde N se determina en runtime desde un array
- **Solucion**: nodo `logic/fan_out` que:
  - Recibe un array de items
  - Crea N ejecuciones paralelas de un sub-grafo
  - Consolida resultados en un nodo `logic/fan_in`
  - Tokio hace el paralelismo real (ventaja Rust)
- **Impacto**: map-reduce es patron fundamental en pipelines de datos

### GAP-005: Cross-thread memory (Store con semantic search)
- **LangGraph tiene**: `Store` con `store.put()` / `store.search()` — memoria entre conversaciones con busqueda semantica
- **Mirai tiene**: `memory/` module (short/long-term), `data/vault_read/write`, `data/entity_store`
- **Gap real**: no hay busqueda por similitud semantica entre sesiones
- **Solucion**:
  - Extender `VectorResource` con embeddings reales (depende de GAP-H)
  - Nuevo nodo `memory/store` y `memory/search`
  - Config en agent spec: `memory: { namespace: "user_{id}" }`
- **Impacto**: chatbots y agentes conversacionales lo necesitan

### GAP-006: Subgrafos composables
- **LangGraph tiene**: `parent.add_node("research", research_subgraph)` — grafo dentro de grafo con state mapping
- **Mirai tiene**: `agent/run_agent` — ejecuta otro agente como sub-agente (max depth 3)
- **Gap real**: el mapping de state entre parent y child puede mejorar. Falta isolation de checkpoints.
- **Solucion**:
  - Mejorar `data_map` bidireccional en `agent/run_agent` (input mapping + output mapping)
  - Namespace isolation para checkpoints del sub-agente
  - Documentar patrones: supervisor → workers, pipeline → sub-pipelines
- **Impacto**: modularidad = reusabilidad de agentes

### GAP-007: Middleware lifecycle (hooks mas granulares)
- **LangGraph tiene**: `@before_model`, `@after_model`, `@wrap_tool_call`, `@dynamic_prompt`, agent jumps
- **Mirai tiene**: 7 hooks (pre/post block, pre/post LLM, on_error, graph start/end)
- **Gap real**: falta:
  - `on_model_select` (modelo diferente segun complejidad de query)
  - `on_prompt_build` (prompt dinamico segun contexto)
  - HookResult::Jump("node_id") para redireccionar desde hook
- **Impacto**: medio — power users lo necesitan, la mayoria no

---

## P2 — Diferenciadores que LangGraph NO tiene (ventaja Mirai)

Estos NO son gaps — son features donde Mirai ya gana. Pulirlos y marketearlos.

### VENTAJA-001: Portabilidad cross-platform
- Single binary compilado. Rust → cualquier OS/arch.
- Pulir: documentar como embedir en iOS/Android/WASM
- Marketing: "same YAML, any platform"

### VENTAJA-002: Determinismo del DAG
- El grafo decide el flujo, no el LLM. Auditable, predecible, testeable.
- Pulir: herramienta de visualizacion de grafos
- Marketing: "your agent is data, not code"

### VENTAJA-003: Triggers nativos
- Webhook, schedule, event, manual, heartbeat — integrados en el motor.
- LangGraph NO tiene triggers — es solo una libreria.
- Pulir: mas trigger types (Kafka, SQS, pub/sub)

### VENTAJA-004: Vault integrado
- Credential management built-in.
- LangGraph delega 100% a soluciones externas.
- Pulir: integracion con AWS Secrets Manager, GCP Secret Manager

### VENTAJA-005: Single binary deployment
- `cargo build --release` → ~9MB binario → deploy en cualquier server.
- LangGraph necesita: Python runtime + pip install + virtualenv + 200MB de deps.
- Marketing: "zero dependency deployment"

### VENTAJA-006: Input/Output contracts
- Typed validation en el boundary del agente. Errores claros antes de ejecutar.
- LangGraph no tiene validacion de inputs declarativa — es codigo Python.

### VENTAJA-007: MCP nativo
- Model Context Protocol integrado: `mcp_servers` en el agent spec.
- LangGraph requiere wrappers adicionales para MCP.

---

## Orden de ejecucion sugerido

```
Siguiente:    GAP-003 (observabilidad basica)
              → Sin esto, produccion es a ciegas

Despues:      GAP-004 (fan-out dinamico)
              GAP-005 (cross-thread memory)
              → Features core de produccion

Futuro:       GAP-006 (subgrafos mejorados)
              GAP-007 (middleware granular)
              → Power users

Paralelo:     Pulir VENTAJA-001 a 007
              → Marketing y documentacion
```

---

## Filosofia

LangGraph resolvio estos problemas primero. Eso es valioso — nos ahorra la investigacion de "que necesita el mercado". Pero lo resolvieron dentro de las limitaciones de Python.

Mirai puede resolver CADA UNO de estos gaps **mejor** porque:
1. Rust performance → fan-out real con Tokio, no fake async con GIL
2. Binario portable → observabilidad embebida sin dependencias externas
3. Config declarativa → middleware como config, no como decoradores Python
4. Arquitectura hexagonal → cada capa mejora independientemente

No estamos copiando — estamos reimplementando con una arquitectura superior.
