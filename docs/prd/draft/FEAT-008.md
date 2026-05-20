# FEAT-008 — Token-level Streaming

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-071

---

## Problem Statement

**Tipo**: Feature nueva (4 capacidades de streaming)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine ejecuta grafos completos y retorna resultados al final. La experiencia de ejecucion es una caja negra: el usuario dispara un agente, ve un spinner, y despues de segundos o minutos recibe el resultado completo. Esto presenta tres problemas concretos:

1. **Latencia percibida**: Un nodo LLM que genera 500 tokens tarda 5-15 segundos. El usuario no ve nada hasta que termina. En chatbots y asistentes esto es inaceptable — el estandar de la industria es typewriter effect token por token (ChatGPT, Claude, Gemini todos hacen streaming).

2. **Sin visibilidad de progreso**: En grafos con multiples nodos, el usuario no sabe cual nodo esta ejecutandose, cual ya termino, ni donde esta el progreso real. Si un grafo tiene 8 nodos y el nodo 3 falla despues de 20 segundos, el usuario descubre la falla recien cuando todo termina.

3. **Sub-grafos opacos**: Cuando un agente ejecuta un sub-agente via `agent/run_agent`, los tokens y progreso del hijo son completamente invisibles para el padre. Un grafo padre que orquesta 3 sub-agentes muestra una sola barra de progreso sin desglose.

Todos los frameworks competidores (LangGraph, CrewAI, AutoGen) implementan streaming. Es un requisito minimo para experiencia de usuario competitiva.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Ver tokens del LLM aparecer uno a uno en tiempo real mientras el nodo ejecuta (typewriter effect)
2. Ver el estado de cada nodo del grafo durante ejecucion (pending, running, completed, error) con actualizacion en vivo
3. Ver progreso desglosado de sub-agentes cuando un grafo padre los invoca
4. Tener una experiencia de sesion que se siente "viva" — no hay momentos de pantalla congelada sin feedback

---

## Features

### 8.1 — LLM Streaming: Tokens via AsyncIterator + SSE

**Problema**: Los LLM adapters (FEAT-002) tienen metodo `stream()` definido pero el runner no lo usa. El nodo `ai/llm_call` siempre hace `call()` sincrono — espera respuesta completa, la guarda en output, avanza al siguiente nodo. Los tokens intermedios se pierden. La UI no tiene canal para recibirlos.

**Solucion**: Modificar cada LLM adapter para implementar `stream()` como AsyncIterator real que emite `NormalizedChunk` por cada token. El runner propaga chunks via `EventEmitter` existente. El server envia chunks al frontend via SSE (Server-Sent Events). El frontend renderiza incrementalmente.

**Arquitectura**:
- Cada LLM adapter implementa `stream()` usando la API nativa de streaming del provider:
  - `OllamaAdapter.stream()` — HTTP streaming contra `/api/generate` con `stream: true`, emite tokens limpios (post-strip de `<think>`)
  - `ClaudeAdapter.stream()` — Anthropic SDK `messages.stream()`, emite content_block_delta events
  - `OpenAIAdapter.stream()` — OpenAI SDK con `stream=True`, emite choices[0].delta.content
  - `GeminiAdapter.stream()` — Google GenAI SDK `generate_content(stream=True)`, emite text chunks
  - `GroqAdapter.stream()` — API compatible OpenAI, mismo approach que OpenAI adapter
  - `OpenRouterAdapter.stream()` — API compatible OpenAI con header X-Title
- `NormalizedChunk`: `{ delta: str, done: bool, tokens_used: { input: int, output: int } | None, tool_calls: list | None }`
- El tool `ai/llm_call` detecta si streaming esta habilitado (config del nodo: `stream: true`, default true) y usa `adapter.stream()` en vez de `adapter.call()`
- Durante streaming, cada chunk se emite como evento `llm.token` via `EventEmitter`:
  - `{ event: "llm.token", node_id: str, session_id: str, delta: str, accumulated: str }`
- Al completar: evento `llm.complete`:
  - `{ event: "llm.complete", node_id: str, session_id: str, response: str, tokens_used: dict }`
- SSE endpoint existente (`GET /api/sessions/{id}/stream`) se extiende para incluir eventos `llm.token` y `llm.complete`
- Fallback: si `stream()` falla o el adapter no lo soporta, cae a `call()` sincrono y emite un solo evento `llm.complete` con la respuesta completa

**Entidades nuevas**: Ninguna. Los chunks son transientes (solo SSE, no se persisten). El resultado final se persiste como siempre en el execution_span.

