# BUILTIN_TOOLS.md

Implementer reference for OpenMirai's built-in tool library (`engine/src/tools/builtin/`).

This document describes the **implementation contracts, boundaries, and security
expectations** of the built-in tools. It is written for contributors who extend
the tool library, audit its behavior, or wire it into a new host.

> **Audience split.** [`USAGE.md`](../../USAGE.md) is the user-facing tool
> catalog — what each tool does and how to call it from an agent YAML. This
> document is the implementer's view — the traits a tool must satisfy, the
> resources it may touch, the failure modes it must produce, and the boundaries
> it must not cross. For the underlying trait/pattern definitions, see
> [`PRIMITIVES.md`](./PRIMITIVES.md).

Engine version at time of writing: **0.7.0**.

---

## 1. Registry overview

All built-in tools live under `engine/src/tools/builtin/` and are wired into a
[`ToolRegistry`](../../engine/src/tools/registry.rs) by
`register_all_builtin_tools()` in `engine/src/tools/builtin/mod.rs`:

```rust
pub fn register_all_builtin_tools(registry: &mut ToolRegistry) {
    logic::register_logic_tools(registry);
    ai::register_ai_tools(registry);
    data::register_data_tools(registry);
    filesystem::register_filesystem_tools(registry);
    system::register_system_tools(registry);
    git::register_git_tools(registry);
    output::register_output_tools(registry);
    agent::register_agent_tools(registry);
    mcp::register_mcp_tools(registry);
    trigger::register_trigger_tools(registry);
    state::register_state_tools(registry);
}
```

Each family exposes a `register_*_tools()` function. There are **52 registered
tools** across 11 families (the count is asserted in
`builtin::tests::register_all_builtin_tools_adds_all`):

| Family | Module | Count | `tool_type` prefix |
|--------|--------|------:|--------------------|
| Logic | `logic.rs` | 7 | `logic/` |
| AI | `ai.rs` | 6 | `ai/` |
| Data | `data/` | 11 | `data/` |
| Filesystem | `filesystem/` | 12 | `filesystem/` |
| System | `system.rs` | 3 | `system/` |
| Git | `git.rs` | 4 | `git/` |
| Output | `output.rs` | 1 | `output/` |
| Agent | `agent.rs` | 1 | `agent/` |
| MCP | `mcp.rs` | 1 | `mcp/` |
| Trigger | `trigger.rs` | 5 | `trigger/` |
| State | `state.rs` | 1 | `state/` |

### Tool type naming

A `tool_type` is a category-qualified string, `"<category>/<name>"` (e.g.
`"ai/llm_call"`, `"filesystem/read_file"`). The category prefix matches the
`category` field on the [`ToolSpec`](#3-toolspec-and-schema-conventions) and is
used by the editor and `list_tools()` for grouping.

### Aliases (backward compatibility)

The registry supports legacy aliases that resolve to a canonical `tool_type`:

```rust
registry.register_alias("fs/read_file", "filesystem/read_file");
registry.register_alias("mcp/mcp_call", "mcp/call");
```

`ToolRegistry::get()` falls back to the alias map on a direct miss. Aliases are
**not** returned by `list_tools()`. Current aliases (`fs/*` → `filesystem/*`,
`mcp/mcp_call` → `mcp/call`) are marked *remove in v0.3.0* in the source and
should not be relied on for new work. Tests: `filesystem::tests::legacy_fs_aliases_resolve`,
`mcp::tests::legacy_mcp_alias_resolves`.

---

## 2. Tool implementation contract

Three traits in `engine/src/tools/registry.rs` define the contract. See
[`PRIMITIVES.md` → Tool + ToolFactory Trait Pattern](./PRIMITIVES.md).

### `Tool` — the executable instance

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}
```

- **`inputs`** — runtime values resolved from the node's `data_map` (edges from
  upstream nodes). Owned by the call; the tool may consume them.
- **`config`** — static, node-level configuration from the agent YAML's
  `config:` block. Borrowed.
- **`context`** — the [`ExecutionContext`](#4-executioncontext-requirements);
  the only legitimate channel to DB/LLM/storage/auth resources.
- **Return** — a `HashMap<String, Value>` whose keys SHOULD match the
  `outputs` declared in the tool's `ToolSpec`. These become available to
  downstream nodes.

A `Tool` instance is **stateless and cheap** — all built-ins are unit structs
(`pub struct BashTool;`). Per-call state lives in locals; cross-call state must
go through a resource on the `ExecutionContext`.

### `ToolFactory` — the spec owner and constructor

```rust
pub trait ToolFactory: Send + Sync {
    fn create(&self) -> Arc<dyn Tool>;
    fn spec(&self) -> &ToolSpec;
}
```

Each `tool_type` has exactly one factory. The factory owns the `ToolSpec` and
stamps out a fresh `Arc<dyn Tool>` per node execution via `create()`.

### The boilerplate macro

Every family defines a local declarative macro (`ai_tool!`, `logic_tool!`,
`fs_tool!`, `system_tool!`, `git_tool!`, `data_tool!`, `output_tool!`,
`agent_tool!`, `mcp_tool!`, `trigger_tool!`) that generates the unit struct, the
factory struct, `new()`/`Default`, and the `ToolFactory` impl from a spec
literal. You write only the `#[async_trait] impl Tool` block. `state/memory` is
the one tool written longhand (no macro) because it documents the
`__memory_spec` injection in detail.

