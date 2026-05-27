# Engine v0.2.0 — Production-Ready

| Campo | Valor |
|-------|-------|
| **ID** | PRD-002 |
| **Fecha** | 2026-05-27 |
| **Estado** | backlog |
| **Branch** | prd/PRD-002 |
| **Target** | v0.2.0 |

---

## Diagrama General

```
┌─────────────────────────────────────────────────────────────────┐
│                     ENGINE v0.2.0 SCOPE                         │
│                                                                 │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────┐       │
│  │ W1: Server  │  │ W2: Streaming│  │ W3: Universe    │       │
│  │ Real LLM    │  │ Real-time    │  │ Multi-Agent     │       │
│  │ (fix mock)  │  │ SSE tokens   │  │ Execution       │       │
│  └──────┬──────┘  └──────┬───────┘  └────────┬────────┘       │
│         │                │                    │                 │
│  ┌──────┴──────┐  ┌──────┴───────┐  ┌────────┴────────┐       │
│  │ W4: Eval    │  │ W5: RAG      │  │ W6: Server      │       │
│  │ LLM Judge   │  │ Embeddings   │  │ Endpoints       │       │
│  │ + CLI       │  │ Reales       │  │ Especializados  │       │
│  └──────┬──────┘  └──────┬───────┘  └────────┬────────┘       │
│         │                │                    │                 │
│  ┌──────┴──────┐  ┌──────┴───────┐  ┌────────┴────────┐       │
│  │ W7: Channel │  │ W8: Voice    │  │ W9: Docs        │       │
│  │ Adapters    │  │ STT/TTS      │  │ + Cleanup       │       │
│  │ Reales      │  │ Real         │  │                 │       │
│  └─────────────┘  └──────────────┘  └─────────────────┘       │
└─────────────────────────────────────────────────────────────────┘

Dependencias:
  W1 ──▶ W3 (Universe necesita LLM real)
  W1 ──▶ W4 (Eval LLM-judge necesita LLM real)
  W1 ──▶ W5 (RAG embeddings necesita LLM real)
  W2 ──▶ W6 (endpoints de stream usan streaming real)
  W5 ──▶ W6 (endpoints RAG usan pipeline real)
```

---

## Problema

- **Tipo**: mejora (gap closure + production hardening)
- **Resumen**: Engine v0.1.0 tiene 11 de 23 features end-to-end funcionales. Las 12 restantes tienen código structural (types, routing, strategies) pero les falta integración con servicios reales. El server HTTP usa MockLLM en producción — viola la regla de ciudad conectada. SSE streaming no es real-time.
- **Actores**: Developer (consume CLI/SDK), Host App (consume HTTP API), Agent (ejecuta en engine)
- **Flujos tocados**: ejecución de agentes via server, multi-agent, evaluación, RAG, canales de mensajería, voice
- **Qué cambia**:
  - HOY: server responde con mocks, streaming es post-execution, Universe solo routea sin ejecutar, Eval sin LLM judge, RAG sin embeddings reales, Channels sin adapters reales, Voice sin STT/TTS real
  - DESPUÉS: todo real, zero mocks en producción, streaming token-by-token, Universe ejecuta agentes completos, stack completo end-to-end

---

## Actores y Permisos

| Actor | Capacidad | Acción | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | ejecutar_agente | Ejecutar agente via CLI o SDK | Estado de ejecución, trace, métricas |
| Developer | configurar_provider | Configurar LLM provider para server | Providers disponibles, status de conexión |
| Developer | gestionar_rag | Ingestar documentos y buscar en RAG | Colecciones, chunks, resultados de búsqueda |
| Developer | evaluar_sesion | Ejecutar evaluaciones sobre sesiones | Scores, detalles por evaluador |
| Host App | ejecutar_via_api | Ejecutar agentes via HTTP API con LLM real | Resultados, stream de eventos, traces |
| Host App | gestionar_universe | Enviar mensajes a Universe multi-agente | Routing decision, respuesta del agente |
| Host App | conectar_canal | Conectar canal de mensajería | Estado de conexión, mensajes entrantes/salientes |
| Agent | ejecutar_tools | Ejecutar tools del registry | Outputs de cada tool |
| Agent | acceder_memoria | Leer/escribir memoria persistente | Datos en memoria |
| Agent | buscar_rag | Buscar en índice RAG | Chunks relevantes |

**Capacidades nuevas**:
- `configurar_provider`: configurar LLM provider en el server (no existía — server era mock-only)
- `gestionar_rag`: crear colecciones, ingestar documentos, buscar
- `evaluar_sesion`: ejecutar evaluaciones programáticas y LLM-as-judge
- `gestionar_universe`: enviar mensajes a router multi-agente
- `conectar_canal`: establecer conexión con plataforma de mensajería
- `procesar_audio`: transcribir audio (STT) y sintetizar voz (TTS)

---

## Entidades

### ServerConfig (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| provider | texto | sí | Nombre del LLM provider (ollama, openai, claude, gemini, groq, openrouter, nvidia) |
| model | texto | sí | Nombre del modelo |
| api_key | texto | no | API key (o vía variable de ambiente) |
| base_url | texto | no | URL base personalizada |