**Contratos API**:
- `GET /api/sessions/{id}/stream` — SSE endpoint existente, se extiende con nuevos event types:
  - `event: llm.token` — `{ node_id, delta, accumulated }` — token individual
  - `event: llm.complete` — `{ node_id, response, tokens_used }` — respuesta completa del nodo

No se crean endpoints nuevos. Se extiende el SSE existente.

**Pantallas**:
- **Session Detail** (`/sessions/[id]`): el output de nodos LLM se renderiza incrementalmente. Tokens aparecen uno a uno con cursor parpadeante (typewriter). Al completar, el cursor desaparece y el texto queda fijo. Si el nodo ya completo (sesion historica), se muestra el resultado completo sin animacion.
- **Live Execution View**: si existe componente de ejecucion en vivo, los tokens de cada nodo LLM aparecen en su panel correspondiente.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-119 | Streaming nunca bloquea el runner. Si el SSE client se desconecta, los tokens se descartan pero la ejecucion continua | EventEmitter fire-and-forget |
| REGLA-120 | El resultado final (post-streaming) es identico al que retornaria call() sincrono. Streaming es solo presentacion, no cambia logica | ai/llm_call acumula chunks y guarda resultado completo en output |
| REGLA-121 | OllamaAdapter.stream() SIEMPRE strip de `<think>` tags de cada chunk, no solo del resultado final. Chunks parciales dentro de think tags se suprimen | OllamaAdapter._clean_stream_chunk |
| REGLA-122 | Si adapter.stream() lanza excepcion, el tool cae a adapter.call() sincrono. Warning en log, no error. El usuario ve el resultado completo de golpe en vez de streaming | ai/llm_call try/except fallback |

**Archivos a crear/modificar**:
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/ollama.py` — implementar stream() real con HTTP streaming + think tag filtering por chunk
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/claude.py` — implementar stream() con Anthropic SDK messages.stream()
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/openai_adapter.py` — implementar stream() con stream=True
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/gemini.py` — implementar stream() con generate_content(stream=True)
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/groq.py` — implementar stream() compatible OpenAI
- MODIFICAR `framework/src/datamirai_engine/llm/adapters/openrouter.py` — implementar stream() compatible OpenAI
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/ai/llm_call.py` — usar stream() cuando config.stream=true, emitir eventos llm.token/llm.complete via context.events
- MODIFICAR `app/server/datamirai_app/routes/heartbeat.py` — extender SSE con eventos llm.token y llm.complete (o el endpoint equivalente de streaming de sesiones)
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — renderizado incremental de tokens con typewriter effect
- CREAR `app/web/src/components/live/StreamingText.tsx` — componente reutilizable de texto streaming con cursor parpadeante

---

### 8.2 — Graph Streaming: Eventos por Nodo en Tiempo Real

**Problema**: Durante la ejecucion de un grafo, el usuario no sabe que nodo esta corriendo, cuales ya terminaron, ni donde esta el progreso. La UI muestra un estado binario: "ejecutando" o "completado". En grafos con 5-15 nodos donde cada uno tarda segundos, esto es minutos de caja negra.

**Solucion**: El runner emite eventos tipados por cada transicion de estado de cada nodo: start, progress, complete, error. El frontend recibe estos eventos via SSE y actualiza la UI en tiempo real — nodos cambian de color, muestran duracion, outputs parciales.

**Arquitectura**:
- `GraphEventType` enum: `node.start`, `node.progress`, `node.complete`, `node.error`, `graph.start`, `graph.complete`, `graph.error`
- Eventos emitidos por el runner via `EventEmitter`:
  - `node.start`: `{ node_id, node_type, step_number, timestamp }`
  - `node.progress`: `{ node_id, progress_pct?: float, message?: str }` — para nodos que reportan progreso (loops, parallel)
  - `node.complete`: `{ node_id, output_preview: str, duration_ms: int, tokens_used?: dict }`
  - `node.error`: `{ node_id, error_type, error_message, duration_ms: int }`
  - `graph.start`: `{ session_id, graph_id, total_nodes: int }`
  - `graph.complete`: `{ session_id, duration_ms: int, nodes_executed: int }`
  - `graph.error`: `{ session_id, failed_node_id, error_message }`
- `output_preview`: primeros 200 chars del output del nodo (truncado con "..."). No incluir datos sensibles.
- El runner ya tiene hooks internos por nodo — se agregan las emisiones de eventos en los puntos correctos del ciclo de ejecucion.