### How the runner drives a tool

`RegistryExecutor::execute()` (in `registry.rs`) is the bridge between the graph
runner and a tool. For each node it:

1. Looks up the factory by `node.tool_type`; missing → `ToolError::NotFound`.
2. **Validates inputs** against `ToolSpec.inputs` via `validate_node_inputs()`
   (PRD-004 Capa 2). Missing required fields or type mismatches →
   `ToolError::ExecutionFailed`. Optional fields with a declared `default` are
   injected here.
3. Calls `factory.create()` for a fresh instance.
4. Runs `execute()` inside `catch_unwind` — **a tool panic becomes a
   `ToolError::ExecutionFailed` ("tool panic: …"), never a process crash.**

Implications for implementers:
- You can rely on required inputs being present and correctly typed *if* you
  declared them in the spec. Inputs you read but did not declare are **not**
  validated — read them defensively.
- A panic is caught, but is a bug, not an error-handling strategy. Return
  `Err(ToolError::ExecutionFailed { … })` for expected failures.

---

## 3. ToolSpec and schema conventions

`ToolSpec` and `ToolField` are defined in `engine/src/tools/base.rs`.

```rust
pub struct ToolSpec {
    pub tool_type: String,   // "ai/llm_call"
    pub name: String,        // "LLM Call" (human label)
    pub description: String,
    pub version: String,     // "1.0.0" for all built-ins
    pub category: String,    // "ai"
    pub inputs: Vec<ToolField>,
    pub outputs: Vec<ToolField>,
    pub config_fields: Vec<ToolField>,
}

pub struct ToolField {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    pub description: Option<String>,
    pub default: Option<Value>,
}
```

Build fields with the canonical `field()` constructor — never hand-roll a
`ToolField`:

```rust
field("prompt", FieldType::String, /* required */ false, "Instruction for the LLM")
```

### FieldType

`FieldType` is a closed enum (no free-form type strings): `String`, `Number`,
`Boolean`, `Array`, `Object`, `Integer`, `File`. `FieldType::matches(&Value)`
backs input validation; `FieldType::File` matches a file-ref object
(`crate::llm::media::is_file_ref` — `{ "_type": "file_ref", "path": … }`,
PRD-010).

### Conventions observed across the library

- **`inputs` vs `config`.** Inputs are per-run, data-flow values; config is
  static node setup. Many tools resolve a value from **input first, then config
  as fallback** (e.g. `ai/llm_call` prompt, `agent/run_agent` `agent_id`,
  `data/rag_search` query/documents). Document this in the field description.
