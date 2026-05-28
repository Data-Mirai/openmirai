# Changelog

All notable changes to datamirai-engine. Consumers: check **Breaking** sections before upgrading.

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
