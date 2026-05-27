# Gaps — PRD-001 Features pendientes de completar

Estado al v0.1.0. Cada feature tiene core logic implementada pero falta integración.

## Quick Wins (1-2 horas cada uno)

### GAP-A: Registrar sandbox como tool
- **Módulo**: `engine/src/sandbox.rs`
- **Estado**: Lógica 100% funcional (execute, timeout, isolation)
- **Falta**: Registrar `system/sandbox_exec` en `tools/builtin/system.rs` que llame a `sandbox::execute()`
- **Esfuerzo**: ~50 líneas

### GAP-B: Conectar security scanner al runner
- **Módulo**: `engine/src/security.rs`
- **Estado**: Scanner 100% funcional (13 patrones, 3 sensibilidades)
- **Falta**: Pre-hook en GraphRunner que llame `security::scan()` antes de cada `ai/llm_call`. Leer config de `security.prompt_injection_scanner` del AgentSpec.
- **Esfuerzo**: ~40 líneas en runner.rs

### GAP-C: Inyectar Soul en ejecución de agente
- **Módulo**: `engine/src/soul.rs`
- **Estado**: Parser 100% funcional (YAML frontmatter → system prompt)
- **Falta**: Cuando AgentSpec tiene campo `soul: "./path/to/SOUL.md"`, cargar el Soul y setear `context.system_prompt` con `soul.to_system_prompt()`.
- **Esfuerzo**: ~30 líneas en cli/main.rs y server/app.rs

### GAP-D: CLI commands para templates
- **Módulo**: `engine/src/templates.rs`
- **Estado**: 10 templates completos, `get_template()` y `list_templates()` funcionan
- **Falta**: Agregar subcomandos `mirai templates` (listar) y `mirai new --template X --name Y` (generar YAML)
- **Esfuerzo**: ~80 líneas en cli/main.rs

### GAP-E: Endpoint /stream SSE
- **Módulo**: `engine/src/streaming.rs`
- **Estado**: StreamEvent enum + SSE formatter 100% funcionales
- **Falta**: (1) Ruta `POST /api/agents/{id}/stream` en server/app.rs, (2) GraphRunner debe emitir StreamEvents durante ejecución (callback), (3) Response streaming via axum SSE
- **Esfuerzo**: ~150 líneas

## Medium Effort (3-6 horas cada uno)

### GAP-F: Universe ejecuta agentes (no solo routea)
- **Módulo**: `engine/src/universe.rs`
- **Estado**: Routing funciona (keyword, explicit, round-robin). A2A types definidos. GroupChat types definidos.
- **Falta**: (1) `Universe.handle_message()` que routea → carga el workflow del Soul → ejecuta con GraphRunner → retorna respuesta. (2) A2A message delivery queue. (3) GroupChat execution loop (N rondas con LLM).
- **Esfuerzo**: ~300 líneas

### GAP-G: Eval integrado en ejecución
- **Módulo**: `engine/src/eval.rs`
- **Estado**: `eval_format_compliance()` y `eval_latency()` funcionan. Judge prompts generados.
- **Falta**: (1) Registrar como tool `eval/run`. (2) CLI `mirai eval <session_id>`. (3) Hook post-execution que ejecute evals configurados. (4) LLM-as-judge necesita llamar al LLM real.
- **Esfuerzo**: ~200 líneas

### GAP-H: RAG con embeddings reales
- **Módulo**: `engine/src/rag.rs`
- **Estado**: Chunking funciona (4 estrategias). read_file_for_rag funciona.
- **Falta**: (1) Conectar con `context.llm().embed()` para generar embeddings. (2) Almacenar en VectorResource existente. (3) `data/rag_search` tool que busque chunks por similitud. (4) CLI `mirai rag create` + `mirai rag ingest`.
- **Esfuerzo**: ~250 líneas

### GAP-I: Server endpoints especializados
- **Módulo**: `engine/src/server/app.rs`
- **Falta**: Endpoints para templates (`/api/templates`), eval (`/api/eval`), RAG (`/api/rag`), metrics (`/api/metrics`), stream (`/api/agents/{id}/stream`)
- **Esfuerzo**: ~200 líneas (5 endpoints)

## High Effort (1-2 semanas cada uno)

### GAP-J: Channel adapters reales (WhatsApp, Telegram, Slack)
- **Módulo**: `engine/src/channels.rs`
- **Estado**: Types y trait definidos. WebhookAdapter funcional.
- **Falta**: (1) TelegramAdapter usando Bot API (long polling o webhook). (2) SlackAdapter usando Events API. (3) WhatsAppAdapter usando Business API. (4) Message ingestion loop. (5) Channel management CLI.
- **Dependencias**: SDKs de cada plataforma o HTTP directo.
- **Esfuerzo**: ~500 líneas por adapter

### GAP-K: Voice STT/TTS real
- **Módulo**: `engine/src/voice.rs`
- **Estado**: Solo tipos y config.
- **Falta**: (1) Whisper integration (HTTP o local). (2) TTS provider integration (ElevenLabs, OpenAI TTS). (3) Audio streaming. (4) Tool registration `voice/transcribe` y `voice/speak`.
- **Dependencias**: Audio processing crate, provider APIs.
- **Esfuerzo**: ~400 líneas por provider

## Orden recomendado

```
Quick wins (v0.1.1):
  GAP-A → GAP-B → GAP-C → GAP-D → GAP-E

Medium (v0.2.0):
  GAP-F → GAP-G → GAP-H → GAP-I

High (v0.3.0+):
  GAP-J → GAP-K
```
