<!--
BLUEPRINT SEED — API.md
Responsable: → blueprint/agents/07-CONTRACTS.md

Estructura esperada:
- Por recurso, por endpoint: descripción, auth, guard, validaciones, request, response, errores
- Cada endpoint con anchor único: {#METODO-ruta}

Reglas:
- NO copiar la estructura de SCHEMA.md — REFERENCIAR con → SCHEMA.md#entidad-X
- Guards se definen aquí (el nombre concreto del guard vive en ARCHITECTURE.md)
- Reglas de negocio se REFERENCIAN a FLUJOS.md, no se duplican aquí
- No código de controllers/services
- Endpoints en /api/v1/ con kebab-case plural (o la convención del proyecto)
- Toda mutación requiere auth
- Paginación estándar para listados (page, page_size)
-->

# API.md — HTTP API Reference

Documentación de todos los endpoints del servidor Axum de openmirai-engine. Ver [DOMINIO.md](../producto/DOMINIO.md) para términos y [FLUJOS.md](../producto/FLUJOS.md) para reglas de negocio.

## Base URL

```
http://localhost:3000
```

## Authentication

**Header:** `X-API-Key`

Enviado en todas las requests excepto `/health` y `/version`:

```bash
curl -H "X-API-Key: tu-api-key" http://localhost:3000/api/v1/agents
```

**Configuración:**
- Variable de entorno: `MIRAI_API_KEY`
- Flag CLI: `--api-key tu-api-key`
- Si no está configurado, el servidor avisa pero no rechaza requests (modo abierto)

**Errores de auth:**
| Código | Condición | Mensaje |
|---|---|---|
| 401 | Header falta o es incorrecto | `{"error": "unauthorized"}` |

---

## Public Endpoints (Sin Auth)

### GET /health {#GET-health}

**Descripción:** Health check del servidor. Retorna status y métricas en tiempo real.

**Auth:** Público (no requiere API key).

**Response:** JSON con uptime, conteos y versión.

```json
{
  "status": "ok",
  "version": "0.6.0",
  "engine": "openmirai-engine-rs",
  "uptime_seconds": 1234,
  "agents_loaded": 5,
  "sessions_total": 42,
  "tools_registered": 50
}
```

---

### GET /version {#GET-version}

**Descripción:** Retorna versión del engine.

**Auth:** Público (no requiere API key).

**Response:**

```json
{
  "version": "0.6.0",
  "engine": "openmirai-engine-rs"
}
```

---

## Graphs CRUD

### POST /api/v1/graphs {#POST-graphs}

**Descripción:** Crear un nuevo grafo dirigido (DAG).

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Request:**

```json
{
  "name": "mi-grafo",
  "nodes": [
    {
      "id": "nodo-1",
      "tool_type": "llm_call",
      "tool_config": {
        "provider": "ollama",
        "model": "llama2"
      }
    },
    {
      "id": "nodo-2",
      "tool_type": "write_file",
      "tool_config": {
        "file_path": "/tmp/output.txt"
      }
    }
  ],
  "edges": [
    {
      "from": "nodo-1",
      "to": "nodo-2"
    }
  ],
  "metadata": {
    "author": "gabo",
    "tags": ["demo"]
  }
}
```

**Response:** `201 Created`

```json
{
  "id": "graph-abc123",
  "name": "mi-grafo",
  "version": "1.0.0",
  "nodes": [...],
  "edges": [...],
  "metadata": {...}
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 400 | Node o Edge inválido | `{"error": "invalid node at index 0: ..."}` |
| 400 | JSON malformado | Error del parser JSON |

---

### GET /api/v1/graphs {#GET-graphs}

**Descripción:** Listar todos los grafos.

**Auth:** Requiere `X-API-Key`.

**Response:** Array de grafos.

```json
[
  {
    "id": "graph-abc123",
    "name": "mi-grafo",
    "version": "1.0.0",
    "nodes": [...],
    "edges": [...],
    "metadata": {...}
  }
]
```

---

### GET /api/v1/graphs/{id} {#GET-graphs-id}

**Descripción:** Obtener un grafo por ID.

**Auth:** Requiere `X-API-Key`.

**Path Parameters:**
| Parámetro | Tipo | Obligatorio | Descripción |
|---|---|---|---|
| id | string | Sí | ID único del grafo |

**Response:** Objeto grafo.

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Grafo no existe | `{"error": "Graph not found"}` |

---

### DELETE /api/v1/graphs/{id} {#DELETE-graphs-id}

**Descripción:** Eliminar un grafo.

**Auth:** Requiere `X-API-Key`.

**Response:** `204 No Content`

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Grafo no existe | `{"error": "Graph not found"}` |

---

## Agents CRUD & Execution

### POST /api/v1/agents {#POST-agents}

**Descripción:** Crear un agente que enlaza a un grafo existente.

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Request:**

```json
{
  "name": "mi-agente",
  "description": "Agente de análisis",
  "graph_id": "graph-abc123",
  "triggers": []
}
```

**Response:** `201 Created`

```json
{
  "id": "agent-xyz789",
  "name": "mi-agente",
  "graph_id": "graph-abc123",
  "status": "created"
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Graph no existe | `{"error": "Graph 'graph-abc123' not found"}` |

---

### GET /api/v1/agents {#GET-agents}

**Descripción:** Listar todos los agentes.

**Auth:** Requiere `X-API-Key`.

**Response:** Array de agentes.

```json
[
  {
    "id": "agent-xyz789",
    "name": "mi-agente",
    "description": "Agente de análisis",
    "graph_id": "graph-abc123"
  }
]
```

---

### GET /api/v1/agents/{id} {#GET-agents-id}

**Descripción:** Obtener un agente por ID.

**Auth:** Requiere `X-API-Key`.

**Path Parameters:**
| Parámetro | Tipo | Obligatorio |
|---|---|---|
| id | string | Sí |

**Response:** Objeto agente.

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |

---

### POST /api/v1/agents/from-spec {#POST-agents-from-spec}

**Descripción:** Crear un agente directamente desde una especificación YAML completa (sin grafo separado).

**Auth:** Requiere `X-API-Key`.

**Request:** [→ SCHEMA.md#AgentSpec](../database/SCHEMA.md#entidad-agents)

```json
{
  "name": "agente-directo",
  "description": "...",
  "version": "v1",
  "agent_type": "workflow",
  "graph": {...},
  "inputs": {...},
  "outputs": {...}
}
```

**Response:** `201 Created`

```json
{
  "id": "agent-direct-123",
  "name": "agente-directo",
  "status": "created"
}
```

---

### POST /api/v1/agents/{id}/execute {#POST-agents-id-execute}

**Descripción:** Ejecutar un agente de forma síncrona. Bloquea hasta que completa o timeout (300s).

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#capabilities-engine](../producto/DOMINIO.md#capabilities-engine).

**Validaciones:** → [FLUJOS.md#flujo-run-agent](../producto/FLUJOS.md#flujo-run-agent).

**Request:**

```json
{
  "entry_node_id": "nodo-1",
  "trigger_data": {
    "user_input": "analiza esto",
    "temperature": 0.7
  }
}
```

**Response:** `200 OK`

```json
{
  "session_id": "session-abc123",
  "agent_id": "agent-xyz789",
  "agent_name": "mi-agente",
  "status": "Completed",
  "trace": [
    {
      "node_id": "nodo-1",
      "tool_type": "llm_call",
      "status": "Ok",
      "duration_ms": 1234,
      "retries": 0,
      "error": null
    }
  ],
  "transcript": [
    {
      "node": "nodo-1",
      "type": "llm_call",
      "input": {"prompt": "..."},
      "output": {"response": "..."}
    }
  ],
  "state": {
    "nodo-1": {
      "response": "...",
      "tokens_used": 150
    }
  },
  "error": null
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |
| 422 | Input validation falla | `{"error": "input validation failed: field_name required"}` |
| 422 | Es agente Live | `{"error": "live agents are controlled via play/stop, not execute"}` |
| 504 | Timeout de ejecución (300s) | `{"error": "execution timed out after 300s"}` |

**Guardias:**
- Agente debe existir
- Inputs validados contra spec.inputs si está definido
- Agente no debe ser tipo Live → [FLUJOS.md#regla-prd-008](../producto/FLUJOS.md)
- Timeout global de 300s → [FLUJOS.md#regla-07](../producto/FLUJOS.md#regla-07)

**Side-effects:**
- Sesión creada en memory_store
- Memory persistida si está configurada → [DOMINIO.md#Agent](../producto/DOMINIO.md#glosario)

---

### POST /api/v1/agents/{id}/stream {#POST-agents-id-stream}

**Descripción:** Ejecutar un agente con streaming en tiempo real vía Server-Sent Events (SSE).

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#capabilities-engine](../producto/DOMINIO.md#capabilities-engine).

**Request:** Mismo que `/execute` → {#POST-agents-id-execute}

**Response:** `200 OK` con `Content-Type: text/event-stream`

Eventos SSE emitidos en tiempo real:

```
event: node_start
data: {"node_id": "nodo-1", "tool_type": "llm_call"}

event: node_progress
data: {"node_id": "nodo-1", "progress": 0.5}

event: node_end
data: {"node_id": "nodo-1", "status": "Ok", "duration_ms": 1234, "output": {...}}

event: graph_end
data: {"status": "Completed", "session_id": "...", "error": null}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |
| 422 | Input validation falla | `{"error": "input validation failed: ..."}` |

**Notas:**
- Los eventos se emiten **durante** la ejecución, no post-ejecución
- Cliente puede desconectar en cualquier momento (pero el agente sigue ejecutando)
- Útil para UIs que muestren progreso en tiempo real

---

### GET /api/v1/agents/{id}/spec {#GET-agents-id-spec}

**Descripción:** Obtener la especificación completa del agente.

**Auth:** Requiere `X-API-Key`.

**Response:** Objeto [AgentSpec](../database/SCHEMA.md#entidad-agents).

```json
{
  "name": "mi-agente",
  "description": "...",
  "version": "v1",
  "agent_type": "workflow",
  "system_prompt": null,
  "soul": null,
  "inputs": {...},
  "outputs": {...},
  "graph": {...},
  "schedule": null,
  "triggers": [],
  "config": {},
  "resources": [],
  "metadata": {}
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |

---

### GET /api/v1/agents/{id}/schema {#GET-agents-id-schema}

**Descripción:** Obtener el contrato de entrada/salida del agente (PRD-004 Capa 1).

**Auth:** Requiere `X-API-Key`.

**Response:**

```json
{
  "name": "mi-agente",
  "version": "v1",
  "description": "...",
  "inputs": {
    "type": "object",
    "properties": {
      "user_input": {"type": "string"},
      "temperature": {"type": "number"}
    },
    "required": ["user_input"]
  },
  "outputs": {
    "type": "object",
    "properties": {
      "analysis": {"type": "string"},
      "confidence": {"type": "number"}
    }
  }
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |

---

## Live Agent Lifecycle (PRD-008)

### POST /api/v1/agents/{id}/play {#POST-agents-id-play}

**Descripción:** Iniciar ciclos automáticos de un agente Live. Solo para agentes con `agent_type: Live` y un schedule configurado.

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Validaciones:**
- Agente debe existir
- Agente debe ser tipo Live
- Agente debe tener schedule configurado
- Agente no debe estar ya en play (en ciclo)

**Response:** `200 OK`

```json
{
  "agent_id": "agent-xyz789",
  "status": "playing",
  "schedule": {
    "interval_seconds": 60
  },
  "memory_keys": ["counter", "last_result"]
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Agente no existe | `{"error": "Agent not found"}` |
| 422 | No es Live | `{"error": "only live agents support play/stop"}` |
| 422 | Sin schedule | `{"error": "live agent has no schedule configured"}` |
| 409 | Ya está en play | `{"error": "agent is already playing"}` |

**Side-effects:**
- Scheduler inicia ciclos cada `interval_seconds`
- Cada ciclo ejecuta el grafo con `trigger_data.cycle_number` y `trigger_data.triggered_by: "scheduler"`
- Memory es persistida entre ciclos

---

### POST /api/v1/agents/{id}/stop {#POST-agents-id-stop}

**Descripción:** Detener los ciclos automáticos de un agente Live.

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Response:** `200 OK`

```json
{
  "agent_id": "agent-xyz789",
  "status": "enabled",
  "cycles_completed": 42
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 409 | No está en play | `{"error": "agent is not playing"}` |

---

### GET /api/v1/agents/{id}/cycles {#GET-agents-id-cycles}

**Descripción:** Obtener historial de ciclos ejecutados.

**Auth:** Requiere `X-API-Key`.

**Query Parameters:**
| Parámetro | Tipo | Default | Descripción |
|---|---|---|---|
| limit | integer | 50 | Número máximo de ciclos a retornar |

**Response:**

```json
{
  "agent_id": "agent-xyz789",
  "total_cycles": 42,
  "cycles": [
    {
      "cycle_number": 42,
      "timestamp": "2025-05-31T10:30:00Z",
      "status": "Completed",
      "duration_ms": 1234,
      "error": null
    }
  ]
}
```

---

### GET /api/v1/agents/{id}/memory {#GET-agents-id-memory}

**Descripción:** Obtener el estado actual de memory del agente (persistente entre ciclos).

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Response:**

```json
{
  "agent_id": "agent-xyz789",
  "memory": {
    "counter": 42,
    "last_result": "completed at 10:30",
    "accumulated_tokens": 15000
  }
}
```

---

### DELETE /api/v1/agents/{id}/memory {#DELETE-agents-id-memory}

**Descripción:** Limpiar la memory de un agente, reseteándola a valores iniciales.

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#rol-product-engineer](../producto/DOMINIO.md#rol-product-engineer).

**Validaciones:**
- Agente no debe estar en play (cycling activo)

**Response:** `200 OK`

```json
{
  "agent_id": "agent-xyz789",
  "memory": {
    "counter": 0,
    "last_result": null
  },
  "reset_to": "initial_values"
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 409 | Agente en play | `{"error": "cannot clear memory while agent is playing"}` |

---

## Tools

### GET /api/v1/tools {#GET-tools}

**Descripción:** Listar todas las herramientas registradas con sus esquemas.

**Auth:** Requiere `X-API-Key`.

**Response:** Array de tools.

```json
[
  {
    "tool_type": "llm_call",
    "name": "LLM Call",
    "description": "Ejecutar un LLM remoto",
    "category": "llm",
    "inputs": [
      {
        "name": "provider",
        "type": "String",
        "required": true,
        "description": "LLM provider (ollama, claude, openai, etc)"
      },
      {
        "name": "model",
        "type": "String",
        "required": true,
        "description": "Model name"
      },
      {
        "name": "prompt",
        "type": "String",
        "required": true,
        "description": "Prompt text"
      }
    ],
    "outputs": [
      {
        "name": "response",
        "type": "String",
        "description": "LLM response text"
      },
      {
        "name": "tokens_used",
        "type": "Integer",
        "description": "Tokens consumed"
      }
    ]
  }
]
```

**Notas:**
- 50+ herramientas builtin disponibles
- Incluye categories: llm, files, network, math, agent, system, etc
- Cada tool tiene inputs/outputs tipados y requeridos

---

## Templates

### GET /api/v1/templates {#GET-templates}

**Descripción:** Listar plantillas de agentes predefinidas.

**Auth:** Requiere `X-API-Key`.

**Response:**

```json
[
  {
    "id": "template-001",
    "name": "Análisis de Documentos",
    "category": "analysis",
    "description": "Template para análisis de PDFs y documentos",
    "required_providers": ["claude", "openai"],
    "tags": ["pdf", "analysis", "nlp"]
  }
]
```

---

## Sessions

### GET /api/v1/sessions {#GET-sessions}

**Descripción:** Listar todas las sesiones de ejecución (en memoria).

**Auth:** Requiere `X-API-Key`.

**Query Parameters:**
| Parámetro | Tipo | Default | Descripción |
|---|---|---|---|
| agent_id | string | — | Filtrar por agent_id (opcional) |
| limit | integer | 50 | Número máximo de sesiones a retornar |

**Response:**

```json
[
  {
    "id": "session-abc123",
    "status": "Completed",
    "trace_len": 3,
    "error": null
  }
]
```

**Notas:**
- Sesiones se eviccionan FIFO cuando se alcanza 10K → [FLUJOS.md#regla-06](../producto/FLUJOS.md#regla-06)
- Solo almacenadas en memoria durante la sesión del servidor

---

### GET /api/v1/sessions/{id} {#GET-sessions-id}

**Descripción:** Obtener los detalles completos de una sesión.

**Auth:** Requiere `X-API-Key`.

**Path Parameters:**
| Parámetro | Tipo | Obligatorio |
|---|---|---|
| id | string | Sí |

**Response:**

```json
{
  "id": "session-abc123",
  "status": "Completed",
  "trace": [
    {
      "node_id": "nodo-1",
      "tool_type": "llm_call",
      "status": "Ok",
      "duration_ms": 1234,
      "retries": 0,
      "error": null
    }
  ],
  "error": null
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 404 | Sesión no existe | `{"error": "Session not found"}` |

---

## Execution Event Bus (PRD-021-E)

### GET /api/v1/events {#GET-events}

**Descripción:** SSE con los `ExecutionEvent` que emite el runner **mientras**
corre cualquier grafo del proceso. Es el bus de observabilidad: un consumidor se
engancha **una vez** y ve TODOS los runs, no uno.

**Auth:** `X-API-Key`, o `?api_key=` en el query (EventSource no pone cabeceras).

**Query params (opcionales):**

| Param | Efecto |
|---|---|
| `session_id` | Solo los eventos de ESE run |
| `types` | Lista separada por comas de `event_type` (ej. `checkpoint_created,session_failed`) |

**Response:** `200 OK` con `Content-Type: text/event-stream`. Primer frame
`: connected` (comentario SSE) apenas se abre; luego un frame por evento con el
**envelope completo**:

```
: connected

event: session_started
data: {"event_type":"session_started","timestamp":1754500000.12,"session_id":"ab12cd","data":{},"event_id":0}

event: checkpoint_created
data: {"event_type":"checkpoint_created","timestamp":1754500000.34,"session_id":"ab12cd","node_id":"ask","data":{"checkpoint_id":"...","step":1},"event_id":3}

event: interrupt_created
data: {"event_type":"interrupt_created","timestamp":1754500000.35,"session_id":"ab12cd","node_id":"ask","data":{},"event_id":4}
```

**Notas:**
- `session_id` correlaciona con `GET /api/v1/sessions/{id}` y con el
  `session_id` que devuelve `POST /execute` / el `run.started` de `/stream`.
- `event_id` es monótono: permite detectar huecos.
- Bus en vivo, **no histórico**: entrega desde el instante de la suscripción.
- Suscriptor lento → se emite `: lagged <n>` y se sigue (nunca bloquea al runner).

**No confundir con `POST /api/v1/agents/{id}/stream`** → {#POST-agents-id-stream}:
ese es un stream **por run**, single-consumer, con otro sobre
(`{"event": …, "data": …}`) y otros nombres de evento (`node.started`,
`graph.completed`). Son dos flujos distintos a propósito; este endpoint es
**aditivo** y no cambia nada de aquel.

---

## Metrics

### GET /api/v1/metrics {#GET-metrics}

**Descripción:** Obtener métricas agregadas de todas las sesiones ejecutadas.

**Auth:** Requiere `X-API-Key`.

**Response:**

```json
{
  "sessions": {
    "total": 100,
    "completed": 95,
    "failed": 5
  },
  "nodes": {
    "total_executed": 450,
    "total_duration_ms": 125000,
    "avg_duration_ms": 277.78
  },
  "tools_registered": 50,
  "agents_loaded": 8
}
```

---

## RAG & Evaluation

### POST /api/v1/rag/search {#POST-rag-search}

**Descripción:** Ejecutar búsqueda RAG sobre documentos con embeddings e indexación semántica.

**Auth:** Requiere `X-API-Key`.

**Request:**

```json
{
  "query": "cómo configurar el servidor",
  "documents": [
    "Contenido del documento 1",
    "Contenido del documento 2"
  ],
  "top_k": 5,
  "chunk_strategy": "paragraph",
  "chunk_size": 512
}
```

**Request Parameters:**
| Parámetro | Tipo | Default | Obligatorio | Descripción |
|---|---|---|---|---|
| query | string | — | Sí | Texto de búsqueda |
| documents | array[string] | — | Sí | Array de documentos/textos a indexar |
| top_k | integer | 3 | No | Número máximo de chunks a retornar |
| chunk_strategy | string | "paragraph" | No | Estrategia de chunking: "paragraph", "sentence", "fixed_size" |
| chunk_size | integer | 512 | No | Tamaño de chunk para "fixed_size" |

**Response:**

```json
{
  "query": "cómo configurar el servidor",
  "results": [
    {
      "chunk": "contenido relevante del documento...",
      "score": 0.95,
      "index": 0
    }
  ],
  "chunks_total": 15,
  "embedding_model": "nomic-embed-text",
  "dimensions": 768
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 400 | query vacío | `{"error": "query is required"}` |
| 400 | documents falta o vacío | `{"error": "documents array is required"}` |
| 500 | Falla en embeddings | `{"error": "Failed to embed query: ..."}` |

**Notas:**
- Utiliza embeddings REAL via Ollama (modelo: nomic-embed-text)
- Realiza chunking automático según la estrategia
- Retorna scores como similitud de coseno normalizada (0.0-1.0)

---

### POST /api/v1/eval {#POST-eval}

**Descripción:** Ejecutar evaluación de calidad en salida de agente usando un juez LLM real.

**Auth:** Requiere `X-API-Key`.

**Request:**

```json
{
  "input": "analiza esto",
  "output": "la salida del agente",
  "context": "información adicional",
  "duration_ms": 1234,
  "judge_model": "claude-3-sonnet",
  "eval_types": ["relevance", "faithfulness", "completeness", "format_compliance", "latency"]
}
```

**Response:**

```json
{
  "results": [
    {
      "eval_type": "relevance",
      "score": 0.92,
      "details": "Output is highly relevant to the input query",
      "judge_model": "claude-3-sonnet"
    },
    {
      "eval_type": "latency",
      "score": 0.78,
      "details": "Execution took 1234ms, acceptable for async use",
      "judge_model": "claude-3-sonnet"
    }
  ],
  "eval_count": 5
}
```

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 400 | eval_types vacío | `{"error": "eval_types array required (relevance, faithfulness, completeness, format_compliance, latency)"}` |

---

## Universe (Multi-Agent Router)

### POST /api/v1/universe/message {#POST-universe-message}

**Descripción:** Enviar un mensaje a un Universe. El sistema routea automáticamente al agente más apropiado según la estrategia configurada.

**Auth:** Requiere `X-API-Key`.

**Autorización:** → [DOMINIO.md#Universe](../producto/DOMINIO.md#glosario).

**Request:**

```json
{
  "message": "necesito ayuda con Python",
  "name": "mi-universo",
  "strategy": "llm_classify",
  "agents": [
    {
      "name": "Asistente Python",
      "agent_id": "agent-python",
      "capabilities": ["coding", "debugging", "documentation"]
    },
    {
      "name": "Asistente SQL",
      "agent_id": "agent-sql",
      "capabilities": ["databases", "queries", "optimization"]
    }
  ]
}
```

**Response:**

```json
{
  "routing": {
    "agent_name": "Asistente Python",
    "agent_id": "agent-python",
    "confidence": 0.95,
    "strategy_used": "llm_classify",
    "reason": "Mensaje relacionado con Python"
  },
  "executed": true,
  "result": {
    "status": "Completed",
    "state": {...},
    "error": null
  }
}
```

**Validaciones:**
- `message` es obligatorio
- `agents` array debe tener al menos 1 agente

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 400 | message falta | `{"error": "message is required"}` |
| 400 | agents falta o vacío | `{"error": "agents array is required"}` |

---

### POST /api/v1/universe/groupchat {#POST-universe-groupchat}

**Descripción:** Ejecutar un debate entre múltiples agentes sobre un tema.

**Auth:** Requiere `X-API-Key`.

**Request:**

```json
{
  "topic": "¿Es mejor monolitico o microservicios?",
  "max_rounds": 3,
  "participants": [
    {
      "name": "DevOps Engineer",
      "personality": "Pragmático, enfocado en escalabilidad"
    },
    {
      "name": "Backend Architect",
      "personality": "Perfectcionista, valora la arquitectura limpia"
    }
  ]
}
```

**Response:**

```json
{
  "topic": "¿Es mejor monolitico o microservicios?",
  "rounds": 3,
  "participants": 2,
  "transcript": [
    {
      "round": 1,
      "agent": "DevOps Engineer",
      "content": "Los microservicios dan mejor escalabilidad..."
    },
    {
      "round": 2,
      "agent": "Backend Architect",
      "content": "Pero la complejidad aumenta significativamente..."
    }
  ]
}
```

**Validaciones:**
- `topic` es obligatorio
- `participants` debe tener al menos 2 elementos

**Errores:**

| Código | Condición | Mensaje |
|---|---|---|
| 400 | topic falta | `{"error": "topic is required"}` |
| 400 | participants < 2 | `{"error": "participants array required (min 2 agents with name + personality)"}` |

---

## Webhooks

### POST /webhooks/{*path} {#POST-webhooks-path}

**Descripción:** Endpoint dinámico para recibir webhooks desde sistemas externos. Se routea automáticamente a agentes con triggers configurados.

**Auth:** Público (no requiere API key) — pero debería estar protegido en producción.

**Path Parameters:**
| Parámetro | Tipo | Obligatorio | Descripción |
|---|---|---|---|
| path | string | Sí | Ruta dinámica (p.ej., `github/push`) |

**Request:** Cualquier JSON.

**Response:**

```json
{
  "received": true,
  "path": "github/push",
  "body": {...}
}
```

**Notas:**
- Placeholder en la implementación actual — el full routing por triggers está en desarrollo
- En producción, proteger con secret en query params o headers

---

## Error Handling

Todos los errores son JSON con este formato:

```json
{
  "error": "descripción del error"
}
```

**HTTP Status Codes:**

| Código | Significado |
|---|---|
| 200 | OK |
| 201 | Created |
| 204 | No Content |
| 400 | Bad Request (validación fallida) |
| 401 | Unauthorized (API key inválida o falta) |
| 404 | Not Found (recurso no existe) |
| 409 | Conflict (estado inconsistente, p.ej., agent ya playing) |
| 422 | Unprocessable Entity (validación de inputs fallida) |
| 500 | Internal Server Error (bug del engine) |
| 504 | Gateway Timeout (execution timeout) |

---

## Ejemplos Completos

### Flujo: Crear grafo → Crear agente → Ejecutar

```bash
# 1. Crear grafo
curl -X POST http://localhost:3000/api/v1/graphs \
  -H "X-API-Key: tu-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "análisis",
    "nodes": [
      {"id": "step1", "tool_type": "llm_call", "tool_config": {"provider": "ollama", "model": "llama2"}},
      {"id": "step2", "tool_type": "write_file", "tool_config": {"file_path": "/tmp/result.txt"}}
    ],
    "edges": [{"from": "step1", "to": "step2"}],
    "metadata": {}
  }'

# Respuesta: {"id": "graph-abc123", ...}

# 2. Crear agente
curl -X POST http://localhost:3000/api/v1/agents \
  -H "X-API-Key: tu-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "mi-agente",
    "graph_id": "graph-abc123"
  }'

# Respuesta: {"id": "agent-xyz789", ...}

# 3. Ejecutar
curl -X POST http://localhost:3000/api/v1/agents/agent-xyz789/execute \
  -H "X-API-Key: tu-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "trigger_data": {"user_input": "analiza esto"}
  }'

# Respuesta: {"session_id": "...", "status": "Completed", ...}
```

### Streaming en tiempo real

```bash
curl -X POST http://localhost:3000/api/v1/agents/agent-xyz789/stream \
  -H "X-API-Key: tu-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "trigger_data": {"user_input": "analiza esto"}
  }' \
  | grep -E "^event:|^data:" # SSE output
```

---

## Timeout & Rate Limiting

- **Request timeout:** 300 segundos (configurable vía `DEFAULT_TIMEOUT_SECS`) → [FLUJOS.md#regla-07](../producto/FLUJOS.md#regla-07)
- **Rate limiting:** No implementado aún (futura mejora)
- **Session eviction:** FIFO cuando alcanza 10K → [FLUJOS.md#regla-06](../producto/FLUJOS.md#regla-06)

---

## Versioning

API versión actual: **v1**. Todos los endpoints bajo `/api/v1/`.

Health & version endpoints sin prefijo de versión (public stable).

---

## See Also

- [DOMINIO.md](../producto/DOMINIO.md) — Términos y roles
- [FLUJOS.md](../producto/FLUJOS.md) — Reglas de negocio
- [SCHEMA.md](../database/SCHEMA.md) — Modelos de datos (JSON Schema)
- [PRIMITIVES.md](PRIMITIVES.md) — Primitivas y builtin tools