**Entidades nuevas**: Ninguna. Los eventos son transientes (SSE). Los datos permanentes ya se persisten en execution_spans.

**Contratos API**:
- `GET /api/sessions/{id}/stream` — SSE endpoint existente, se extiende con:
  - `event: node.start` — nodo empieza a ejecutar
  - `event: node.progress` — progreso parcial del nodo
  - `event: node.complete` — nodo completo con preview del output
  - `event: node.error` — nodo fallo
  - `event: graph.start` — grafo empieza
  - `event: graph.complete` — grafo completo
  - `event: graph.error` — grafo fallo

**Pantallas**:
- **Session Detail → Graph View**: nodos del grafo cambian de color segun estado:
  - Gris: pending (no ejecutado aun)
  - Azul pulsante: running (ejecutandose ahora)
  - Verde: completed (exito)
  - Rojo: error (fallo)
  - Cada nodo muestra duracion (0.3s, 2.1s) al completar
  - Nodo activo muestra output preview debajo
- **Session Detail → Timeline**: barra de progreso por nodo con timestamps y duraciones

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-123 | Eventos de grafo NUNCA ralentizan la ejecucion. Emision es fire-and-forget async. Si no hay subscribers, no hay overhead | EventEmitter async |
| REGLA-124 | output_preview NUNCA contiene datos de credenciales, API keys, o tokens. Se trunca a 200 chars y se filtra contra patrones sensibles | Event serializer con filter |
| REGLA-125 | Cada nodo emite exactamente un node.start y exactamente un node.complete o node.error. Nunca ambos. El frontend puede confiar en este contrato | Runner lifecycle hooks |
| REGLA-126 | graph.complete se emite SIEMPRE, incluso si un nodo fallo (en cuyo caso tambien se emitio graph.error antes). El frontend usa graph.complete como senal de "ya termino" | Runner finally block |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/core/graph_events.py` — GraphEventType enum + event dataclasses + GraphEventEmitter wrapper
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — emitir eventos node.start/complete/error/progress y graph.start/complete/error en los puntos correctos del ciclo
- MODIFICAR `app/server/datamirai_app/routes/heartbeat.py` — propagar graph events al SSE
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — estado visual por nodo (colores, duracion, preview)

---

### 8.3 — Sub-graph Streaming: Propagacion de Tokens Padre-Hijo

**Problema**: Cuando un grafo ejecuta `agent/run_agent` (sub-agente), los tokens y eventos del sub-agente son invisibles para el padre. El nodo run_agent aparece como "ejecutando" por minutos sin detalle. El usuario no sabe si el sub-agente esta en el nodo 2 de 5, si esta generando tokens del LLM, o si se colgo.

**Solucion**: Propagar eventos SSE del sub-agente al padre con namespacing. Los tokens y graph events del hijo se emiten en el stream del padre con prefijo que identifica al sub-agente. El frontend puede mostrar desglose del sub-agente dentro del nodo run_agent del padre.

**Arquitectura**:
- Cuando `agent/run_agent` ejecuta un sub-agente, el runner hijo emite eventos normalmente via EventEmitter
- El runner padre subscribe a los eventos del hijo y los re-emite con contexto adicional:
  - `{ ...child_event, parent_node_id: str, parent_session_id: str, sub_agent_name: str, depth: int }`
- `depth` indica nivel de anidamiento (0 = root, 1 = primer sub-agente, 2 = sub-sub-agente). Max depth: 5 (ya existente como limite de sub-grafos).
- Eventos del hijo se propagan al SSE del padre con type prefijado:
  - `sub.llm.token` — token de LLM del sub-agente
  - `sub.node.start` — nodo del sub-agente empezo
  - `sub.node.complete` — nodo del sub-agente completo
  - etc.

**Entidades nuevas**: Ninguna.

**Contratos API**:
- `GET /api/sessions/{id}/stream` — SSE existente, se extiende con eventos prefijados `sub.*`:
  - `event: sub.llm.token` — token de LLM de sub-agente: `{ parent_node_id, sub_agent_name, depth, delta, accumulated }`
  - `event: sub.node.start` — nodo de sub-agente empezo: `{ parent_node_id, sub_agent_name, depth, node_id, node_type }`
  - `event: sub.node.complete` — nodo de sub-agente completo: `{ parent_node_id, sub_agent_name, depth, node_id, output_preview, duration_ms }`
  - `event: sub.graph.complete` — sub-agente completo: `{ parent_node_id, sub_agent_name, depth, duration_ms }`

**Pantallas**:
- **Session Detail → nodo run_agent expandido**: al hacer click en un nodo run_agent, se expande mostrando:
  - Mini grafo del sub-agente con mismos colores de estado (8.2)
  - Tokens del LLM del sub-agente en streaming (8.1)
  - Indicador de depth si hay multiples niveles de anidamiento

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-127 | Propagacion de eventos sub-agente a padre es fire-and-forget. Si el padre no esta escuchando (no hay SSE client), los eventos se descartan | EventEmitter check subscribers |
| REGLA-128 | Max depth de propagacion = 5 (misma restriccion que sub-grafos). Eventos de profundidad > 5 se descartan | Depth check en propagacion |
| REGLA-129 | El volumen de eventos SSE de sub-agentes se throttlea a max 50 eventos/segundo por session. Si se excede, se descartan eventos de progreso (no start/complete/error) | SSE throttle middleware |

**Archivos a crear/modificar**:
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/agent/run_agent.py` — subscribe a eventos del runner hijo y re-emitir con contexto padre
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — soporte para event propagation con depth tracking
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — renderizado de sub-agente expandible dentro de nodo run_agent