**Regla**: el server DEBE resolver provider usando la misma cadena que el CLI: flag explícito > variable de ambiente > auto-detect del modelo > default ollama. NO usar MockLLMResource.

### StreamEvent (modificada)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| event_type | enum [graph.started, node.started, node.token, node.output, node.completed, node.error, graph.completed] | sí | Tipo de evento |
| node_id | texto | no | Nodo que emite (null en graph.*) |
| data | json | sí | Payload del evento |
| timestamp_ms | número | sí | Momento de emisión (relativo al inicio) |

**Regla**: los eventos se emiten DURANTE la ejecución, no después. `node.token` se emite por cada token del LLM.

### UniverseSession (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| identificador | id | sí | ID de la sesión multi-agente |
| universe_config | referencia → UniverseConfig | sí | Config del universe (agentes, routing) |
| estado | enum [created, routing, executing, completed, failed] | sí | Estado actual |
| routing_decision | json | no | Qué agente fue seleccionado y por qué |
| agent_result | json | no | Resultado de la ejecución del agente |
| message_queue | lista de json | no | Cola de mensajes A2A pendientes |

**Ciclo de vida**:
```
┌─────────┐  mensaje  ┌─────────┐  agente    ┌───────────┐  resultado  ┌───────────┐
│ CREATED │─────────▶ │ ROUTING │──────────▶ │ EXECUTING │──────────▶ │ COMPLETED │
└─────────┘           └────┬────┘            └─────┬─────┘            └───────────┘
                           │                       │
                           │ no match              │ error
                           ▼                       ▼
                      ┌─────────┐            ┌─────────┐
                      │ FAILED  │            │ FAILED  │
                      └─────────┘            └─────────┘
```

### EvalRun (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| identificador | id | sí | ID de la evaluación |
| session_id | referencia → Session | sí | Sesión evaluada |
| eval_types | lista de texto | sí | Tipos: relevance, faithfulness, completeness, format_compliance, latency |
| estado | enum [pending, running, completed, failed] | sí | Estado |
| scores | json | no | Scores por evaluador (0.0 a 1.0) |
| details | json | no | Detalles: razones, evidence, timings |
| judge_model | texto | no | Modelo usado como juez (si LLM-as-judge) |

### RAGCollection (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| identificador | id | sí | ID de la colección |
| nombre | texto | sí | Nombre descriptivo |
| estado | enum [empty, ingesting, indexed, error] | sí | Estado |
| source_paths | lista de texto | sí | Archivos fuente ingestados |
| chunk_strategy | enum [fixed_size, sentence, paragraph, semantic] | sí | Estrategia de chunking |
| chunk_count | número | no | Cantidad de chunks |
| embedding_model | texto | sí | Modelo usado para embeddings |
| embedding_dimensions | número | no | Dimensiones del vector |

**Ciclo de vida**:
```
┌───────┐  ingest   ┌───────────┐  done    ┌─────────┐
│ EMPTY │─────────▶ │ INGESTING │───────▶ │ INDEXED │
└───────┘           └─────┬─────┘         └─────────┘
                          │ error
                          ▼
                     ┌─────────┐
                     │  ERROR  │
                     └─────────┘
```

### ChannelConnection (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| identificador | id | sí | ID de la conexión |
| platform | enum [telegram, slack, whatsapp, discord, webhook] | sí | Plataforma |
| estado | enum [disconnected, connecting, connected, error] | sí | Estado de conexión |
| credentials | json | sí | Token/API key de la plataforma (encriptado) |
| agent_id | referencia → Agent | sí | Agente que procesa mensajes |
| webhook_url | texto | no | URL para recibir webhooks |

### VoiceSession (nueva)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| identificador | id | sí | ID de la sesión de voz |
| estado | enum [idle, listening, transcribing, synthesizing, done, error] | sí | Estado |
| stt_provider | enum [whisper, google, azure, deepgram] | sí | Proveedor STT |
| tts_provider | enum [elevenlabs, openai_tts, google_tts, coqui] | no | Proveedor TTS |
| audio_format | enum [wav, mp3, ogg, webm] | sí | Formato de audio |
| transcript | texto | no | Texto transcrito |
| audio_output | archivo | no | Audio sintetizado |

---

## Reglas de Negocio

### R1: Zero mocks en producción
- **Invariante**: MockLLMResource, MockDBResource, y cualquier recurso "fake" solo se usan dentro de bloques `#[cfg(test)]` o archivos de test. El servidor, CLI, y runtime SIEMPRE resuelven recursos reales.
- **Cuándo se verifica**: al compilar y al arrancar el server
- **Si se viola**: error de compilación (los mocks no deben ser accesibles fuera de test scope) o error al arrancar sin provider configurado

### R2: Streaming es real-time
- **Invariante**: SSE events se emiten DURANTE la ejecución del grafo, no después. Cada token del LLM genera un evento `node.token`. El cliente recibe eventos mientras el grafo ejecuta.
- **Cuándo se verifica**: al ejecutar stream endpoint
- **Si se viola**: el usuario ve latencia completa antes de recibir cualquier output