- **Optional + default.** Declare optional fields with `required = false`. If
  you set `ToolField.default`, the runner injects it on absence — so your
  `execute()` can also rely on it. Most built-ins instead apply defaults inline
  with `.unwrap_or(...)`, which is fine but means the default is invisible to
  the editor; prefer declared defaults for editor-facing values.
- **Output keys must match declared outputs.** Downstream `data_map` wiring
  references output names. Keep them in sync with the spec.
- **`__`-prefixed keys are internal carriers**, not part of the public schema.
  They are injected by the host/runner or passed between phases and are never
  declared in the spec. Examples: `__memory_spec`, `__agent_id`,
  `__mcp_servers`, `__agent_call_chain__`, `__memory_write`, `__user_media`.
  Treat them as a private side-channel.
- **Version** is `"1.0.0"` for every built-in today; bump it if you make a
  breaking schema change to a single tool.

`ToolSpec` and `ToolField` are `Serialize`/`Deserialize`; round-trip tests live
in `base.rs::tests`.

---

## 4. ExecutionContext requirements

`ExecutionContext` (`engine/src/core/context.rs`) is the **only** sanctioned way
for a tool to reach shared resources. Tools receive `&dyn ExecutionContext`.

```rust
pub trait ExecutionContext: Send + Sync {
    fn db(&self) -> Option<&dyn DBResource>;        // optional
    fn llm(&self) -> &dyn LLMResource;              // always present
    fn storage(&self) -> Option<&dyn StorageResource>;
    fn vector(&self) -> Option<&dyn VectorResource>;
    fn auth(&self) -> &AuthContext;
    fn session_id(&self) -> &str;
    fn node_id(&self) -> Option<&str>;
    fn system_prompt(&self) -> Option<&str>;
    fn scratch_dir(&self) -> Option<&str> { None }  // PRD-010
}
```

Rules for tools:

- **`llm()` is the only always-available resource.** `db()`, `storage()`,
  `vector()` return `Option`; a tool that needs one MUST handle `None` with a
  clear `ToolError::ExecutionFailed`, not `.unwrap()`. See
  `data::tests::db_read_no_db_fails`, `storage_read_no_storage_fails`.
- **Resources return `ResourceError`**, which you map into
  `ToolError::ExecutionFailed`. `ResourceError` carries `PermissionDenied`,
  `NotFound`, `Database`, `Llm`, `Storage`, `Other`.
- **`auth()`** exposes `AuthContext { user_id, role, universe_id,
  environment_id }` with `Role ∈ {Owner, Admin, Editor, Viewer}`. RBAC is
  enforced at the resource/host layer (see Community 27 auth tests), not
  re-implemented per tool; read `auth()` only when a tool needs to scope by
  user/universe.
- **`scratch_dir()`** is a per-execution temp dir (PRD-010), cleaned up after
  the run. `system/bash` exposes it to subprocesses as `MIRAI_SCRATCH_DIR` and
  searches it for declared `output_files`. Use it for transient files — never
  write durable state there.
- **`system_prompt()`** is the agent-level system prompt; `ai/llm_call`
  concatenates it ahead of the node-level `system_prompt` config.

For testing, `InMemoryContext` (`#[cfg(test)]`, in `context.rs`) provides a
stub LLM and no DB/storage/vector. Data-tool tests use a richer `TestContext`
in `data/mod.rs` with stub `DBResource`/`StorageResource`.

---

## 5. Error handling conventions

```rust
pub enum ToolError {        // engine/src/core/runner/types.rs
    NotFound { tool_type: String },
    ExecutionFailed { tool_type: String, message: String },
}
```

- Tools return **only** `ExecutionFailed` from `execute()`; `NotFound` is the
  registry's to raise. Always set `tool_type` to your own `tool_type` so the
  runner can attribute the failure to the right node.
- **Validate-then-act.** Required inputs missing → `ExecutionFailed` with a
  message naming the field (`"input 'text' is required"`). This mirrors the
  runner's own pre-validation and covers undeclared inputs you read directly.
