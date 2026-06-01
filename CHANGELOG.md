# Changelog

All notable changes to openmirai-engine. Consumers: check **Breaking** sections before upgrading.

---

## v0.6.2 (2026-06-01)

The visual editor (`mirai edit`) becomes a real local sandbox — design, run and debug agents before production. No breaking changes.

### Added
- **Clean canvas layout (PRD-016).** Nodes are laid out left→right by longest-path level, each column centered vertically, and the whole graph is centered and scaled to fit the viewport on load. Edges are smooth horizontal curves; a **Fit** button re-centers after dragging or resize.
- **Typed run inputs (PRD-016).** The Run dialog is now a form generated from the agent's declared `inputs` (label · type · required · placeholder) with required-field validation and per-type coercion (number / boolean / object|array→JSON). Agents with no declared inputs still get a free JSON field. No more guessing what to send.

### Changed
- **The editor runs agents for real — zero mock (PRD-016).** `mirai edit` resolves a real provider (Ollama / your configured default) and **rejects `--provider mock`**. The sandbox executes the agent against a real LLM so you debug against real output, not fake responses. (MockLLM stays confined to unit tests.)

---

## v0.6.1 (2026-05-31)

No breaking changes. The agent YAML spec, runtime, and API are backward-compatible.

### Added
- **Configurable default model + guided preflight (PRD-014).** Pick your default provider/model from a curated Ollama catalog (merged with your installed models) — persisted to `~/.openmirai/config.toml`. Before any run, a preflight check turns cryptic failures ("HTTP 404: model not found") into actionable guidance: start Ollama, pull the missing model (with consent + streamed progress), or set an API key. New commands: `mirai doctor` (environment check) and `mirai models` (`list` / `pull <name>` / `use <name>`). **API keys are never written to config — env/flag only.**
- **Visual editor: run, debug, configure (PRD-013/014/015).** `mirai edit <agent.yaml>` gained: step-by-step run visualization on the canvas (nodes light up running → completed/error); a **pre-run gate** that surfaces missing config with a one-click model download; a **Settings** panel to browse the catalog and set the default model; and a **Run result drawer** (PRD-015) that coexists with the graph and shows the final result plus per-node outputs (collapsible JSON) and errors.
- **Run output in the stream (PRD-015).** SSE `node.completed` now carries the node's real output map and `graph.completed` carries the final state snapshot, so clients can show what the agent actually produced. The replay path (`trace_to_events`) populates the same outputs from final state.

