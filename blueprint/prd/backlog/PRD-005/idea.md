# Engine v0.3.0 — Completar Integraciones Reales

| Campo | Valor |
|-------|-------|
| **ID** | PRD-005 |
| **Fecha** | 2026-05-27 |
| **Estado** | in_progress |
| **Branch** | prd/PRD-005 |
| **Target** | v0.3.0 |

---

## Diagrama General

```
┌──────────────────────────────────────────────────────────────┐
│                    ENGINE v0.3.0 SCOPE                        │
│                                                              │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────┐     │
│  │ S1: Eval    │  │ S2: GroupChat│  │ S3: A2A         │     │
│  │ LLM Judge   │  │ Real Loop   │  │ Message Queue   │     │
│  │ + CLI       │  │ N rounds    │  │ + Tool          │     │
│  └──────┬──────┘  └──────┬──────┘  └────────┬────────┘     │
│         │                │                   │               │
│  ┌──────┴──────┐  ┌──────┴───────┐  ┌───────┴────────┐     │
│  │ S4: RAG     │  │ S5: CLI      │  │ S6: SDKs       │     │
│  │ Tool in     │  │ Complete     │  │ Tests +         │     │
│  │ Graph       │  │ All Commands │  │ Publish-ready   │     │
│  └─────────────┘  └──────────────┘  └────────────────┘     │
│                                                              │
│  Dependencias:                                               │
│    S1 ──▶ S5 (mirai eval CLI usa eval module)                │
│    S4 ──▶ S5 (mirai rag CLI usa rag + embeddings)            │
│    S2 ──▶ S3 (GroupChat usa A2A internamente)                │
│    S1..S5 ──▶ S6 (SDKs testean contra features reales)      │
└──────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: mejora (completar integraciones parciales)
- **Resumen**: Engine v0.2.0 tiene 13 features E2E validadas. Quedan 6 módulos con código structural (tipos, funciones standalone) que no están integrados en el flujo de ejecución real: Eval no llama al LLM, GroupChat no ejecuta rondas, A2A no entrega mensajes, RAG no es un tool en el graph, CLI tiene stubs, SDKs no tienen tests.
- **Actores**: Developer (CLI/SDK), Host App (HTTP API), Agent (ejecución en graph)
- **Flujos tocados**: eval post-execution, multi-agent debate, agent-to-agent messaging, RAG en graph, CLI completeness, SDK consumability
- **Qué cambia**:
  - HOY: Eval es standalone, GroupChat son tipos, A2A son tipos, RAG es solo endpoint, CLI tiene stubs, SDKs sin tests
  - DESPUÉS: Todo integrado end-to-end, todo validado con servicios reales, zero fachadas

---

## Actores y Permisos

| Actor | Capacidad | Acción | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | evaluar_sesion | Ejecutar eval via CLI o API | Scores + details |
| Developer | gestionar_rag | Ingestar + buscar via CLI | Colecciones + resultados |
| Developer | gestionar_universe | Iniciar universe, enviar mensajes via CLI | Routing + respuestas |
| Developer | gestionar_agentes | Cargar y listar agentes via CLI | Agentes registrados |
| Host App | ejecutar_groupchat | Ejecutar debate multi-agente via API | Transcript + consensus |
| Agent | buscar_rag | Ejecutar data/rag_search dentro de un graph | Chunks relevantes |
| Agent | enviar_a2a | Enviar mensaje a otro agente dentro de un graph | Delivery confirmation |

---

## Entidades

### EvalRun (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| id | id | sí | Identificador |
| session_id | texto | sí | Sesión evaluada |
| eval_types | lista de texto | sí | Tipos ejecutados |
| scores | json | sí | Score por tipo (0.0-1.0) |
| details | json | no | Razones del LLM judge |
| judge_model | texto | no | Modelo usado |
| duration_ms | número | sí | Tiempo de evaluación |

### A2AQueue (nueva — runtime)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| agent_id | texto | sí | Agente receptor |
| messages | lista de A2AMessage | sí | Cola FIFO |
| max_size | número | sí | Límite (default 100) |

### RAGCollection (nueva — runtime)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| id | id | sí | Identificador |
| name | texto | sí | Nombre |
| chunks | lista de Chunk | sí | Chunks con embeddings |
| embedding_model | texto | sí | Modelo usado |
| dimensions | número | sí | Dimensión del vector |

---

## Reglas de Negocio

### regla-eval-llm-real
- **Invariante**: Eval LLM-as-judge DEBE llamar al LLM real configurado, no mock. Si LLM no disponible → error explícito.
- **Cuándo se verifica**: al ejecutar eval types relevance/faithfulness/completeness
- **Si se viola**: Error: "LLM judge unavailable"

### regla-groupchat-min-agents
- **Invariante**: GroupChat requiere mínimo 2 agentes participantes.
- **Cuándo se verifica**: al iniciar groupchat
- **Si se viola**: Error: "GroupChat requires at least 2 participants"

### regla-a2a-same-universe
- **Invariante**: A2A solo entre agentes del mismo contexto de ejecución.
- **Cuándo se verifica**: al enviar mensaje
- **Si se viola**: Error: "target agent not found"

### regla-rag-tool-embeddings-real
- **Invariante**: data/rag_search tool genera embeddings reales via context.llm().embed().
- **Cuándo se verifica**: al ejecutar el tool
- **Si se viola**: Error: "embedding model not available"

### regla-sdk-tests-reales
- **Invariante**: SDK tests ejecutan contra el server real, no mocks.
- **Cuándo se verifica**: al correr test suite del SDK
- **Si se viola**: Test falla con connection error (no fake success)

---

## Operaciones

### S1: Eval LLM-as-Judge

#### ejecutar_eval
- **Actor**: Developer
- **Input**: session_id (texto), eval_types (lista), judge_model (texto, opcional)
- **Output exitoso**: EvalRun con scores por tipo
- **Lógica**:
  1. Cargar sesión completada (state + transcript)
  2. Para `format_compliance` y `latency` → evaluación programática (ya existe)
  3. Para `relevance`, `faithfulness`, `completeness` → build_judge_prompt() + llamar LLM real + parse_judge_response()
  4. Agregar scores al EvalRun
- **Errores**: session not found, session not completed, LLM unavailable

#### eval_post_hook
- **Actor**: Engine (automático)
- **Input**: AgentSpec con `config.eval_types` definido
- **Output**: EvalRun ejecutado automáticamente al terminar el graph
- **Lógica**: Si spec.config tiene campo `eval_types`, ejecutar eval después de cada ejecución completada

### S2: GroupChat Real

#### ejecutar_groupchat
- **Actor**: Host App / Developer
- **Input**: topic, participants (lista de AgentSpecs con Soul), max_rounds, moderator_strategy
- **Output exitoso**: GroupChatResult con transcript + consensus
- **Lógica por ronda**:
  1. Moderador selecciona speaker (round_robin o topic_based)
  2. Speaker recibe: topic + transcript previo como contexto
  3. Speaker genera respuesta via LLM real (usando su Soul como system prompt)
  4. Respuesta se agrega al transcript
  5. Evaluar consensus: si últimos N mensajes coinciden en conclusión → consensus=true
  6. Si max_rounds → parar con consensus=false
- **Errores**: less than 2 agents, LLM unavailable

### S3: A2A Message Delivery

#### enviar_mensaje_a2a
- **Actor**: Agent (via tool `agent/send_message`)
- **Input**: target_agent_name, message_type, payload
- **Output exitoso**: delivery_id, status=delivered
- **Lógica**: Almacenar en A2AQueue del target. Si target tiene handler registrado → ejecutar.
- **Errores**: target not found, queue full

### S4: RAG Search Tool

#### buscar_rag_en_graph
- **Actor**: Agent (via tool `data/rag_search`)
- **Input**: query (texto), documents (lista de texto), top_k (número)
- **Output exitoso**: chunks rankeados con scores
- **Lógica**: Reutiliza la misma lógica del endpoint /api/rag/search pero como tool registrado en el ToolRegistry
- **Errores**: embedding model not available

### S5: CLI Commands

#### mirai eval
- **Input**: `mirai eval <session_id> --types relevance,format_compliance`
- **Output**: Tabla con scores por evaluador

#### mirai rag
- **Input**: `mirai rag search --query "deploy" --documents ./docs/*.md --top-k 3`
- **Output**: Chunks rankeados con scores

#### mirai universe
- **Input**: `mirai universe send --config universe.yaml --message "analyze sales"`
- **Output**: Routing decision + agent response

#### mirai agent load/list
- **Input**: `mirai agent load agent.yaml` / `mirai agent list`
- **Output**: Confirmación / lista de agentes

### S6: SDKs con Tests

#### test_python_sdk
- **Lógica**: Arrancar server con Ollama, ejecutar `engine.run()` y `engine.stream()` contra agente real, verificar que response no es mock
- **Archivos**: `sdks/python/tests/test_integration.py`

#### test_typescript_sdk
- **Lógica**: Igual pero con fetch contra server real
- **Archivos**: `sdks/typescript/tests/integration.test.ts`

---

## Interfaces

### CLI (comandos nuevos/completados)

| Comando | Tipo | Descripción |
|---------|------|-------------|
| `mirai eval <session_id> --types <list>` | Nuevo | Evaluar sesión |
| `mirai rag search --query <q> --documents <paths> --top-k <n>` | Nuevo | RAG search |
| `mirai universe send --config <yaml> --message <msg>` | Nuevo | Enviar a universe |
| `mirai agent load <file>` | Completar stub | Cargar agente |
| `mirai agent list` | Completar stub | Listar agentes |

### HTTP Endpoints (nuevos)

| Método | Ruta | Descripción |
|--------|------|-------------|
| POST | /api/eval | Ejecutar eval sobre sesión |
| POST | /api/universe/groupchat | Ejecutar debate multi-agente |

### Tools (nuevos/actualizados)

| Tool | Tipo | Descripción |
|------|------|-------------|
| `data/rag_search` | Nuevo | Buscar en RAG dentro del graph |
| `agent/send_message` | Actualizar | Entrega real de A2A messages |

---

## Matriz de Permutaciones

| Flujo | Permutación | Actor | Resultado esperado |
|---|---|---|---|
| eval | Happy: format_compliance + latency (programático) | Developer | Scores sin LLM |
| eval | Happy: relevance + faithfulness (LLM judge) | Developer | LLM real evalúa, scores 0-1 |
| eval | Session not found | Developer | Error: not found |
| eval | Session not completed | Developer | Error: not completed |
| eval | LLM unavailable | Developer | Error: judge unavailable |
| groupchat | Happy: 3 agents, 3 rounds, consensus | Developer | Transcript + consensus=true |
| groupchat | Max rounds sin consensus | Developer | Transcript + consensus=false |
| groupchat | Less than 2 agents | Developer | Error: requires 2+ |
| a2a | Happy: send + receive | Agent | Message delivered |
| a2a | Target not found | Agent | Error: not found |
| rag_tool | Happy: query + documents → results | Agent | Top-K chunks con scores |
| rag_tool | No embedding model | Agent | Error: model unavailable |
| cli eval | Happy path | Developer | Scores table |
| cli rag | Happy path | Developer | Chunks listed |
| cli universe | Happy path | Developer | Routing + response |
| cli agent load | Happy path | Developer | Agent registered |
| cli agent list | With agents | Developer | List with names/IDs |
| sdk python | engine.run() returns real response | Developer | response != mock |
| sdk python | engine.stream() yields events | Developer | Events received |
| sdk typescript | engine.run() returns real response | Developer | response != mock |

---

## Escenarios GWT

TEST-058: Eval LLM-as-judge con Ollama real
  Given: Sesión completada con input "What is AI?" y output "AI is..."
  When: Developer ejecuta eval con types=[relevance]
  Then: LLM real genera score (0.0-1.0) con reasoning

TEST-059: Eval programático (format + latency)
  Given: Sesión completada con output JSON válido, duration 500ms
  When: Developer ejecuta eval con types=[format_compliance, latency]
  Then: format_compliance=1.0, latency score basado en 500ms

TEST-060: GroupChat 3 agentes, 3 rondas
  Given: 3 agents con Souls distintos, topic="Should we use microservices?"
  When: POST /api/universe/groupchat con max_rounds=3
  Then: 3 rondas de debate, cada agente habla una vez por ronda, transcript completo

TEST-061: A2A send_message tool en graph
  Given: Agent A con nodo agent/send_message, Agent B registrado
  When: Agent A ejecuta y envía request a B
  Then: Message entregado con delivery_id

TEST-062: data/rag_search tool en graph
  Given: Agent con nodo data/rag_search, documentos en config
  When: Agent ejecuta con query "deployment process"
  Then: Top-K chunks retornados con similarity scores reales

TEST-063: mirai eval CLI
  Given: Server corriendo, sesión completada
  When: mirai eval <session_id> --types relevance
  Then: Score mostrado en consola

TEST-064: mirai rag search CLI
  Given: Archivos markdown en ./docs/
  When: mirai rag search --query "deploy" --documents ./docs/*.md
  Then: Chunks relevantes mostrados con scores

TEST-065: mirai agent load + list
  Given: Archivo agent.yaml válido
  When: mirai agent load agent.yaml && mirai agent list
  Then: Agent registrado, listado con nombre e ID

TEST-066: Python SDK integration test
  Given: Server corriendo con Ollama
  When: engine.run(agent) ejecuta
  Then: result.response no contiene "mock", tokens > 0

TEST-067: TypeScript SDK integration test
  Given: Server corriendo con Ollama
  When: engine.run(agent) ejecuta
  Then: result no contiene "mock"

---

## Fuera de Alcance

- Channel adapters (Telegram, Slack, WhatsApp) → v0.4.0
- Voice STT/TTS → v0.4.0
- Visual builder → Mirai Local
- npm publish / PyPI publish (preparamos pero no publicamos)
- Eval dashboard UI

---

## Dependencias

```
S1 (Eval) → independiente, usa LLM real del server
S2 (GroupChat) → usa S1 conceptualmente (eval de consensus)
S3 (A2A) → independiente, runtime
S4 (RAG tool) → usa lógica existente de /api/rag/search
S5 (CLI) → depende de S1, S4 para los comandos
S6 (SDKs) → depende de S1-S5 estar funcionales

Orden: S4 → S1 → S3 → S2 → S5 → S6
```