### R3: Universe routing es determinista
- **Invariante**: dado el mismo input y la misma configuración, Universe SIEMPRE selecciona el mismo agente (excepto round_robin que rota por diseño). `llm_classify` usa LLM real para decidir routing.
- **Cuándo se verifica**: al routear mensaje
- **Si se viola**: comportamiento impredecible en multi-agent

### R4: Eval requiere sesión completada
- **Invariante**: no se puede evaluar una sesión en progreso o que no existe. El evaluador LLM-as-judge usa LLM real, no mock.
- **Cuándo se verifica**: al iniciar evaluación
- **Si se viola**: error "session not completed" o "session not found"

### R5: RAG ingest genera embeddings reales
- **Invariante**: al ingestar documentos, cada chunk se pasa por el modelo de embeddings real (context.llm().embed()). No se almacenan chunks sin vector.
- **Cuándo se verifica**: al ingestar
- **Si se viola**: error si el embedding model no está disponible

### R6: Channel adapters validan credenciales
- **Invariante**: antes de activar un adapter, se hace un health check contra la API de la plataforma. Credenciales inválidas → estado ERROR, no CONNECTED.
- **Cuándo se verifica**: al conectar
- **Si se viola**: error "invalid credentials" con detalle de la plataforma

### R7: Voice requiere provider disponible
- **Invariante**: transcripción requiere STT provider configurado y accesible. Síntesis requiere TTS provider configurado. Si no hay provider → error explícito, no silencio.
- **Cuándo se verifica**: al iniciar sesión de voz
- **Si se viola**: error "STT provider not configured" o "TTS provider not available"

---

## Patrones de Diseño

### Provider Resolution → Server + CLI unificado
- **Aplica a**: W1 (Server Real LLM)
- **Por qué**: la lógica de resolver provider (flag > env > auto-detect > default) ya existe en CLI. Server debe reusar exactamente la misma cadena de resolución.
- **Participantes**: ServerConfig, AdapterFactory, CLI resolve_provider()

```
┌─ Provider Resolution ─────────────────────────┐
│                                                │
│  Input: (provider?, model?, api_key?, url?)    │
│           │                                    │
│           ▼                                    │
│  ┌────────────────┐                            │
│  │ 1. Flag/Header │──▶ explícito? ──▶ usar    │
│  │ 2. Env var     │──▶ definido? ──▶ usar     │
│  │ 3. Auto-detect │──▶ modelo conocido? ──▶   │
│  │ 4. Default     │──▶ ollama                  │
│  └────────────────┘                            │
│           │                                    │
│           ▼                                    │
│  AdapterFactory::create_adapter(provider)      │
│           │                                    │
│           ▼                                    │
│  Box<dyn LLMResource> (REAL, nunca mock)       │
└────────────────────────────────────────────────┘
```

### Event Channel → Real-time streaming
- **Aplica a**: W2 (SSE Streaming)
- **Por qué**: GraphRunner ejecuta nodos secuencialmente/paralelamente. Para streaming real, necesita emitir eventos durante ejecución via canal asíncrono. El consumer (endpoint SSE) drena el canal y envía al cliente.
- **Participantes**: GraphRunner, EventChannel, SSE endpoint

```
┌─ Streaming Architecture ──────────────────────┐
│                                                │
│  GraphRunner                SSE Endpoint       │
│  ┌──────────┐               ┌──────────┐      │
│  │ run()    │──event_tx──▶  │ event_rx │──▶ SSE│
│  │          │               │          │      │
│  │ node A   │──token──▶    │ drain &  │      │
│  │ node B   │──output──▶   │ serialize│      │
│  │ ...      │──complete──▶ │ & send   │      │
│  └──────────┘               └──────────┘      │
└────────────────────────────────────────────────┘
```

### Agent Composition → Universe execution
- **Aplica a**: W3 (Universe Multi-Agent)
- **Por qué**: Universe routea mensajes a agentes. Cada agente es un GraphRunner completo con su propio Soul, tools, y context. Universe orquesta: selecciona agente → carga config → ejecuta → retorna respuesta.
- **Participantes**: Universe, AgentSpec, GraphRunner, Soul

```
┌─ Universe Execution ──────────────────────────┐
│                                                │
│  Message ──▶ Router ──▶ Agent Selection        │
│                              │                 │
│              ┌───────────────┤                 │
│              ▼               ▼                 │
│         ┌────────┐     ┌────────┐              │
│         │Agent A │     │Agent B │              │
│         │Soul +  │     │Soul +  │              │
│         │Graph + │     │Graph + │              │
│         │Tools   │     │Tools   │              │
│         └───┬────┘     └────────┘              │
│             │                                  │
│             ▼                                  │
│         Response ──▶ Caller                    │
└────────────────────────────────────────────────┘
```

---

## Operaciones

### W1: Server Real LLM

#### configurar_server
- **Actor**: Developer
- **Capacidad requerida**: configurar_provider
- **Input**:
  - provider (texto, opcional — resuelve por cadena si omitido)
  - model (texto, opcional)
  - api_key (texto, opcional — o vía env var)
  - base_url (texto, opcional)