- **Distinguish "absent resource" from "operation failed."** A missing DB is a
  config error (`"no database resource configured"`); a failed query is an
  execution error (mapped from `ResourceError`).
- **Soft-fail vs hard-fail.** Some tools return a *successful* result object
  with a `success: false` / `error: "…"` field rather than an `Err` — used when
  the failure is a normal, inspectable outcome a graph may branch on. `mcp/call`
  does this for "no servers configured" / "server not found". Reserve this
  pattern for outcomes the graph is expected to route on; otherwise return
  `Err`.
- Never `panic!`/`unwrap()` on external input. The `catch_unwind` net exists for
  bugs, not control flow.

---

## 6. Security & sandbox expectations by family

Risk rises sharply from logic/output (pure) → data (mediated) → filesystem →
system/git (direct host access). The mediation boundary matters: see
[§13 Do not bypass these boundaries](#13-do-not-bypass-these-boundaries).

| Family | Side effects | Mediated by `ExecutionContext`? | Notes |
|--------|--------------|----------------------------------|-------|
| `logic/*` | None (pure / sleep) | n/a | Deterministic, safe. |
| `output/*` | None | n/a | Formatting only. |
| `state/*` | Buffers memory writes | Indirect (host persists) | Honors declared keys + persist mode. |
| `ai/*` | LLM network calls + media file reads + (claude_code) subprocess | `llm()` for `llm_call`/`embeddings`/`transcribe`; **not** for `claude_code` | Prompt-injection scanner on `llm_call`. |
| `data/*` | DB / storage / vector / **outbound HTTP** | Yes (db/storage/vector); `web_scrape` does direct HTTP | Access scoped by the injected resource. |
| `agent/*` | Sub-agent execution | Host-driven (placeholder in-tool) | Depth + cycle guards. |
| `mcp/*` | External MCP server calls | Via `MCPManager` from injected config | Network/process per transport. |
| `trigger/*` | None at execute time | n/a | Entry-point metadata nodes. |
| `filesystem/*` | **Direct host FS read/write/delete** | **No** — raw `std::fs` | No path jail. See §13. |
| `system/*` | **Shell / process / code exec** | No | `bash` blocklist; `sandbox_exec` temp-dir isolation. |

Headline rule: **filesystem and system tools touch the host directly and are
NOT sandboxed by the `ExecutionContext`.** Only `system/sandbox_exec` provides
real isolation, and only process-level (§9).

---

## 7. AI tools — behavior and LLM resource dependencies

`engine/src/tools/builtin/ai.rs` registers four tools.

| `tool_type` | Resource dependency | Notes |
|-------------|---------------------|-------|
| `ai/llm_call` | `context.llm()` | Core LLM node. |
| `ai/embeddings` | `context.llm().embed()` | Returns vector + dimensions. |
| `ai/transcribe` | `context.llm()` + media file read | Multimodal audio. |
| `ai/claude_code` | **Local `claude` CLI subprocess** | No `ExecutionContext` LLM; uses host Claude Code subscription. |

### `ai/llm_call`

The most feature-dense tool. Behavior:

- Resolves `prompt` from input then config; empty → error.
- **Prompt-injection scan (on by default).** Before calling the LLM it
  recursively collects every string in `inputs`, joins them, and runs
  `crate::security::scan`. On a block it returns `ExecutionFailed` with the
  threat type and confidence. Toggle via config `security_scan` (bool),
  `security_sensitivity` (`low|medium|high`), `security_block` (bool). See
  Community 32 (`security`) injection-detection tests. **Do not disable the
  scanner on paths that feed untrusted/tool-fetched content into prompts.**
- Builds a `=== Session Context ===` block from all non-`prompt` inputs,
  proportionally truncated to `max_context_length` chars (default 12000,
  per-key floor 200). Tests: `format_session_context_*`.
- **Structured output.** If `output_schema` (JSON string or object) is set, it
  enriches the prompt with a mandatory-JSON instruction, validates the response
  (required fields, types, enums), and retries up to `max_retries` (default 2)
  with error feedback. `output_schema_strict` (default **true**) makes a final
  validation failure a hard error; non-strict returns best-effort with
  `schema_valid: false`. Tests: `validate_response_*`, `extract_json_*`,
  `build_retry_prompt_*`.
- **Multimodal.** `media_path` (input or config; string path or file-ref) is
  read via `crate::llm::media::read_media_file` for the provider and attached to
  the user turn.
- Outputs: `response`, `model`, `tokens_input`, `tokens_output`,
  `structured_output`, `schema_valid`.

### `ai/embeddings` / `ai/transcribe`

`embeddings` calls `llm().embed(text, model)`; `transcribe` reads an audio file,
sends it as multimodal content with a verbatim-transcription system prompt at
temperature 0. Both map `ResourceError` → `ExecutionFailed`.

### `ai/claude_code`

Spawns the locally installed `claude -p` CLI (auto-detected in `$PATH` or via
`cli_path`), pipes the assembled prompt to stdin, enforces `timeout_ms`
(default 60s), and returns stdout. **This bypasses the `ExecutionContext` LLM
abstraction entirely** — it is a host subprocess using the user's Claude
subscription, not an API key. Treat it with the same caution as `system/bash`:
arbitrary local execution under the engine's user. Token counts are rough
length/4 estimates.

---

## 8. Data tools — behavior and storage dependencies

`engine/src/tools/builtin/data/` registers 11 tools; each lives in its own file
and shares the `data_tool!` macro from `data/mod.rs`.

| `tool_type` | Depends on | Behavior |
|-------------|-----------|----------|
| `data/db_read` | `context.db()` | Read rows; `mode = one\|all`; returns `rows`/`row` + `count`. |
| `data/db_write` | `context.db()` | Insert/upsert; auto table creation; parses string JSON data. |
| `data/entity_query` | `context.db()` | Query entities by type + filters. |
| `data/entity_upsert` | `context.db()` | Create/update entity; returns `id` + `action`. |
| `data/storage_read` | `context.storage()` | Read blob; `mode = content\|presign`; `found` flag, `NotFound` → soft. |
| `data/storage_write` | `context.storage()` | Write blob; returns `bytes_written`. |
| `data/vault_read` | (placeholder) | Knowledge-vault read; returns empty until a FS backend is wired. |
| `data/vault_write` | (placeholder) | Acknowledges write; returns path + `written`. |
| `data/rag_search` | `context.llm().embed()` | Chunk → embed → cosine top-K. Real embeddings. |
| `data/html_to_markdown` | none | Pure transform; **strips `<script>`**. Tests: `html_to_markdown_strips_script`. |
| `data/web_scrape` | **direct HTTP** | Search (Google/Bing/DuckDuckGo) or fetch URL. |

Boundaries:

- DB/storage/vector access is **always mediated** by the resource the host
  injected into the `ExecutionContext`. A tool cannot reach a database the host
  did not provide; missing resource → error. The resource implementation (not
  the tool) enforces tenancy/RBAC and SQL safety.
- `data/web_scrape` is the exception: it performs **outbound HTTP directly**
  (not through a context resource). It applies a per-session `SessionFingerprint`
  (deterministic stealth headers), URL normalization (strips tracking params),
  request jitter, and a TTL response cache. Tests: `fingerprint_*`,
  `url_normalization_strips_tracking_params`, `serp_extraction_*`,
  `jitter_*`, `cache_*`. Because it fetches arbitrary remote content, its output
  is **untrusted** — anything downstream that feeds it into an LLM should keep
  the `ai/llm_call` injection scanner enabled.
- `vault_read`/`vault_write` are placeholders today (no real persistence);
  document the gap if you build on them.

---

## 9. Filesystem & system tools — operational risk

### Filesystem (`engine/src/tools/builtin/filesystem/`, 12 tools)

`filesystem/read_file`, `write_file`, `edit_file`, `glob_files`, `grep_files`,
`list_dir`, `tree`, `copy`, `move`, `delete`, `mkdir`, `file_info`.

- These use **raw `std::fs`** on the host filesystem. There is **no path jail
  and no `ExecutionContext` mediation** — they operate anywhere the engine's OS
  user can reach. `read_file`/`delete` call `fs::canonicalize` (which resolves
  symlinks and rejects nonexistent paths) but do **not** confine the result to
  any root.
- `filesystem/delete` is **recursive for directories** (`fs::remove_dir_all`).
  `filesystem/write_file` creates missing parent directories. `filesystem/edit_file`
  refuses ambiguous replacements unless `replace_all` is set
  (`edit_file_ambiguous_without_replace_all`).
- Path confinement, if required, is the **host's** responsibility (run the
  engine as a least-privileged user, containerize, or chroot). See §13.