---

### 8.4 — UI Streaming: Componente de Chat/Output con Renderizado Incremental

**Problema**: El frontend no tiene componentes preparados para renderizar texto incremental. Los outputs se muestran completos de golpe cuando el nodo termina. No hay typewriter effect, no hay cursor, no hay transiciones suaves. La experiencia es estatica y abrupta.

**Solucion**: Crear componentes React reutilizables para renderizado incremental de texto streaming. Typewriter effect con cursor parpadeante para LLM outputs. Integracion con el SSE client existente.

**Arquitectura**:
- `StreamingText` component:
  - Props: `sessionId`, `nodeId`, `fallbackText` (para sesiones historicas)
  - Se suscribe a eventos `llm.token` del SSE filtrados por nodeId
  - Renderiza texto acumulado con cursor parpadeante al final
  - Al recibir `llm.complete`, remueve cursor y muestra texto final
  - Si la sesion ya completo (historica), muestra `fallbackText` sin animacion
- `useStreamingSSE` hook:
  - `useStreamingSSE(sessionId) -> { events, isConnected, subscribe, unsubscribe }`
  - Gestiona conexion SSE, parseo de eventos, buffer de reconexion
  - Expone reactive state de eventos por tipo y nodeId
- `NodeStatusIndicator` component:
  - Props: `status` (pending/running/completed/error), `durationMs`
  - Renderiza badge de estado con color y animacion (pulsante para running)
- `ExecutionTimeline` component:
  - Props: `sessionId`, `nodes`
  - Timeline horizontal con nodos como puntos, coloreados por estado
  - Progreso animado del nodo activo

**Entidades nuevas**: Ninguna (componentes UI, sin persistencia).

**Contratos API**: Ninguno nuevo. Consume SSE existente.

**Pantallas**:
- Todos los componentes se integran en **Session Detail** (`/sessions/[id]`):
  - `StreamingText` reemplaza el render estatico de outputs de nodos LLM
  - `NodeStatusIndicator` se agrega a cada nodo en la vista de grafo
  - `ExecutionTimeline` se agrega como componente inferior de la pagina
- **Responsive**: componentes funcionan en mobile (texto streaming sin layout shifts)

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-130 | StreamingText no causa layout shifts. El contenedor tiene min-height reservado. Tokens nuevos se agregan sin mover contenido existente | CSS contain + scroll anchoring |
| REGLA-131 | Si SSE se desconecta durante streaming, el componente muestra ultimo texto acumulado + indicador "reconectando...". Al reconectar, fetcha resultado completo via REST | useStreamingSSE reconnect logic |
| REGLA-132 | Sesiones historicas (ya completadas) renderizan outputs completos sin animacion. No hay typewriter en replay. StreamingText detecta estado de session al montar | Props fallbackText + session status check |

**Archivos a crear/modificar**:
- CREAR `app/web/src/components/live/StreamingText.tsx` — componente de texto streaming con typewriter
- CREAR `app/web/src/components/live/NodeStatusIndicator.tsx` — badge de estado de nodo animado
- CREAR `app/web/src/components/live/ExecutionTimeline.tsx` — timeline de ejecucion del grafo
- CREAR `app/web/src/components/live/useStreamingSSE.ts` — hook para consumir SSE con estado reactivo
- MODIFICAR `app/web/src/app/sessions/[id]/page.tsx` — integrar StreamingText, NodeStatusIndicator, ExecutionTimeline
- MODIFICAR `app/web/src/app/globals.css` — estilos para cursor parpadeante, pulsante, timeline