- **Output exitoso**: confirmación con provider/model resueltos
- **Errores posibles**:
  - Provider no soportado → "unknown provider: {name}"
  - API key inválida → "authentication failed for {provider}"
- **Efectos secundarios**: server crea LLM adapter real para todas las ejecuciones

#### ejecutar_agente_via_api
- **Actor**: Host App
- **Capacidad requerida**: ejecutar_via_api
- **Input**:
  - agent_id (id, requerido)
  - trigger_data (json, opcional)
  - provider_override (texto, opcional — override per-request)
  - model_override (texto, opcional)
- **Output exitoso**: resultado de ejecución con state, trace, transcript
- **Errores posibles**:
  - Agente no encontrado → "agent not found"
  - LLM no disponible → "LLM provider unreachable: {details}"
  - Ejecución fallida → resultado con status=failed + error

### W2: Real-time SSE Streaming

#### stream_ejecucion
- **Actor**: Host App
- **Capacidad requerida**: ejecutar_via_api
- **Input**:
  - agent_id (id, requerido)
  - trigger_data (json, opcional)
- **Output exitoso**: flujo de StreamEvent emitidos en real-time durante la ejecución
- **Errores posibles**:
  - Agente no encontrado → "agent not found"
  - Conexión interrumpida → cierre limpio del canal
- **Efectos secundarios**: sesión creada y almacenada al completar

### W3: Universe Multi-Agent

#### enviar_mensaje_universe
- **Actor**: Host App / Developer
- **Capacidad requerida**: gestionar_universe
- **Input**:
  - universe_config (json, requerido — agentes + routing strategy)
  - message (texto, requerido)
  - sender_id (texto, opcional — para @mention routing)
- **Output exitoso**: UniverseSession con routing_decision + agent_result
- **Errores posibles**:
  - Ningún agente coincide → "no agent matched for routing"
  - Agente falla → resultado con status=failed del agente interno
  - Config inválida → "invalid universe config: {details}"

#### ejecutar_groupchat
- **Actor**: Host App / Developer
- **Capacidad requerida**: gestionar_universe
- **Input**:
  - agents (lista de json, requerido — specs de los agentes participantes)
  - topic (texto, requerido)
  - max_rounds (número, requerido)
  - moderator_strategy (enum [round_robin, llm_selected, topic_based], requerido)
  - consensus_threshold (número, opcional — 0.0 a 1.0)
- **Output exitoso**: lista de respuestas por ronda, consensus alcanzado (sí/no)
- **Errores posibles**:
  - Menos de 2 agentes → "groupchat requires at least 2 agents"
  - Max rounds excedido sin consensus → resultado parcial con consensus=false

#### enviar_mensaje_a2a
- **Actor**: Agent (internal)
- **Capacidad requerida**: ejecutar_tools
- **Input**:
  - target_agent_id (texto, requerido)
  - message_type (enum [request, response, broadcast, delegate], requerido)
  - payload (json, requerido)
- **Output exitoso**: confirmación de entrega con delivery_id
- **Errores posibles**:
  - Agente destino no encontrado → "target agent not found"
  - Cola llena → "message queue full for agent {id}"

### W4: Eval Integration

#### evaluar_sesion
- **Actor**: Developer
- **Capacidad requerida**: evaluar_sesion
- **Input**:
  - session_id (id, requerido)
  - eval_types (lista de texto, requerido — subconjunto de: relevance, faithfulness, completeness, format_compliance, latency)
  - judge_model (texto, opcional — modelo para LLM-as-judge, default al modelo del server)
- **Output exitoso**: EvalRun con scores por tipo (0.0 a 1.0) + details
- **Errores posibles**:
  - Sesión no encontrada → "session not found"
  - Sesión no completada → "session not completed, cannot evaluate"
  - LLM judge no disponible → "LLM judge unreachable" (para eval types que requieren LLM)

### W5: RAG Real Embeddings

#### crear_coleccion_rag
- **Actor**: Developer
- **Capacidad requerida**: gestionar_rag
- **Input**:
  - nombre (texto, requerido)
  - chunk_strategy (enum, requerido)
  - embedding_model (texto, opcional — default al modelo del server)
- **Output exitoso**: RAGCollection en estado EMPTY

#### ingestar_documentos
- **Actor**: Developer
- **Capacidad requerida**: gestionar_rag
- **Input**:
  - collection_id (id, requerido)
  - source_paths (lista de texto, requerido — archivos a ingestar)
- **Output exitoso**: RAGCollection en estado INDEXED con chunk_count
- **Errores posibles**:
  - Colección no encontrada → "collection not found"
  - Archivo no encontrado → "source file not found: {path}"
  - Embedding model no disponible → "embedding model unreachable"
  - Formato no soportado → "unsupported file format: {ext}"

#### buscar_en_rag
- **Actor**: Developer / Agent
- **Capacidad requerida**: buscar_rag
- **Input**:
  - collection_id (id, requerido)
  - query (texto, requerido)
  - top_k (número, opcional — default 5)
- **Output exitoso**: lista de chunks con score de similaridad
- **Errores posibles**:
  - Colección no encontrada → "collection not found"
  - Colección no indexada → "collection not indexed yet"

