# Showcase — real agents from the community

Real agent specs donated from actual projects built on OpenMirai (image editing,
voice pipelines, framework-comparison demos). Unlike `examples/*.yaml` (minimal
teaching examples), these are agents that shipped inside real apps — copied here
verbatim, secrets scrubbed, and adapted only where the v0.7.0 spec required it.

Every file in this folder passes `mirai validate`. Whether it *executes* depends
on which tools it uses:

| State | Meaning |
|---|---|
| **runnable** | Runs green end-to-end with `--provider mock` (no keys, no network). |
| **needs-key / needs-llm** | Graph uses only built-in tools, but needs a real provider (API key or local Ollama) to produce real output. |
| **showcase** | References custom tools that lived in the host app's tool registry — the engine's built-in registry doesn't have them, so the spec is illustrative. `mirai validate` passes (validation checks the graph, not the registry); execution stops at the custom node with `tool not found`. To run one, register the custom tools via the SDK. |

All commands below assume you are in the repo root with a built binary
(`cargo build --release`), using `target/release/mirai` or `mirai` on PATH.

---

## `tunevision/` — TuneVision (AI image editor)

Agent extracted from **TuneVision**, a macOS/iOS image-editing app.

### `tunevision-editor.yaml` — **needs-key (OPENAI_API_KEY)**

Edits a specific area of an image using AI inpainting (`ai/image_edit`,
OpenAI GPT-Image-1). Takes a base PNG, an optional mask (transparent = area to
edit), and a text prompt; returns the edited PNG plus OpenAI's revised prompt.

```bash
export OPENAI_API_KEY=sk-...
mirai run examples/showcase/tunevision/tunevision-editor.yaml \
  --input '{"image_path": "./photo.png", "prompt": "make the background a sunset"}'
```

Why it needs a key: `ai/image_edit` calls the OpenAI Images API directly — the
`--provider mock` flag only mocks the *LLM* resource, not the image tool.
Without the key it fails with `OpenAI API key not found`.

---

## `voice-aftrmeet/` — Aftrmeet voice pipeline (TTS + STT)

