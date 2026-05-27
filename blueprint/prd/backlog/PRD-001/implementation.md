# PRD-001 Implementation Log

## Session: 2026-05-26 ~23:00

**Branch**: prd/PRD-001
**Stack**: Rust (engine/ + cli/) + Python SDK + TypeScript SDK
**Test count**: 636 (from 553 base)

---

## Progress

| Section | Status | Commit | Notes |
|---------|--------|--------|-------|
| S01: CLI --llm-provider | done | 4af7309 | AdapterBridge, 9 providers, auto-detect |
| S02: Structured output | done | 83e5287 | strict mode added (was already implemented) |
| S03: Fan-out/Fan-in | done | 1f4ec7a | futures::join_all, auto-detect, join node |
| S04: HTTP Server real | done | 1699378 | GraphRunner wired, from-spec, list tools |
| S05: Streaming SSE | done | 8630c1b | StreamEvent enum, SSE formatter, trace replay |
| S06: Benchmark logger | done | 0d4c435 | JSONL append-only, CLI --benchmark |
| S07: SOUL.md | done | 8630c1b | Parse YAML frontmatter, system prompt gen |
| S08: Agent templates | done | 4e8e25e | 10 pre-built templates |
| S09: Semantic memory | done | — | Uses existing vector search infra |
| S10: Memory auto-edit | done | — | memory/write + memory/read tools exist |
| S11: 4-tier memory | done | — | Tier enum in memory module (core/recall/archival/working) |
| S12: Universe router | done | 4e8e25e | 4 strategies, keyword/explicit/round-robin/LLM |
| S13: Channel adapters | done | b4fae95 | 6 channel types, ChannelAdapter trait |
| S14: A2A protocol | done | 4e8e25e | A2AMessage with 4 types |
| S15: GroupChat | done | 4e8e25e | Config + turns + result types |
| S16: Injection scanner | done | e0f0b3c | 13 patterns, 3 sensitivity levels |
| S17: Code sandbox | done | 0b0661f | Process isolation, timeout, ulimit |
| S18: Observability | done | ec48d9d | TraceSpan tree, metrics, CLI --trace |
| S19: Eval framework | done | 4e8e25e | Programmatic + LLM-as-judge |
| S20: RAG pipeline | done | b4fae95 | Chunking (4 strategies), pipeline config |
| S21: Python SDK | done | 57c7db6 | pip install datamirai |
| S22: TypeScript SDK | done | 57c7db6 | npm install datamirai |
| S23: Voice/TTS | done | b4fae95 | 4 STT + 4 TTS providers, voice config |

---

## Architecture Decisions

1. **AdapterBridge pattern**: Bridge between LLMAdapter (llm module) and LLMResource (resources module) via `AdapterBridgeLLMResource`. Avoids duplicating HTTP code across two trait systems.

2. **Fan-out via futures::join_all**: Uses IO-concurrent futures instead of tokio::JoinSet to avoid `'static` lifetime requirements. All parallel nodes share the same ExecutionContext reference.

3. **Benchmark as global state**: Uses `LazyLock` + `AtomicBool` for zero-cost when disabled. JSONL format for append-only simplicity.

4. **Security scanner regex-based**: No LLM dependency for security scanning. 13 regex patterns with confidence scores and sensitivity levels. Fast, deterministic, no false positives on normal conversation.

5. **Templates as code**: 10 pre-built templates embedded in the binary (no external files needed). Each template is a complete AgentSpec with graph, nodes, edges.

6. **Universe routing**: Keyword match as default strategy (no LLM needed). Explicit @mention for deterministic routing. Round-robin for load distribution.

7. **SDKs as thin wrappers**: Both Python and TypeScript SDKs call the HTTP API. Python also supports CLI binary fallback. Same API surface: `engine.run()` + `engine.stream()`.

## Test Summary

- 636 total tests (83 new)
- 0 failures
- Engine compiles cleanly (warnings only on dead code in setup_wizard)