### W6: Server Endpoints Especializados

#### obtener_metricas
- **Actor**: Host App / Developer
- **Capacidad requerida**: ejecutar_via_api
- **Input**: ninguno
- **Output exitoso**: métricas agregadas (sesiones totales, duración promedio, errores, tools más usados)

#### obtener_trace_sesion
- **Actor**: Host App / Developer
- **Capacidad requerida**: ejecutar_via_api
- **Input**:
  - session_id (id, requerido)
- **Output exitoso**: trace tree con spans, durations, inputs/outputs por nodo
- **Errores posibles**:
  - Sesión no encontrada → "session not found"

### W7: Channel Adapters

#### conectar_canal
- **Actor**: Developer
- **Capacidad requerida**: conectar_canal
- **Input**:
  - platform (enum, requerido)
  - credentials (json, requerido — token/API key según plataforma)
  - agent_id (id, requerido — agente que procesa mensajes)
- **Output exitoso**: ChannelConnection en estado CONNECTED
- **Errores posibles**:
  - Credenciales inválidas → "authentication failed for {platform}"
  - Plataforma no soportada → "unsupported platform: {name}"
  - Agente no encontrado → "agent not found"

#### procesar_mensaje_entrante
- **Actor**: sistema (trigger automático desde plataforma)
- **Input**:
  - channel_id (id, requerido)
  - message (json, requerido — formato normalizado de la plataforma)
- **Output exitoso**: respuesta del agente enviada de vuelta a la plataforma
- **Errores posibles**:
  - Canal desconectado → "channel not connected"
  - Agente falla → error logueado, mensaje de error genérico enviado al usuario

### W8: Voice STT/TTS

#### transcribir_audio
- **Actor**: Developer / Agent
- **Capacidad requerida**: procesar_audio
- **Input**:
  - audio (archivo, requerido)
  - stt_provider (enum, requerido)
  - language (texto, opcional)
- **Output exitoso**: texto transcrito + confidence score
- **Errores posibles**:
  - Provider no disponible → "STT provider not configured"
  - Audio inválido → "unsupported audio format"
  - Transcripción fallida → "transcription failed: {details}"

#### sintetizar_voz
- **Actor**: Developer / Agent
- **Capacidad requerida**: procesar_audio
- **Input**:
  - text (texto, requerido)
  - tts_provider (enum, requerido)
  - voice_id (texto, opcional — voz específica del provider)
  - output_format (enum, requerido)
- **Output exitoso**: archivo de audio generado
- **Errores posibles**:
  - Provider no disponible → "TTS provider not configured"
  - Voice no encontrada → "voice not found: {id}"
  - Texto demasiado largo → "text exceeds provider limit"

---

## Interfaces

### CLI: mirai serve (modificada)

- **Propósito**: Arrancar server HTTP con LLM real
- **Actor**: Developer
- **Cambio**: acepta --provider, --model, --api-key, --base-url (mismos flags que `mirai run`)
- **Información que muestra**: provider/model resuelto, puerto, tools registrados
- **Ejemplo**: `mirai serve --port 3000 --provider openai --model gpt-4o`

### CLI: mirai universe (nueva)

- **Propósito**: Arrancar Universe multi-agente
- **Actor**: Developer
- **Acciones**:
  - `mirai universe start <config.yaml>` — arrancar Universe con config
  - `mirai universe send <message>` — enviar mensaje al Universe (stdin/flag)
- **Información que muestra**: routing decision, agente seleccionado, respuesta

### CLI: mirai eval (nueva)

- **Propósito**: Evaluar sesión de ejecución
- **Actor**: Developer
- **Acciones**:
  - `mirai eval <session_id> --types relevance,format_compliance`
- **Información que muestra**: scores por evaluador, detalles

### CLI: mirai rag (nueva)

- **Propósito**: Gestionar colecciones RAG
- **Actor**: Developer
- **Acciones**:
  - `mirai rag create --name <name> --strategy <strategy>`
  - `mirai rag ingest <collection_id> --source <path>`
  - `mirai rag search <collection_id> --query <text> --top-k <n>`
- **Información que muestra**: estado de colección, chunks indexados, resultados de búsqueda

### CLI: mirai agent (completada — reemplaza stubs)

- **Propósito**: Gestionar agentes
- **Actor**: Developer
- **Acciones**:
  - `mirai agent load <path.yaml>` — cargar agente desde archivo (antes: "not implemented")
  - `mirai agent list` — listar agentes cargados (antes: "not implemented")
- **Información que muestra**: agentes cargados con nombre, ID, graph info

### HTTP: Endpoints nuevos