### System (`engine/src/tools/builtin/system.rs`, 3 tools)

| `tool_type` | Isolation | Risk |
|-------------|-----------|------|
| `system/bash` | Blocklist only | Runs `sh -c <command>` on the host. |
| `system/process_list` | none | Runs `ps aux`. |
| `system/sandbox_exec` | Temp-dir + timeout + best-effort ulimit | Lowest-risk code exec. |

- **`system/bash`** executes the command through `sh -c` with full host
  privileges. It blocks a small set of catastrophic patterns (`rm -rf /`,
  fork bomb, `mkfs`, `dd of=/dev/…`, `> /dev/sd*`, `curl|sh`, `wget|sh` — see
  `blocked_patterns()`), enforces a timeout (default 120s, max 600s), validates
  `cwd`, truncates output at 30 000 chars, injects non-`command` inputs as
  `MIRAI_<key>` env vars, and exposes `MIRAI_SCRATCH_DIR`. **The blocklist is a
  guardrail against accidents, not a security boundary** — it is trivially
  bypassable and must not be treated as a sandbox. Tests: `bash_blocks_*`,
  `bash_timeout`, `bash_with_cwd`.
- **`system/sandbox_exec`** is the only tool offering real isolation, and only
  *process-level* (`engine/src/sandbox.rs`, Phase 1): a fresh temp working
  directory, timeout-kill, a restricted environment, and best-effort memory
  limits via `ulimit` on supported OSes. `network_access` defaults to **false**.
  Supports `python`/`javascript`/`bash`. It is **not** a kernel sandbox (no
  namespaces/seccomp) — do not rely on it to contain hostile code.

