# Gaps — PRD-001 pendientes

Estado al v0.1.0. Actualizado después de quick wins.

## Completados

| GAP | Feature | Estado |
|-----|---------|--------|
| GAP-A | Sandbox como tool (`system/sandbox_exec`) | done |
| GAP-B | Scanner integrado en `ai/llm_call` | done |
| GAP-C | Soul inyectado en ejecución (CLI + server) | done |
| GAP-D | `mirai templates` + `mirai new --template` | done |
| GAP-E | `POST /api/agents/{id}/stream` SSE + `/api/templates` | done |

## Pendientes — Medium Effort (3-6 horas cada uno)

### GAP-F: Universe ejecuta agentes completos
- **Módulo**: `engine/src/universe.rs`
- **Estado**: Routing funciona (keyword, explicit, round-robin)
- **Falta**:
  1. `Universe.handle_message()` → routea + carga workflow del Soul + ejecuta con GraphRunner + retorna respuesta
  2. A2A message delivery queue (cola interna por agente)
  3. GroupChat execution loop (N rondas con LLM)
  4. CLI: `mirai universe start <config.yaml>`

### GAP-G: Eval integrado en ejecución
- **Módulo**: `engine/src/eval.rs`
- **Estado**: Evaluadores programáticos funcionan, judge prompts generados
- **Falta**:
  1. Registrar `eval/run` como tool
  2. CLI: `mirai eval <session_id> --types relevance,format_compliance`
  3. Hook post-execution que ejecute evals configurados en AgentSpec
  4. LLM-as-judge: llamar al LLM real para relevance/faithfulness/completeness

### GAP-H: RAG con embeddings reales
- **Módulo**: `engine/src/rag.rs`
- **Estado**: Chunking funciona (4 estrategias)
- **Falta**:
  1. Conectar con `context.llm().embed()` para generar embeddings de cada chunk
  2. Almacenar embeddings en VectorResource existente (`SimpleVectorResource`)
  3. `data/rag_search` tool: busca chunks por similitud coseno
  4. CLI: `mirai rag create --source ./docs/` + `mirai rag ingest`

### GAP-I: Server endpoints especializados
- **Módulo**: `engine/src/server/app.rs`
- **Falta**:
  1. `GET /api/metrics` — métricas agregadas (del módulo observability)
  2. `GET /api/sessions/{id}/trace` — trace tree de una sesión
  3. `POST /api/rag/ingest` — ingestar documentos
  4. `POST /api/rag/search` — buscar en RAG

## Pendientes — High Effort (1-2 semanas cada uno)

### GAP-J: Channel adapters reales
- **Módulo**: `engine/src/channels.rs`
- **Estado**: Types + trait + WebhookAdapter
- **Falta**: TelegramAdapter (Bot API), SlackAdapter (Events API), WhatsAppAdapter (Business API)
- **Cada adapter**: ~500 líneas, necesita SDK o HTTP directo a la API del proveedor

### GAP-K: Voice STT/TTS real
- **Módulo**: `engine/src/voice.rs`
- **Estado**: Solo tipos y config
- **Falta**: Whisper integration (STT), ElevenLabs/OpenAI TTS integration, audio streaming
- **Cada provider**: ~400 líneas, necesita manejo de audio (bytes, codecs)

## Orden recomendado

```
v0.2.0: GAP-F → GAP-H → GAP-G → GAP-I
v0.3.0: GAP-J → GAP-K
```