| Método | Ruta | Operación | Input | Output |
|--------|------|-----------|-------|--------|
| POST | /api/agents/{id}/execute | ejecutar_agente_via_api | trigger_data, provider_override | resultado completo |
| POST | /api/agents/{id}/stream | stream_ejecucion | trigger_data | SSE events real-time |
| GET | /api/metrics | obtener_metricas | — | métricas agregadas |
| GET | /api/sessions/{id}/trace | obtener_trace_sesion | — | trace tree |
| POST | /api/universe/message | enviar_mensaje_universe | config, message | routing + result |
| POST | /api/universe/groupchat | ejecutar_groupchat | agents, topic, rounds | debate results |
| POST | /api/eval/{session_id} | evaluar_sesion | eval_types, judge_model | scores |
| POST | /api/rag/collections | crear_coleccion_rag | nombre, strategy | collection |
| POST | /api/rag/collections/{id}/ingest | ingestar_documentos | source_paths | ingestion result |
| POST | /api/rag/collections/{id}/search | buscar_en_rag | query, top_k | chunks |
| POST | /api/channels | conectar_canal | platform, credentials, agent_id | connection |
| POST | /api/voice/transcribe | transcribir_audio | audio, provider | transcript |
| POST | /api/voice/synthesize | sintetizar_voz | text, provider, voice_id | audio |

---

## Matriz de Permutaciones

| Flujo | Permutación | Actor | Resultado esperado |
|---|---|---|---|
| configurar_server | Happy path: provider válido | Developer | Server arranca con LLM real |
| configurar_server | Provider no soportado | Developer | Error: unknown provider |
| configurar_server | Sin provider → auto-detect | Developer | Resuelve por cadena (env > model > ollama) |
| configurar_server | API key inválida | Developer | Error al primer request, no al arrancar |
| ejecutar_via_api | Happy path: agente ejecuta con LLM real | Host App | Resultado con state, trace, transcript reales |
| ejecutar_via_api | Agente no encontrado | Host App | Error: agent not found |
| ejecutar_via_api | LLM unreachable | Host App | Error: provider unreachable |
| ejecutar_via_api | Provider override per-request | Host App | Usa provider del request, no del server |
| stream_ejecucion | Happy path: tokens llegan real-time | Host App | Eventos SSE durante ejecución |
| stream_ejecucion | Conexión cortada mid-stream | Host App | Server cierra canal limpiamente |
| stream_ejecucion | Agente con múltiples nodos LLM | Host App | Tokens de cada nodo en secuencia |
| enviar_mensaje_universe | Happy path: keyword routing | Developer | Agente correcto seleccionado y ejecutado |
| enviar_mensaje_universe | Happy path: llm_classify routing | Developer | LLM real decide routing |
| enviar_mensaje_universe | Ningún agente coincide | Developer | Error: no agent matched |
| enviar_mensaje_universe | Agente seleccionado falla | Developer | Error del agente propagado |
| ejecutar_groupchat | Happy path: 3 agentes, 5 rondas | Developer | Debate con respuestas por ronda |
| ejecutar_groupchat | Consensus alcanzado early | Developer | Para antes de max_rounds |
| ejecutar_groupchat | Menos de 2 agentes | Developer | Error: requires 2+ agents |
| enviar_mensaje_a2a | Happy path: request/response | Agent | Mensaje entregado, respuesta recibida |
| enviar_mensaje_a2a | Agente destino no existe | Agent | Error: target not found |
| evaluar_sesion | Happy path: eval programático | Developer | Scores por tipo |
| evaluar_sesion | Happy path: LLM-as-judge | Developer | Score + reasoning del LLM |
| evaluar_sesion | Sesión no completada | Developer | Error: session not completed |
| evaluar_sesion | Sesión no existe | Developer | Error: session not found |
| crear_coleccion_rag | Happy path | Developer | Colección creada en estado EMPTY |
| ingestar_documentos | Happy path: PDF + MD | Developer | Chunks creados con embeddings |
| ingestar_documentos | Archivo no encontrado | Developer | Error: file not found |
| ingestar_documentos | Embedding model no disponible | Developer | Error: model unreachable |
| buscar_en_rag | Happy path: query con resultados | Developer | Top-K chunks con scores |
| buscar_en_rag | Colección vacía | Developer | Lista vacía |
| buscar_en_rag | Colección no indexada | Developer | Error: not indexed |
| conectar_canal | Happy path: Telegram | Developer | Connection CONNECTED |
| conectar_canal | Credenciales inválidas | Developer | Error: auth failed |
| conectar_canal | Happy path: Slack | Developer | Connection CONNECTED |
| procesar_mensaje_entrante | Happy path: mensaje → agente → respuesta | Sistema | Respuesta enviada a plataforma |
| procesar_mensaje_entrante | Canal desconectado | Sistema | Error: not connected |
| transcribir_audio | Happy path: Whisper | Developer | Texto transcrito |
| transcribir_audio | Provider no configurado | Developer | Error: not configured |
| transcribir_audio | Audio inválido | Developer | Error: unsupported format |
| sintetizar_voz | Happy path: ElevenLabs | Developer | Audio generado |
| sintetizar_voz | Provider no disponible | Developer | Error: not configured |
| README update | Estructura, tests, tools, commands correctos | Developer | Documentación acurada |
| agent load | Happy path: cargar YAML | Developer | Agente registrado |
| agent list | Con agentes cargados | Developer | Lista con nombres e IDs |

---

## Escenarios GWT

### Journey: Developer — Server con LLM real