---

## 10. State & output tools — effect on execution state

### `state/memory` (`state.rs`)

Persists declared keys across agent cycles/executions (PRD-008). Mechanics:

- The host injects `__memory_spec` (`{ persist, keys }`) and `__agent_id` into
  `config` before execution (done by `run_agent_spec`).
- `persist: none` → **no-op**, returns empty `persisted_keys`.
- Otherwise it filters `inputs` to **only keys declared in `graph.memory`**
  (undeclared keys are dropped with a warning), echoes the persisted key list,
  and stashes the filtered map under the internal output key `__memory_write`.
- **The tool does not persist anything itself.** It stages data; the host
  (server/scheduler) reads `__memory_write` from state after execution and
  writes it to the memory store. Tests: `persist_none_is_noop`,
  `persist_cycle_filters_declared_keys`, `persist_execution_writes_data`.

This declared-keys filter is a deliberate boundary: an agent cannot persist
arbitrary state, only what its spec opted into.

### `output/response` (`output.rs`)

Terminal node that formats the final result. `format ∈ {text, json, markdown
(default), bullets}`, with optional `${key}` `template` substitution and
`title`. `extract_content` digs the meaningful string out of common keys
(`result`, `output`, `text`, `content`, `response`, `summary`). Pure formatting,
no side effects. Tests: `response_json_format`, `response_bullets_format`,
`response_template`.

---

## 11. Agent, MCP & trigger tools — composition boundaries

### `agent/run_agent` (`agent.rs`)

Runs another agent as a synchronous sub-task. The tool itself is a **placeholder
that enforces the composition guardrails**; the real child execution happens in
the host/app layer, which reads `_input_data` and `_call_chain` from the output.

