# Gaps — Feature tracking

Estado al v0.4.3 — documento histórico. Versión actual: v0.7.0; ver [ROADMAP-PARITY.md](ROADMAP-PARITY.md) para el estado más reciente.

## Completados

| GAP | Feature | Resuelto en |
|-----|---------|-------------|
| GAP-A | Sandbox como tool (`system/sandbox_exec`) | v0.3.0 |
| GAP-B | Scanner integrado en `ai/llm_call` | v0.3.0 |
| GAP-C | Soul inyectado en ejecucion (CLI + server) | v0.3.0 |
| GAP-D | `mirai templates` + `mirai new --template` | v0.3.0 |
| GAP-E | `POST /api/v1/agents/{id}/stream` SSE + `/api/v1/templates` | v0.4.0 |
| GAP-001 | LLM providers reales en CLI (7 providers) | v0.4.0 |
| GAP-002 | Structured output con `output_schema` + auto-retry | v0.4.0 |
| GAP-I | Server endpoints especializados (metrics, rag/search, eval, universe) | v0.4.0 |

## Pendientes — Medium Effort (3-6 horas cada uno)

### GAP-F: Universe ejecuta agentes completos
- **Modulo**: `engine/src/universe.rs`
- **Estado**: Routing funciona (keyword, explicit, round-robin, LLM-based)
- **Falta**:
  1. `Universe.handle_message()` → routea + carga workflow del Soul + ejecuta con GraphRunner + retorna respuesta
  2. A2A message delivery queue (cola interna por agente)
  3. GroupChat execution loop (N rondas con LLM)
  4. CLI: `mirai universe start <config.yaml>`

### GAP-G: Eval integrado en ejecucion
- **Modulo**: `engine/src/eval.rs`
- **Estado**: Evaluadores programaticos funcionan, judge prompts generados
- **Falta**:
  1. Registrar `eval/run` como tool
  2. CLI: `mirai eval <session_id> --types relevance,format_compliance`
  3. Hook post-execution que ejecute evals configurados en AgentSpec
  4. LLM-as-judge: llamar al LLM real para relevance/faithfulness/completeness

### GAP-H: Ingesta RAG persistente
- **Modulo**: `engine/src/rag.rs`
- **Estado**: HTTP, `data/rag_search` y `mirai rag search` usan embeddings reales y similitud coseno. La indexacion por request no es persistente.
- **Falta**:
  1. CLI: `mirai rag ingest --source ./docs/`
  2. Indice persistente, incremental y reutilizable entre requests
  3. Politicas de actualizacion, borrado, namespace y retencion

## Pendientes — High Effort (1-2 semanas cada uno)

### GAP-J: Channel adapters
- **Estado**: `channels.rs` y `voice.rs` eliminados en v0.4.0 (dead code). Empezar desde cero.
- **Falta**: TelegramAdapter (Bot API), SlackAdapter (Events API), WhatsAppAdapter (Business API)
- **Approach**: Cada adapter como feature flag independiente. ~500 lineas cada uno.

### GAP-K: Voice STT/TTS
- **Estado**: `voice.rs` eliminado en v0.4.0. Empezar desde cero.
- **Falta**: Whisper integration (STT), ElevenLabs/OpenAI TTS integration, audio streaming
- **Approach**: Modulo nuevo `engine/src/voice/`. Feature flag `voice`.

## Pendientes — Roadmap (ver ROADMAP-PARITY.md)

| GAP | Feature | Esfuerzo |
|-----|---------|----------|
| GAP-003 | Observabilidad de produccion (UI + export OTLP; el JSON `/otel-trace` ya existe) | Alto |
| GAP-004 | Fan-out dinamico (`logic/fan_out` con N runtime) | Medio |
| GAP-005 | Cross-thread memory (store + semantic search) | Medio |
| GAP-006 | Subgrafos composables (mejor data_map bidireccional) | Medio |
| GAP-007 | Middleware lifecycle (hooks mas granulares) | Medio |

## Orden recomendado

```
Siguiente:  GAP-F → GAP-H → GAP-G
Despues:    GAP-004 → GAP-005 → GAP-003
Futuro:     GAP-J → GAP-K → GAP-006 → GAP-007
```