TEST-024: Server arranca con provider real
  Given: binary compilado, Ollama corriendo en localhost
  When: Developer ejecuta `mirai serve --port 3000 --provider ollama --model gemma3`
  Then: Server arranca, health endpoint reporta provider=ollama, model=gemma3

TEST-025: Server resuelve provider por env var
  Given: MIRAI_LLM_PROVIDER=openai, OPENAI_API_KEY=sk-xxx en ambiente
  When: Developer ejecuta `mirai serve --port 3000` (sin flags de provider)
  Then: Server resuelve provider=openai, usa API key del env

TEST-026: Ejecución via API usa LLM real (no mock)
  Given: Server corriendo con provider=ollama
  When: Host App hace POST /api/agents/{id}/execute
  Then: Respuesta contiene output generado por LLM real (no "mock response")
  And: trace muestra llamada real al LLM con latency > 0ms

TEST-027: Provider override per-request
  Given: Server corriendo con provider=ollama
  When: Host App hace POST /api/agents/{id}/execute con header X-LLM-Provider: openai
  Then: Ejecución usa openai para ese request, server sigue con ollama para los demás

### Journey: Host App — SSE Streaming real-time

TEST-028: Tokens llegan durante ejecución
  Given: Server corriendo, agente con nodo ai/llm_call
  When: Host App hace POST /api/agents/{id}/stream
  Then: Recibe event node.started ANTES de node.completed
  And: Recibe events node.token con tokens individuales entre started y completed
  And: Tiempo entre primer evento y último > 0 (no batch)

TEST-029: Stream con múltiples nodos
  Given: Agente con 3 nodos secuenciales (trigger → llm_call → output)
  When: Host App hace POST /api/agents/{id}/stream
  Then: Recibe graph.started, luego node events para cada nodo en orden, luego graph.completed
  And: Cada nodo tiene su par started/completed

TEST-030: Conexión cortada mid-stream
  Given: Stream en progreso
  When: Cliente cierra conexión
  Then: Server detecta desconexión, limpia recursos, ejecución continúa en background

### Journey: Developer — Universe Multi-Agent

TEST-031: Universe routea por keyword
  Given: Universe con 2 agentes: "ventas" (keywords: comprar, precio) y "soporte" (keywords: error, ayuda)
  When: Developer envía mensaje "¿cuál es el precio del plan pro?"
  Then: Universe selecciona agente "ventas"
  And: Agente ejecuta y retorna respuesta

TEST-032: Universe routea por llm_classify
  Given: Universe con 3 agentes especializados, routing=llm_classify
  When: Developer envía mensaje ambiguo
  Then: LLM real clasifica y selecciona agente más apropiado
  And: routing_decision incluye reasoning del LLM

TEST-033: GroupChat con consensus
  Given: 3 agentes con Souls distintos, topic="¿debemos migrar a microservicios?"
  When: Developer ejecuta groupchat con max_rounds=5, consensus_threshold=0.7
  Then: Cada ronda produce respuesta de cada agente
  And: Moderador evalúa consensus después de cada ronda
  And: Para cuando consensus >= threshold o max_rounds alcanzado

TEST-034: A2A message delivery
  Given: Agente A con tool agent/send_message configurado, Agente B registrado
  When: Agente A envía message_type=request a Agente B
  Then: Mensaje entregado a cola de B
  And: B procesa y retorna response
  And: A recibe la response en su estado

### Journey: Developer — Eval

TEST-035: Eval programático (latency, format_compliance)
  Given: Sesión completada con trace
  When: Developer ejecuta eval con types=[latency, format_compliance]
  Then: Score de latency basado en duración real (0-1)
  And: Score de format_compliance basado en schema match (0-1)
  And: No requiere LLM (evaluación programática)

TEST-036: Eval LLM-as-judge (relevance, faithfulness)
  Given: Sesión completada con transcript
  When: Developer ejecuta eval con types=[relevance, faithfulness]
  Then: LLM real evalúa relevancia del output vs input
  And: LLM real evalúa fidelidad del output vs contexto
  And: Scores entre 0.0 y 1.0 con reasoning

TEST-037: Eval via CLI
  Given: Sesión completada con session_id conocido
  When: Developer ejecuta `mirai eval <session_id> --types relevance,format_compliance`
  Then: CLI muestra tabla con scores por evaluador
  And: Exit code 0 si todos los scores > 0.5

TEST-038: Eval sesión no completada
  Given: Sesión en progreso (status != completed)
  When: Developer intenta evaluar
  Then: Error: "session not completed, cannot evaluate"

### Journey: Developer — RAG

TEST-039: Crear colección e ingestar
  Given: Directorio con 3 archivos markdown
  When: Developer ejecuta `mirai rag create --name docs --strategy paragraph` y luego `mirai rag ingest <id> --source ./docs/`
  Then: Colección pasa a estado INDEXED
  And: chunk_count > 0
  And: Cada chunk tiene embedding vector real (no zeros)

TEST-040: Buscar en RAG
  Given: Colección indexada con documentos sobre "configuración de agentes"
  When: Developer ejecuta `mirai rag search <id> --query "cómo configuro un agente" --top-k 3`
  Then: Retorna 3 chunks más relevantes
  And: Scores de similaridad entre 0.0 y 1.0
  And: Chunks contienen texto relacionado con la query