- `agent_id` resolves input → config; required.
- **Nesting cap:** `MAX_NESTING_DEPTH = 3`. Reads `__agent_call_chain__` from
  inputs; exceeding the cap → error.
- **Cycle guard:** an `agent_id` already in the chain → "Circular agent
  reference" error.
- Outputs the extended chain so the host can recurse safely. Tests:
  `run_agent_nesting_depth_exceeded`, `run_agent_circular_reference`,
  `run_agent_missing_id_fails`.

When you wire the real sub-agent runner, **preserve these two invariants** —
they are the only loop/recursion protection for agent composition.

### `mcp/call` (`mcp.rs`)

Invokes a tool on an external Model Context Protocol server.

- Reads `server_name`/`tool_name` from config and `arguments` from input/config.
- The host injects available servers as `__mcp_servers` (`Vec<AgentMcpServerSpec>`).
  No servers configured, or server name not in the spec → a **soft failure**
  (`success: false`, descriptive `error`), not an `Err`.
- Builds an `MCPManager`, calls the tool, extracts text content from the MCP
  `content` array for convenience, and **always `close_all()`s** connections
  (success or failure). Transport is per-server (`stdio`/HTTP). Tests:
  `mcp_call_without_servers_config_returns_descriptive_error`,
  `mcp_call_with_nonexistent_server_returns_not_found`.

MCP is the sanctioned extension boundary: it lets agents call out to external
tool servers without adding Rust code to the engine. Network/process exposure is
inherited from the configured transport.

### `trigger/*` (`trigger.rs`, 5 tools)

`trigger/webhook`, `trigger/manual`, `trigger/schedule`, `trigger/event`,
`trigger/heartbeat`. These are **graph entry points** — they carry
trigger-config metadata and produce a starting payload; they perform no host
side effects at `execute()` time. The actual scheduling/eventing lives in the
host (server/scheduler).

---

## 12. How to add a new built-in tool

1. **Pick a family** and open its module (`engine/src/tools/builtin/<family>.rs`,
   or a new file under `data/` or `filesystem/`).
2. **Declare the tool** with the family's macro — provide `tool_type`, `name`,
   `description`, and `inputs`/`outputs`/`config_fields` built with `field()`.
   Keep `tool_type` as `"<category>/<name>"` matching the family category.
3. **Implement `#[async_trait] impl Tool`.** Validate required/undeclared inputs
   up front, reach resources only through `context`, map `ResourceError` →
   `ToolError::ExecutionFailed { tool_type: "<your type>", … }`, and return a
   `HashMap` whose keys match your declared `outputs`.
4. **Register it** in the family's `register_*_tools()` and bump that family's
   count assertion test. If you rename an existing tool, add a
   `register_alias(old, new)` for one release.
5. **For a new family**, add `pub mod <family>;` and a `register_*_tools()` call
   to `builtin/mod.rs`, then update the total in
   `register_all_builtin_tools_adds_all` (currently 52).
6. **Respect the boundaries.** If the tool touches the filesystem, shell, or
   network, re-read §13 first and prefer routing through an `ExecutionContext`
   resource (`db`/`storage`/`vector`) or `system/sandbox_exec` rather than raw
   host access.
7. **Update `USAGE.md`** (the user-facing catalog) and, if you add a reusable
   pattern, `PRIMITIVES.md`.

---

## 13. Do not bypass these boundaries

These rules apply specifically to the **filesystem, system, and git** families,
which act on the host directly and are not mediated by the `ExecutionContext`.

1. **Do not treat the `system/bash` blocklist as a security boundary.** It
   catches a handful of catastrophic patterns and nothing more. It is an
   accident guardrail. Real isolation = run the engine as a least-privileged
   user, in a container/VM, with the filesystem and network locked down at the
   OS level. Never widen the blocklist and call it "hardened."