Agents extracted from **Aftrmeet**, an iOS meeting-notes app (record → transcribe
→ insights). Only the voice agents are donated here. All three are **runnable**
— verified green with `--provider mock`. Note: `tts.yaml` runs as-is, but the two
STT agents transcribe a file, so point `audio_path` at a **real audio file that
exists** on disk (a missing path errors out — that's correct behavior). For real
transcription/preprocessing use a Gemini key (the specs pin `gemini-2.5-flash`).

### `tts.yaml` — **runnable**

Text → text optimized for speech synthesis (expands abbreviations, adds natural
pauses, fixes punctuation). The actual audio synthesis happens in the host app;
this agent is the LLM preprocessing stage.

```bash
# smoke test (no keys)
mirai run examples/showcase/voice-aftrmeet/tts.yaml --provider mock \
  --input '{"text": "The avg. resp. time is 200ms, dr. Smith."}'

# real run
mirai run examples/showcase/voice-aftrmeet/tts.yaml \
  --provider gemini --api-key $GOOGLE_API_KEY \
  --input '{"text": "The avg. resp. time is 200ms, dr. Smith."}'
```

### `dictation-stt.yaml` — **runnable**

Audio file → faithful verbatim transcription (`ai/transcribe`). Dedicated to
short, single-speaker dictation — never normalizes, rewrites, or omits words.

```bash
mirai run examples/showcase/voice-aftrmeet/dictation-stt.yaml --provider mock \
  --input '{"audio_path": "./recording.m4a"}'
# real: --provider gemini --api-key $GOOGLE_API_KEY
```

### `meeting-stt.yaml` — **runnable**

Same faithful-verbatim transcription, as a separate agent for long multi-speaker
meeting audio — kept apart from dictation so each can be tuned independently
without risking dictation fidelity.

```bash
mirai run examples/showcase/voice-aftrmeet/meeting-stt.yaml --provider mock \
  --input '{"audio_path": "./meeting.m4a"}'
# real: --provider gemini --api-key $GOOGLE_API_KEY
```

---

## `langgraph-comparison/` — LangGraph/LangChain comparison demos

Agent specs from a benchmark project that reimplemented classic
LangChain/LangGraph patterns on OpenMirai (and n8n) to compare the three
frameworks. File numbering follows the original demo numbering — demos 08–13
built their specs inline in Python (no standalone YAML), so they are not
included here; 14a/14b keep their original numbers.

Note: the original specs declared a `resources:` block from a pre-0.7 schema;
it was removed here (provider selection is the CLI's job — `--provider`,
`MIRAI_LLM_PROVIDER`, or default Ollama). Graphs are otherwise untouched.

### `01-llm-provider.yaml` — **runnable**

The "hello world" of the comparison: manual trigger → one `ai/llm_call`.
The trigger carries a default payload, so no `--input` is needed.

```bash
mirai run examples/showcase/langgraph-comparison/01-llm-provider.yaml --provider mock
```

### `02-structured-output.yaml` — **needs-llm**

Meeting transcript → structured insights (title, participants, action items,
summary) enforced via `output_schema` + `schema_retries`. Needs a *real* LLM:
the mock provider returns a canned non-JSON string, so schema validation fails
by design. Works with any real provider (Ollama, OpenAI, Claude, Gemini, Groq).

```bash
mirai run examples/showcase/langgraph-comparison/02-structured-output.yaml \
  --provider ollama --model gemma3
```

### `03-lcel-chain.yaml` — **runnable**

The LCEL-chain equivalent: a single-prompt text classifier
(bug / feature / question / other).

```bash
mirai run examples/showcase/langgraph-comparison/03-lcel-chain.yaml --provider mock
```

### `04-tool-calling.yaml` — **showcase**

Tool-calling pattern using a custom `custom/get_weather` tool that lived in the
comparison harness. Not in the built-in registry → execution stops with
`tool not found: custom/get_weather`.

### `05-rag-pipeline.yaml` — **showcase**

RAG pattern: vector search → grounded generation. Uses `data/vector_search`,
which is not a built-in (today's engine ships `data/rag_search` instead — see
`examples/` for a runnable RAG-style pipeline). Kept as-is for fidelity to the
original comparison.

### `06-state-graph.yaml` — **showcase**

StateGraph equivalent: an order pipeline (`order/validate` → `order/calculate`
→ `order/confirm`) built on custom tools registered by the comparison harness.
Illustrates multi-step state passing via `data_map`.

### `07-conditional-routing.yaml` — **showcase**

Conditional-edges equivalent: `ticket/classify` fans out to high/medium/low
handlers via `condition:` on edges. Custom `ticket/*` tools → illustrative.
The conditional-edge syntax itself is fully supported (see
`examples/conditional-routing.yaml` for a runnable version).

### `14a-cross-platform.yaml` — **runnable**

The same text classifier as 03 but exposed as a webhook (`triggers:` block with
bearer auth) — the "one YAML, CLI + server" portability demo.

```bash
# one-shot via CLI
mirai run examples/showcase/langgraph-comparison/14a-cross-platform.yaml \
  --provider mock --input '{"texto": "El botón de login no responde"}'

# or serve it as an HTTP endpoint
mirai serve
```

### `14b-meeting-transcriber.yaml` — **showcase**

The full Aftrmeet-style pipeline as one graph: transcript → structured insights
→ embed → chat-with-your-meeting. `meeting/embed` and `meeting/chat` were
custom tools in the host app → illustrative. (The insights-extraction node
alone is the same pattern as 02.)

---

## Verification snapshot (v0.7.0)

Every YAML re-validated and mock-run on the 0.7.0 binary:

- `mirai validate` — **13/13 pass**.
- `mirai run --provider mock` — green (`error: null`): `tts`, `dictation-stt`,
  `meeting-stt`, `01`, `03`, `14a`.
- Needs a real provider: `02` (schema validation vs. mock output),
  `tunevision-editor` (OpenAI Images API).
- Stops at custom tool, as documented: `04`, `05`, `06`, `07`, `14b`.