TEST-041: RAG como tool dentro de agente
  Given: Agente con nodo data/rag_search, colección indexada
  When: Agente ejecuta y el nodo busca "deployment instructions"
  Then: Nodo retorna chunks relevantes en su output
  And: Siguiente nodo puede usar esos chunks como contexto para LLM

TEST-042: Ingest con embedding model no disponible
  Given: Colección creada, embedding model configurado como "nonexistent-model"
  When: Developer intenta ingestar documentos
  Then: Error: "embedding model unreachable"
  And: Colección queda en estado ERROR (no INDEXED)

### Journey: Developer — Channels

TEST-043: Conectar Telegram
  Given: Bot token de Telegram válido, agente registrado
  When: Developer hace POST /api/channels con platform=telegram, credentials={token}
  Then: Engine valida token con Telegram API (getMe)
  And: ChannelConnection en estado CONNECTED
  And: Webhook registrado para recibir mensajes

TEST-044: Recibir y responder mensaje Telegram
  Given: Canal Telegram conectado a agente
  When: Usuario envía "hola" al bot
  Then: Engine recibe webhook, ejecuta agente con input="hola"
  And: Respuesta del agente enviada de vuelta via Telegram sendMessage API

TEST-045: Conectar Slack
  Given: Bot token de Slack válido, agente registrado
  When: Developer hace POST /api/channels con platform=slack, credentials={token}
  Then: Engine valida token con Slack auth.test API
  And: ChannelConnection en estado CONNECTED

TEST-046: Credenciales inválidas
  Given: Token de Telegram inválido
  When: Developer intenta conectar
  Then: Error: "authentication failed for telegram"
  And: ChannelConnection en estado ERROR

### Journey: Developer — Voice

TEST-047: Transcribir con Whisper
  Given: Archivo WAV con habla en español
  When: Developer hace POST /api/voice/transcribe con provider=whisper
  Then: Retorna texto transcrito
  And: Confidence score > 0.8

TEST-048: Sintetizar con OpenAI TTS
  Given: Texto "Hola, soy un agente de Mirai"
  When: Developer hace POST /api/voice/synthesize con provider=openai_tts, format=mp3
  Then: Retorna archivo MP3 con audio sintetizado
  And: Duración del audio > 0 seconds

TEST-049: STT provider no configurado
  Given: No hay API key de Whisper configurada
  When: Developer intenta transcribir
  Then: Error: "STT provider not configured: whisper"

### Journey: Developer — Cleanup

TEST-050: README refleja estado real
  Given: README.md en raíz del proyecto
  When: Developer lee README
  Then: Estructura de directorio coincide con la real (no dice `rust/`)
  And: Número de tests coincide con `cargo test` real
  And: Lista de tools completa (47)
  And: Quick start funciona tal como está documentado

TEST-051: mirai agent load funciona
  Given: Archivo agent.yaml válido
  When: Developer ejecuta `mirai agent load agent.yaml`
  Then: Agente cargado y registrado
  And: Mensaje de confirmación con nombre y ID

TEST-052: mirai agent list funciona
  Given: 2 agentes cargados previamente
  When: Developer ejecuta `mirai agent list`
  Then: Lista 2 agentes con nombre, ID, nodos, edges

---

## Fuera de Alcance

- **Observability UI** (web dashboard para traces): requiere frontend, será PRD separado
- **OTLP export** (enviar trazas a Grafana/Datadog): será PRD separado
- **Marketplace de tools**: futuro
- **WhatsApp Cloud API**: requiere Meta Business verification, solo Telegram + Slack para v0.2.0
- **Discord adapter**: Telegram + Slack son prioridad, Discord en v0.3.0
- **Coqui TTS local**: requiere modelo local, solo APIs cloud (Whisper, ElevenLabs, OpenAI TTS) para v0.2.0
- **Embeddings locales**: v0.2.0 usa API de embeddings del provider (OpenAI, Ollama); embeddings locales en futuro
- **Agent persistence en disco**: agentes se cargan en memoria; persistencia SQLite en futuro

---

## Dependencias

- **W1 (Server Real LLM)** debe completarse PRIMERO — W3, W4, W5 dependen de LLM real
- **W2 (Streaming)** necesita refactor de GraphRunner para emitir eventos via canal
- **W5 (RAG)** necesita embedding support en al menos 1 provider (OpenAI o Ollama)
- **W7 (Channels)** necesita acceso a APIs externas (Telegram Bot API, Slack Events API)
- **W8 (Voice)** necesita acceso a APIs externas (OpenAI Whisper, ElevenLabs)
- **Ollama** debe estar corriendo localmente para tests de integración

### Orden de implementación recomendado

```
Fase 1 (blocker):  W1 → W2 → W9
                   Server real + streaming real + cleanup

Fase 2 (core):    W3 → W5 → W4
                   Universe + RAG + Eval (usan LLM real de W1)

Fase 3 (endpoints): W6
                     Server endpoints para todo lo anterior

Fase 4 (integración): W7 → W8
                       Channels + Voice (APIs externas)
```
