# Advanced Subsystems and Wiring Status

This guide maps supporting engine modules that do not belong to the main
AgentSpec → GraphRunner → Tool lifecycle. It prevents library types, partial
HTTP handlers, and product-ready features from being conflated.

Status terms match [System Lifecycle](../SYSTEM_LIFECYCLE.md): **wired**,
**library-only**, **partial**, and **placeholder**.

## Summary

| Subsystem | Main source | Status | Active surface |
|---|---|---|---|
| Evaluation | `eval.rs` | Partial/wired HTTP | `POST /api/v1/eval`; no eval graph tool or automatic post-run hook. |
| Universe | `universe.rs` | Partial | HTTP routing can execute an already registered agent; broader persistent universe/A2A product is incomplete. |
| Group chat | server handler | Wired specialized HTTP | Repeated direct LLM calls, not a GraphRunner multi-agent graph. |
| Intelligence | `intelligence/` | Mostly library-only | Analysis/context/memory helper types; not a default automatic execution pipeline. |
| Energy | `energy/` | Library-only | Rate calculation and in-memory recording primitives; not globally attached metering/billing. |
| Render | `render/` | Library/CLI helper | Custom Markdown/Chart HTML generation; not a general browser service. |
| Benchmark | `benchmark.rs` | Wired when enabled | Local append-only JSONL timing/memory samples. |
| Catalog | `catalog.rs` | Wired in editor/model discovery | Curated Ollama metadata merged with local discovery. |
| Templates | `templates.rs` | Wired | CLI and HTTP template listing/generation. |
| Vault | `vault/` | Mixed | Filesystem note/parser/service plus data tools; host paths and persistence matter. |
| Search/RAG | `search/`, `rag.rs` | Wired specialized paths | Covered in [RAG](RAG.md). |

## Evaluation

Evaluation types are relevance, faithfulness, completeness, format compliance,
and latency. Format and latency are programmatic. The first three build judge
prompts and call the configured `LLMResource` through `POST /api/v1/eval`.

The HTTP body accepts `input`, `output`, optional `context`, `duration_ms`,
`judge_model`, and a required non-empty `eval_types` array. The response is
`{results, eval_count}` with normalized scores.

Limitations:

- the endpoint evaluates supplied text, not a session ID despite older roadmap wording;
- no `eval/run` tool;
- no AgentSpec post-execution eval hook;
- no durable evaluation repository or experiment/dataset model;
- judge output quality depends on the provider/model and parsed JSON.

## Universe routing

`POST /api/v1/universe/message` constructs a Universe from request agents,
chooses a route using explicit mention, keyword match, round robin, or the
declared `llm_classify` strategy. The synchronous `route()` path falls back to
keyword matching for `llm_classify`.

If the selected `agent_id` exists in the server's process-local agent map, the
handler executes that AgentSpec with `{message}` trigger data. Otherwise it
returns a routing decision with `executed: false`.

The module also defines A2A message types, but it does not provide a durable
message queue, delivery guarantees, persistent universe configuration, or a
full workflow loader.

## Group chat

`POST /api/v1/universe/groupchat` requires a topic and at least two participant
objects with names/personalities. It rotates speakers for `max_rounds` and
calls the configured LLM directly with the accumulated transcript.

This is not agent-to-agent graph execution: participants do not load
AgentSpecs, use tools, isolate memory, or run concurrently. Individual LLM
errors are inserted into the transcript rather than failing the whole handler.

## Intelligence module

| Component | Responsibility |
|---|---|
| `ContextCompiler` | Assemble prioritized, token-budgeted context sections. |
| `ExecutionTracer` | Record richer per-node inputs/outputs/tokens and summaries. |
| `Reflector` | Build/parse LLM reflection over execution evidence. |
| `Suggester` | Build/parse proposed graph improvements. |
| `Playbook` | Match rules and inject guidance. |
| `MemoryFlusher` | Convert selected execution evidence into long-term memory entries. |

These are composable library services. The default GraphRunner does not
automatically run a reflect→suggest→playbook→flush cycle after every execution.
Embedding hosts must choose lifecycle, LLM, storage, privacy, and failure
semantics.

## Energy and cost accounting

The energy module defines immutable `EnergyEvent` records for LLM, MCP, tool,
compute, storage, and DB operations. Rates match provider/model patterns and
produce costs classified as internal infrastructure, external service, or
platform fee. In-memory stores and an `EnergyRecorder` are implemented.

It is not attached globally to provider/tool/resource execution, persisted by
the default server, exposed through HTTP, or suitable as billing truth without
additional wiring, currency/precision rules, idempotency, and audit controls.

## Render

`RenderEngine` converts a supported Markdown subset into self-contained HTML
with embedded CSS and optional chart snippets. It is a custom parser, not a
CommonMark compatibility promise. Treat untrusted generated HTML and chart
content according to the embedding UI's sanitization/CSP requirements.

## Benchmark

Benchmarking is wired through CLI/environment flags and writes local JSONL.
It is diagnostic instrumentation, not the energy meter, OpenTelemetry export,
or a published capacity benchmark. See [Observability](OBSERVABILITY.md).

## Catalog and templates

The catalog is a curated offline-first list of popular Ollama models combined
with locally installed model discovery. It is not a live remote catalog and
can become stale independently of the binary.

Built-in templates are compiled data exposed by CLI and HTTP. Template
availability does not prove required providers/tools are configured.

## Production checklist for adopting a library-only subsystem

1. Identify the owner/host lifecycle.
2. Define persistence and restart behavior.
3. Define tenant/auth/resource boundaries.
4. Attach bounded retries, cancellation, and timeouts.
5. Add structured telemetry and redaction.
6. Add end-to-end tests through the actual host path.
7. Update the component status table when production wiring changes.

## Related documentation

- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [RAG](RAG.md)
- [Memory](MEMORY.md)
- [Observability](OBSERVABILITY.md)
- [Storage and vault](../database/STORAGE.md)