2. **Do not add a filesystem tool that confines paths in-process and assume it
   is safe.** The existing tools have **no path jail**; `canonicalize` resolves
   symlinks but does not confine to a root, and `delete` is recursive. If a
   deployment needs path confinement, it MUST come from the host environment
   (chroot/container/dedicated user), not from string checks in a tool. Don't
   build a half-jail that invites reliance.

3. **Do not route privileged work around `ExecutionContext`.** DB, storage, and
   vector access exist precisely so the host can scope tenancy, RBAC, and SQL
   safety at the resource layer. A tool that opens its own DB connection or its
   own S3 client defeats that. The only sanctioned direct-network exceptions
   today are `data/web_scrape`, `ai/claude_code`, and `mcp/call` — each
   deliberately scoped and documented above; do not add more without review.

4. **Do not run untrusted code outside `system/sandbox_exec`.** If you need to
   execute model- or user-generated code, use `sandbox_exec` (temp dir, timeout,
   `network_access: false`) — never `system/bash` or `ai/claude_code`, which run
   with the engine's full privileges.

5. **Do not let `git/commit` amend, force, or skip hooks.** `git/commit` is
   intentionally limited: it stages named files, refuses an empty message,
   refuses when nothing is staged, and runs a plain `git commit -m` (the source
   comments this: *NEVER amend, NEVER skip hooks*). Other git tools (`status`,
   `diff`, `log`) are read-only. Keep git tools incapable of history rewriting
   or remote mutation (no `push`, `reset --hard`, `commit --amend`,
   `--no-verify`). Adding those would turn a low-risk family into a destructive
   one.

6. **Do not disable the `ai/llm_call` injection scanner on untrusted input.**
   Content from `data/web_scrape`, `mcp/call`, or any tool that ingests external
   text can carry prompt-injection payloads. The scanner is on by default for a
   reason; leave it on wherever such content can reach a prompt.

7. **Do not bypass the `agent/run_agent` depth and cycle guards** when you
   implement the host-side recursion. They are the only protection against
   unbounded sub-agent loops.

---

## 14. Testing expectations

- **Co-locate unit tests** in each tool module's `#[cfg(test)] mod tests`.
  Follow the existing naming (`<tool>_<scenario>`, e.g. `bash_blocks_fork_bomb`).
- **Use the test contexts.** `InMemoryContext::new("test-run")` for tools that
  only need the stub LLM; the `TestContext` in `data/mod.rs` (stub
  `DBResource`/`StorageResource`, configurable rows) for data tools. Filesystem
  tests use `tempfile::TempDir`.
- **Cover, at minimum:**
  - the happy path with declared inputs;
  - each missing-required-input / missing-resource failure
    (`*_no_db_fails`, `*_no_storage_fails`, `*_missing_*_fails`);
  - every security guard you add (the `bash_blocks_*` set is the model);
  - any pure helper directly (e.g. `evaluate_condition`, `validate_response`,
    `normalize_url`, `convert_html_to_md`) — these are unit-testable without a
    context.
- **Registration test per family.** Each `register_*_tools()` has a test
  asserting the registered tools resolve and the `list_tools()` count is exact
  (`register_*_tools_adds_*`). Update it when you add or remove a tool, and keep
  the aggregate `register_all_builtin_tools_adds_all` (52) in sync.
- **Aliases** get their own resolution test
  (`legacy_fs_aliases_resolve`, `legacy_mcp_alias_resolves`).

Run the suite from `engine/`:

```bash
cargo test -p openmirai-engine tools::builtin
```

---

## See also

- [`USAGE.md`](../../USAGE.md) — user-facing tool catalog and agent YAML patterns.
- [`PRIMITIVES.md`](./PRIMITIVES.md) — `Tool`/`ToolFactory`/`ToolRegistry`
  patterns and other reusable engine primitives.
- [`API.md`](./API.md) — HTTP API that exposes graph execution.
- `engine/src/tools/registry.rs` — registry, executor, validation, panic net.
- `engine/src/core/context.rs` — `ExecutionContext` and resource traits.
- `engine/src/core/runner/types.rs` — `ToolError` and runner types.
</content>
</invoke>