### Changed
- **MSRV is now Rust 1.80** (declared via `rust-version`), and the repo pins a toolchain so `cargo build` from source guides you to the right compiler instead of failing on a lockfile-version mismatch. Added the `toml` dependency for user config.
- **Internal cleanup (PRD-011 + DRY pass).** Unified duplicated helpers (`field()`, `value_type_label`, Ollama URL/client/status, the editor's SSE parser and LLM factory) and collapsed boilerplate — net negative lines, zero behavior change, all tests green.

---

## v0.6.0 (2026-05-31)

### Changed
- **Rebrand: Data Mirai Engine → OpenMirai.** The open-source engine is now **OpenMirai** — positioning around running AI agents wherever you are, with no cloud lock-in. Added community on-ramp: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, issue templates, and PR template.

### Breaking
Identifiers were renamed. The agent YAML format, the runtime, and the API are **unchanged** — only package/crate names move. Update your imports and dependency declarations:

| Surface | Old | New |
|---------|-----|-----|
| Rust crate (engine) | `datamirai-engine` | `openmirai-engine` |
| Rust crate (cli) | `datamirai-cli` | `openmirai-cli` |
| Rust lib import | `datamirai_engine` | `openmirai_engine` |
| Python package | `datamirai` (`pip install datamirai`) | `openmirai` (`pip install openmirai`) |
| TypeScript package | `datamirai` | `openmirai` |
| Repository | `Gabo-TheCreator/data-mirai-engine` | `Gabo-TheCreator/openmirai` |

### Unchanged (no action needed)
- CLI binary and command: still `mirai`.
- Environment variables: still `MIRAI_API_KEY`, `MIRAI_SCRATCH_DIR`, `MIRAI_*`.
- Local data directory: still `~/.datamirai/`.
- Company name and domain: still `Data Mirai` / `datamirai.com`.
- Agent YAML spec: fully backward-compatible — all v0.5.x agents run unchanged.

### Consumer action
- **Rust**: rename the dependency in `Cargo.toml` and update `use datamirai_engine::…` → `use openmirai_engine::…`.
- **Python**: `pip install openmirai` and update `from datamirai import …` → `from openmirai import …`.
- **TypeScript**: update the `datamirai` dependency to `openmirai`.
- **Binary users**: no change — keep using `mirai`.

---

## v0.5.0 (2026-05-28)

### Added
- **Multimodal file input (PRD-009)**: `ai/transcribe` and `ai/llm_call` now accept real audio, video, and image files. The engine reads the file, base64-encodes it, and sends it as native multimodal content to the LLM provider.
  - `ai/transcribe`: sends audio/video/image as inline data (replaces text-only placeholder)
  - `ai/llm_call`: new `media_path` config field for attaching files alongside text prompts
  - Supported formats: audio (.m4a, .mp3, .wav, .ogg, .flac), video (.mov, .mp4, .webm), image (.png, .jpg, .webp, .gif)
  - Provider support: Gemini (all types), Claude (images), OpenAI/Groq (images), Ollama (images)
  - Validation: file existence, size limit (20MB), MIME type + provider compatibility
- **File-first data layer (PRD-010)**: Files are first-class citizens in the graph.
  - `FileRef`: standardized JSON object (`{_type, path, mime_type, size_bytes}`) that flows through `data_map` like any other value
  - `FieldType::File`: new field type for tool specs that validates FileRef objects
  - `resolve_file_input()`: tools auto-resolve both string paths and FileRef objects (backward compatible)
  - `system/bash` `output_files` config: declare files the command generates — engine produces FileRef outputs
  - `MIRAI_SCRATCH_DIR`: per-execution temp directory injected as env var to bash tools
  - `create_file_ref()`: utility to construct FileRef from any file path
- **`system/bash` environment variables (PRD-009)**: `data_map` values now available as `MIRAI_*` env vars in bash commands.
- **`LLMResource::provider_name()`**: tools can query which LLM provider is active.
- New examples: `audio-transcription.yaml`, `image-analysis.yaml`
- All examples updated with `inputs:` contract declarations (PRD-004 best practice)

### Changed
- `Message` struct: new optional `media` field for multimodal content
- All 7 LLM adapters (Gemini, Claude, OpenAI, Groq, OpenRouter, NVIDIA, Ollama) handle media in their native format
- `ai/transcribe` default prompt: now produces literal, faithful transcription (including filler words)

### Consumer action
- **No breaking changes** — all v0.4.x agents work unchanged. `media`, `output_files`, and `FileRef` are all opt-in.
- Update binary. New capabilities available via new config fields.
- Recommended: add `inputs:` section to your agent YAMLs for clear host contracts.

---

## v0.4.5 (2026-05-28)

### Added
- **`mirai tools` CLI command**: catalog of all 49 tools with inputs, outputs, and config.
  - `mirai tools` → lists all tools grouped by category
  - `mirai tools ai/claude_code` → shows full contract (inputs, outputs, config with types and descriptions)
- Solves the "how do I know what fields to use in data_map?" problem. Each tool declares its contract.

### Consumer action
- None — new feature, no breaking changes. Update binary to use `mirai tools`.

---

## v0.4.4 (2026-05-28)

### Added
- **`ai/claude_code` tool**: Native Claude Code CLI integration. Spawns `claude -p` as child process, sends prompt via stdin, reads response from stdout. Zero API keys — uses user's existing Claude subscription (Max/Pro). Config: timeout_ms, max_tokens, system_prompt, model, cli_path.
- Auto-detects `claude` binary in $PATH. Clear error if not installed.
- 49 tools total (was 48).

### Consumer action
- None — new tool, no breaking changes. Use `tool_type: ai/claude_code` in agent YAML.

---

## v0.4.3 (2026-05-28)

### Fixed
- **MCP client hang**: `notifications/initialized` was blocking on `read_line` for 600s because `transport.send()` always waits for a response. Notifications are fire-and-forget per JSON-RPC spec. Added `Transport::send_notification()` that writes without reading. Unblocks ALL MCP tool usage.

### Consumer action
- None — no breaking changes. Just update the binary.

---

## v0.4.2 (2026-05-27)

### Changed
- **Agent specs are YAML-only**. `AgentSpec::from_file()` and Python SDK `Agent.from_file()` now reject `.json` files. Rename your agent files to `.yaml`.
- **Full-word comparison operators**: `equals`, `not_equals`, `greater_than`, `less_than`, `greater_or_equal`, `less_or_equal`, `contains`, `in`. Short forms (`eq`, `gt`, etc.) still work.

### Consumer action
- Rename any `.json` agent specs to `.yaml`
- Optionally update edge conditions to use readable operators

---

## v0.4.1 (2026-05-27)

### Fixed
- **Critical: `to_graph()` dropped edge conditions**. All conditional edges were silently treated as unconditional (fan-out instead of exclusive routing). Now `AgentEdgeSpec.condition` is strongly typed (`Option<EdgeCondition>`) and preserved through conversion.
- `ComparisonOp` now accepts both PascalCase (`Eq`) and lowercase (`eq`) in YAML.

### Added
- 8 new tests covering all condition operators (equals, neq, gt, lt, gte, lte, contains, in)

### Consumer action
- None — this is a bug fix. Conditional routing now works correctly.

---

## v0.4.0 (2026-05-27)

### Breaking
- **API routes**: `/api/*` → `/api/v1/*` (health and version unchanged)
- **Renamed**: `SimpleExecutionContext` → `DefaultExecutionContext`
- **Renamed**: `resources::` → `adapters::`
- **Renamed**: `server::app::serve` → `server::serve`
- **Removed**: `voice.rs`, `channels.rs` (dead code)

### Added
- **Auth**: `X-API-Key` header middleware. Set `MIRAI_API_KEY` env var or `--api-key` flag.
- **Graceful shutdown**: SIGTERM + Ctrl+C handled cleanly.
- **Session eviction**: FIFO with 10K cap (prevents memory leak).
- **Request timeout**: 300s configurable on execution endpoints.
- **Feature flags**: `server` feature is optional (`cargo build --no-default-features` for core-only).
- **ValueType**: Unified type enum replacing both `InputType` and `FieldType`.
- **well_known.rs**: Named constants for all magic strings in the runner.
- **4 examples**: hello-world, conditional-routing, data-pipeline, python-quickstart.
- **README**: Rewritten in English with working quick start.

### Refactored
- `runner.rs` (3,305 lines) → 6 files (max 1,655)
- `app.rs` (1,593 lines) → 5 files (max 775)
- `data.rs` (2,400 lines) → 12 files (1 per tool)
- `filesystem.rs` (1,798 lines) → 13 files (1 per tool)
- `run_from()`: 640 → 116 lines + 10 descriptive submethods
- `map_reqwest_error()`: 4 copies → 1 in `llm/error.rs`
- Silent failures: 12 critical → 0

### Consumer action
- Search and replace `/api/` → `/api/v1/` in all client code
- Replace `SimpleExecutionContext` → `DefaultExecutionContext`
- Replace `resources::` → `adapters::`
- Replace `server::app::serve` → `server::serve`
- Set `MIRAI_API_KEY` for production deployments

---

## v0.3.1 (2026-05-27)

### Added
- PRD-004: Agent input/output contracts with typed validation
- PRD-005: CLI eval + rag commands with real LLM

---

## v0.3.0 (2026-05-27)

### Added
- PRD-001: Base engine — 23 features, 47 tools, 636 tests
- PRD-004: Agent Contract — Input/Output Schema + strict runner