---

## Dependencias entre features

```
8.1 (LLM Streaming) <- independiente, se implementa primero
8.2 (Graph Streaming) <- independiente de 8.1, puede ser paralelo
8.3 (Sub-graph Streaming) <- depende de 8.1 + 8.2 (propaga ambos tipos de eventos)
8.4 (UI Streaming) <- depende de 8.1 + 8.2 (necesita eventos para renderizar)
```

Orden de implementacion: 8.1 + 8.2 (paralelo) → 8.3 + 8.4 (paralelo)

Dependencias externas:
- FEAT-002 — LLM adapters (8.1 modifica sus metodos stream())
- FEAT-004 — Sub-grafos (8.3 depende de agent/run_agent existente)
- EventEmitter existente en `framework/src/datamirai_engine/core/events.py`

---

## Entidades nuevas (resumen consolidado)

Ninguna tabla nueva. Todo el streaming es transiente (SSE events en memoria). Los datos permanentes (outputs, execution_spans) se persisten como siempre al completar cada nodo.

---

## Maquinas de estado

### Streaming Connection (frontend)

```
DISCONNECTED -> CONNECTING -> CONNECTED
CONNECTED -> DISCONNECTED (session complete)
CONNECTED -> RECONNECTING (connection lost)
RECONNECTING -> CONNECTED (reconnected)
RECONNECTING -> DISCONNECTED (max retries)
```

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-119 | Streaming nunca bloquea runner. SSE disconnect no afecta ejecucion | EventEmitter fire-and-forget |
| REGLA-120 | Resultado final identico con o sin streaming. Streaming es presentacion | ai/llm_call accumulate |
| REGLA-121 | Ollama stream() strip think tags por chunk, no solo al final | OllamaAdapter._clean_stream_chunk |
| REGLA-122 | stream() falla -> fallback a call() sincrono, warning no error | ai/llm_call try/except |
| REGLA-123 | Graph events fire-and-forget. Sin subscribers = sin overhead | EventEmitter async |
| REGLA-124 | output_preview sin datos sensibles, max 200 chars | Event serializer filter |
| REGLA-125 | Cada nodo: exactamente 1 start + 1 complete o 1 error | Runner lifecycle |
| REGLA-126 | graph.complete SIEMPRE se emite, incluso si hubo error previo | Runner finally |
| REGLA-127 | Sub-agent event propagation es fire-and-forget | EventEmitter check |
| REGLA-128 | Max depth propagacion = 5 | Depth check |
| REGLA-129 | Max 50 events/sec por session, throttle descarta progress | SSE throttle |
| REGLA-130 | Sin layout shifts en StreamingText | CSS contain |
| REGLA-131 | SSE disconnect: ultimo texto + reconectando + fetch REST | useStreamingSSE |
| REGLA-132 | Sesiones historicas sin typewriter | Session status check |

---

## Notas de implementacion

- **SSE vs WebSocket**: se mantiene SSE para streaming porque es unidireccional (server->client), funciona con HTTP/1.1 sin upgrades, y el server ya tiene infraestructura SSE. WebSocket (FEAT-007) es para comunicacion bidireccional (CRUD reactivo). Ambos coexisten: SSE para ejecucion, WebSocket para UI reactiva.
- **Backpressure**: si el frontend no consume tokens tan rapido como llegan (pestaña en background, red lenta), los eventos se bufferizan en el SSE connection hasta un max de 1000 eventos. Si el buffer se llena, se descartan eventos `llm.token` (los mas frecuentes). `node.complete` y `graph.complete` nunca se descartan.
- **Think tag streaming**: Para Ollama, el adapter mantiene un buffer interno durante streaming. Si detecta apertura de `<think>`, acumula chunks sin emitir hasta detectar `</think>`. Si el buffer excede 10KB sin cerrar, se descarta (think tag malformado). Esto evita emitir tokens parciales dentro de think blocks.
- **Token counting en streaming**: La mayoria de providers retornan token counts solo al final del stream (en el ultimo chunk). Los adapters acumulan y retornan counts completos en el evento `llm.complete`.
- **Backward compatibility**: si `stream: false` en config del nodo (o si el adapter no soporta streaming), todo funciona identico a antes. Un solo evento `llm.complete` con la respuesta completa.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-002.md` — multi-LLM adapters (8.1 modifica stream() de cada adapter)
- `docs/prd/draft/FEAT-004.md` — sub-grafos (8.3 propaga eventos de run_agent)
