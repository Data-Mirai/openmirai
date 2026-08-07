# OpenMirai — CLI Reference (`mirai`)

> 🌐 Disponible en [Español](CLI.es.md).

The `mirai` binary is the primary way to run OpenMirai locally. It has two
distinct modes:

- **Non-interactive commands** — one-shot subcommands (`run`, `validate`,
  `serve`, `tools`, …) that execute and exit. These map onto the engine's
  `GraphRunner` and are what you script, embed, and call from CI.
- **Interactive mode** — running `mirai` with no arguments launches a setup
  wizard followed by an agentic chat terminal. This mode does **not** use the
  graph runner; it drives an LLM adapter directly through a tool-calling loop.

Source: `cli/src/main.rs` (command dispatch), `cli/src/terminal.rs`
(interactive loop), `cli/src/setup_wizard.rs` (wizard), `cli/src/session_storage.rs`
(persistence), `cli/src/adapter_factory.rs` (provider→adapter), `cli/src/colors.rs`
(ANSI output).

Related docs: [ARCHITECTURE.md](ARCHITECTURE.md) ·
[backend/API.md](backend/API.md) (the `serve` HTTP surface) ·
[frontend/PANTALLAS.md](frontend/PANTALLAS.md) and
[frontend/DESIGN-GUIDE.md](frontend/DESIGN-GUIDE.md) (terminal UX) ·
[USAGE.md](../USAGE.md) (agent YAML authoring).

---

## 1. Building the binary

```bash
# From the repo root
cargo build --release
./target/release/mirai version       # → mirai 0.7.0

# Or run unoptimized during development
cargo run -p openmirai-cli -- run examples/hello-world.yaml
```

All examples below assume `mirai` is on your `PATH` (or substitute
`./target/release/mirai`).

---

## 2. Command surface

`mirai <command> [args]`. The first argument selects the command; anything else
is parsed positionally or as `--flag value` pairs.

| Command | Interactive? | What it does |
|---|---|---|
| `mirai` (no args) | **Yes** | Setup wizard → interactive agentic terminal |
| `mirai run <file>` | No | Execute a YAML agent spec once and print result JSON |
| `mirai validate <file>` | No | Parse + report node/edge counts; non-zero exit on error |
| `mirai serve` | No | Start the HTTP server (see [API.md](backend/API.md)) |
| `mirai tools [<tool_type>]` | No | List all tools, or show one tool's inputs/outputs/config |
| `mirai templates` | No | List built-in agent templates |
| `mirai new --template <id> --name <n>` | No | Scaffold a `<n>.yaml` from a template |
| `mirai describe <file>` | No | Print an agent's input/output contract |
| `mirai eval --output <text> …` | No | Run evaluators (relevance, latency, …) on an I/O pair |
| `mirai rag search --query … --documents …` | No | Chunk + embed + cosine-rank local documents |
| `mirai agent <load\|list>` | No | **Stubs** — print "not yet implemented" |
| `mirai version` (`--version`, `-V`) | No | Print version |
| `mirai help` (`--help`, `-h`) | No | Print usage |

Unknown commands print `Unknown command: <x>. Run \`mirai help\` for usage.` to
stderr and exit `1`.

> **No `mirai play` command exists** even though `mirai run` refers to it (see
> [§11 Troubleshooting](#11-common-failure-modes-and-troubleshooting)).

---

## 3. Provider detection and configuration

Most commands that touch an LLM (`run`, `serve`, `eval`, `rag`) resolve the
provider, model, API key, and base URL using the same chain
(`resolve_provider` in `main.rs`).

### Resolution order

**Provider:**
1. `--provider <name>` flag
2. `MIRAI_LLM_PROVIDER` env var
3. Auto-detect from the model name (see table below)
4. Default: `ollama`

**Model:**
1. `--model <name>` flag
2. `MIRAI_LLM_MODEL` env var
3. Provider default from `adapter_factory::default_model`

### Model → provider auto-detection

`detect_provider_from_model` inspects the lowercased model name:

| Model name pattern | Detected provider |
|---|---|
| `gpt*`, `o1*`, `o3*`, `o4*` | `openai` |
| `claude*` | `claude` |
| `gemini*`, `gemma*` | `gemini` |
| contains `llama`, `mistral`, `qwen`, `phi`, `deepseek` | `ollama` |
| anything else | (no detection → falls through to default) |

### Supported providers and defaults

From `adapter_factory.rs`:

| `--provider` | Default model | API key env (or `--api-key`) | Default base URL |
|---|---|---|---|
| `ollama` (default) | `qwen3:8b` | none (local) | `OLLAMA_BASE_URL` or `http://localhost:11434` |
| `openai` | `gpt-4o` | `OPENAI_API_KEY` | `https://api.openai.com/v1` |
| `claude` / `anthropic` | `claude-sonnet-4-20250514` | `ANTHROPIC_API_KEY` | `https://api.anthropic.com/v1` |
| `gemini` / `google` | `gemini-2.5-flash` | `GOOGLE_API_KEY` | `https://generativelanguage.googleapis.com/v1beta` |
| `groq` | `qwen-qwq-32b` | `GROQ_API_KEY` | (adapter default) |
| `nvidia` | `meta/llama-3.3-70b-instruct` | `NVIDIA_API_KEY` | (adapter default) |
| `openrouter` | `meta-llama/llama-3.3-70b-instruct` | `OPENROUTER_API_KEY` | (adapter default) |
| `mock` | — | — | uses `MockLLMResource` (testing only) |
| *(unknown)* | `qwen3:8b` | optional | treated as OpenAI-compatible, defaults to `http://localhost:11434/v1` |

An API key resolves as: explicit `--api-key` value → provider env var → empty
string. For `claude`/`gemini`, an empty key prints a warning but still
constructs the adapter.

`--provider mock` swaps in `MockLLMResource` for deterministic testing — valid
for both `run` and `serve`.

---

## 4. Non-interactive commands

### `mirai run <file>` — execute an agent once

```bash
mirai run agent.yaml
mirai run agent.yaml --input '{"query": "hello"}'
mirai run agent.yaml --provider ollama --model gemma4
mirai run agent.yaml --provider openai --model gpt-4o
mirai run agent.yaml --provider claude --api-key $ANTHROPIC_API_KEY
mirai run agent.yaml --trace            # print trace tree + metrics to stderr
mirai run agent.yaml --benchmark        # log timing to benchmarks.jsonl
```

**Flags:**

| Flag | Meaning |
|---|---|
| `-i`, `--input <JSON>` | Input payload. Objects become the trigger payload; non-objects are wrapped as `{"_raw": …}` |
| `--provider`, `--model`, `--api-key`, `--base-url` | Provider config (see §3) |
| `--trace` | After the run, render the trace tree + metrics (nodes, total/avg ms, retries) to stderr |
| `--benchmark` | Enable benchmark logging (also via `MIRAI_BENCHMARK=1`); file via `MIRAI_BENCHMARK_FILE` (default `benchmarks.jsonl`) |

**Execution pipeline** (this is the canonical CLI→engine mapping; see also §9):

1. `AgentSpec::from_file(path)` parses a `.yaml` or `.yml` spec. JSON files are rejected.
2. If `agent_type == Live`, the run is **rejected** (use a server / live runner).
3. `spec.to_graph()` → `auto_generate_edge_ids()` → `graph.validate()`.
4. A `ToolRegistry` is populated by `register_all_builtin_tools`.
5. A `RegistryExecutor` is wrapped in a `GraphRunner`.
6. If the spec declares a `soul`, it is loaded and converted into the system
   prompt; otherwise `spec.system_prompt` is used.
7. The `ExecutionContext` is built with the resolved LLM plus an **in-memory**
   DB and storage (`InMemoryDBResource`, `InMemoryStorageResource`).
8. If `--input` is given and the spec declares `inputs`, the payload is
   validated (`validate_agent_inputs`); failures exit `1` with per-field errors.
   The validated payload is injected into the first `trigger/*` node.
9. `model` is injected into any `ai/llm_call` node that didn't set one; MCP
   server configs are injected into `mcp/call` nodes.
10. `GraphRunner::run` executes the graph.

**Output** — pretty-printed JSON on stdout:

```json
{
  "status": "Completed",
  "state": { "...": "final state snapshot" },
  "trace": [ /* per-node execution records */ ],
  "transcript": [ /* message transcript */ ],
  "error": null,
  "interrupt_info": null
}
```

**Exit codes:** `0` when `status == Completed`; `1` on load/validation error,
input-validation failure, live-agent rejection, or any non-`Completed` status.
Provider info (`LLM: <provider>/<model>`) and soul/warning lines go to
**stderr**, so stdout stays clean JSON for piping.

> **In-memory only:** `mirai run` gives the agent ephemeral DB/storage. Data and
> storage tools won't persist across runs. Use `mirai serve` for a longer-lived
> context.

### `mirai validate <file>`

```bash
mirai validate my-agent.yaml
# → ✓ Valid agent spec: 'my-agent' (3 nodes, 2 edges)
```

Parses the spec and reports node/edge counts. Invalid specs print
`✗ Invalid agent spec: <error>` to stderr and exit `1`. It does **not** run the
graph or contact any LLM.

### `mirai serve`

```bash
mirai serve --port 3000
mirai serve --host 127.0.0.1 --port 8080 --provider openai --model gpt-4o
MIRAI_API_KEY=secret mirai serve --port 3000      # require X-API-Key
```

| Flag / env | Default | Meaning |
|---|---|---|
| `--port` | `3000` | Listen port |
| `--host` | `0.0.0.0` | Bind address |
| `--provider` / `--model` / `--api-key` / `--base-url` | see §3 | LLM config |
| `--api-key` *(server)* / `MIRAI_API_KEY` | none | If set, requests must send a matching `X-API-Key` |

The server builds a **real** LLM resource per request via an adapter factory
(`mock` is only used when `--provider mock` is set explicitly). Endpoints and
request/response contracts live in [backend/API.md](backend/API.md).

### `mirai tools [<tool_type>]`

```bash
mirai tools                    # all tools, grouped by category
mirai tools ai/claude_code     # inputs, outputs, and config for one tool
```

The list view groups every registered tool by category and truncates
descriptions to 60 chars. The detail view prints the tool's `INPUTS`
(received via `data_map`), `OUTPUTS` (usable in the next edge's `data_map`),
and `CONFIG` fields (with required/optional and defaults). Unknown tool types
exit `1`.

### `mirai templates` / `mirai new`

```bash
mirai templates
mirai new --template <id> --name my-agent
mirai new --template <id> --name my-agent --provider ollama
```

`templates` lists built-in templates (id, category, description). `new` clones
a template's spec, overrides `name`, optionally injects `provider` into every
`ai/llm_call` node, and writes `<name>.yaml` to the current directory. Missing
`--template` prints usage and exits `1`; an unknown template id exits `1`.
`--name` defaults to the template id if omitted.

### `mirai describe <file>`

```bash
mirai describe my-agent.yaml
```

Prints the agent's name/version/description and its declared `inputs` and
`outputs` contracts (name, type, required/optional, description). Agents with no
declared inputs show `(none declared — accepts any payload)`.

### `mirai eval`

```bash
mirai eval --input "What is 2+2?" --output "4" --types relevance,format_compliance
```

| Flag | Default | Meaning |
|---|---|---|
| `--input <text>` | empty | The original input/prompt |
| `--output <text>` | — | **Required** — the response to score |
| `--types <list>` | `format_compliance,latency` | Comma-separated evaluators |

Valid `--types`: `relevance`, `faithfulness`, `completeness`,
`format_compliance`, `latency` (unknown names are silently dropped). Results
print as a labeled bar chart with scores in `[0,1]`. Missing `--output` exits
`1`.

### `mirai rag search`

```bash
mirai rag search --query "vector databases" --documents notes.md,paper.txt --top-k 5
```

| Flag | Default | Meaning |
|---|---|---|
| `--query <text>` | — | **Required** search query |
| `--documents <paths>` | — | **Required** comma-separated file paths |
| `--top-k <n>` | `3` | Number of results |

Reads each document, chunks by paragraph (size 512 / overlap 50), embeds the
query and every chunk via the resolved provider's embeddings, ranks by cosine
similarity, and prints the top-k chunk previews with scores. Unreadable files
warn and are skipped; if no documents load, it exits `1`. Any subcommand other
than `search` prints usage and exits `1`.

> `rag search` needs a provider that supports embeddings. With Ollama, ensure an
> embedding-capable model/endpoint is available.

### `mirai agent <load|list>` — not implemented

Both subcommands currently print a `not yet implemented` notice. They exist as
placeholders for future agent registry management; do not script against them.

---

## 5. Interactive mode — lifecycle

Running `mirai` with no arguments calls `run_default`:

```
mirai
  └─ setup_wizard::run_setup_wizard(false, "", "", "")   # always interactive
        └─ Some(SessionConfig)  → terminal::run_interactive_session(config)
        └─ None                 → "Setup cancelled." + exit 0
```

There is **no flag to pre-seed the wizard or skip it** from `main.rs` — bare
`mirai` always opens the wizard. (`run_setup_wizard` has a non-interactive quick
path, but it's only reachable programmatically, not via CLI flags today.)

---

## 6. Setup wizard behavior

`setup_wizard.rs` uses `crossterm` raw mode for arrow-key selection (↑/↓ or
`k`/`j` to move, Enter to pick, Esc/`q` to cancel). The flow:

### Step 1 — Local or Cloud

```
Donde correra el modelo?
  -> Local (Ollama — corre en tu maquina)
     Cloud (necesita API key)
```

### Step 2a — Local provider setup (Ollama)

`setup_local_provider` runs a self-healing loop:

1. **Installed?** Checks `which ollama`. If missing, offers to install
   (Homebrew on macOS, `curl … | sh` on Linux, manual link elsewhere), retry,
   switch to Cloud, or exit.
2. **Running?** Calls `GET {base_url}/api/tags`. If `not_running`, it tries
   `ollama serve` in the background and polls health for ~6s.
3. **Has models?** If models exist, you pick one (model name, locality,
   modality, context window are shown). If none, it offers a curated list to
   `ollama pull` (`qwen3:8b`, `llama3.2`, `gemma4`, `deepseek-r1:8b`), then
   re-detects.

Per-model context windows come from `POST /api/show` (`*.context_length`).
Vision support is inferred from the model name (`llava`, `gpt-4o`, `gemini`,
`claude-3`, `pixtral`, …).

### Step 2b — Cloud provider setup

`setup_cloud_provider` detects which provider env keys are set and lets you pick
among the available ones:

| Provider | Default model | Env key |
|---|---|---|
| Groq (free tier) | `qwen-qwq-32b` | `GROQ_API_KEY` |
| NVIDIA NIM (free tier) | `meta/llama-3.3-70b-instruct` | `NVIDIA_API_KEY` |
| OpenAI (paid) | `gpt-4o` | `OPENAI_API_KEY` |
| OpenRouter (multi-model) | `meta-llama/llama-3.3-70b-instruct` | `OPENROUTER_API_KEY` |

If no keys are set, it shows the `export …` hints and offers retry / switch to
Local / exit. Cloud models default to a 128K context window.

### Step 3 — Context window

Offers Default (the model's own capacity) plus 4K / 8K / 16K / 32K / 64K
presets (deduplicated).

### Step 4 — Autonomy level

| Level | `max_tool_rounds` | Confirms writes? |
|---|---|---|
| Assisted | 1 | Yes |
| **Copilot** (default) | 25 | No |
| Autopilot | 50 | No |
| Self-Driving | 100 | No |

The wizard returns a `SessionConfig { provider, model, autonomy_level,
max_tool_rounds, confirm_writes, context_window, supports_vision,
temperature: 0.3, max_tokens: 4096 }`.

---

## 7. Interactive terminal — the agentic loop

`run_interactive_session` (`terminal.rs`) is the chat REPL. On start it prints
the banner, provider/model/CWD, registers all built-in tools, builds the tool
schemas, creates a session, and enters the input loop.

### Tool exposure

Built-in tools are converted to OpenAI-style function schemas
(`spec_to_openai_schema`); the function name is the `tool_type` with `/`
replaced by `_` (e.g. `filesystem/read_file` → `filesystem_read_file`), and a
`name_map` reverses it for execution.

- **Excluded everywhere:** `trigger/webhook`, `trigger/manual`,
  `trigger/schedule`, `trigger/heartbeat`, `output/response` (these are
  graph-only blocks, meaningless in a chat).
- **Core-only for Ollama:** when `provider == "ollama"`, only a smaller "core"
  set is exposed (filesystem read/write/edit/glob/grep/list/tree/mkdir/delete,
  `system/bash`, and `git status/diff/log/commit`) to fit smaller context
  windows. Cloud providers get the full catalog.

### One turn

For each user message, `agentic_loop` runs up to `max_tool_rounds` rounds:

1. Convert the message history to engine `Message`s.
2. Try **streaming** (`stream_with_messages`, tokens print as they arrive); on
   any error, fall back to non-streaming (`call_with_messages`). A spinner shows
   while waiting.
3. Track token usage (`TokenTracker`).
4. If the response has **no tool calls**, it's the final answer — return it.
5. Otherwise, append the assistant message (with `tool_calls`) and execute each
   call:
   - Resolve the function name back to a `tool_type`.
   - **Confirmation:** at `assisted` autonomy, write-class tools
     (`filesystem/write_file`, `edit_file`, `move`, `copy`, `delete`, `mkdir`,
     `system/bash`, `git/commit`) prompt `? Confirm …? [Y/n]`. Declining injects
     a `"User declined this action"` tool result.
   - Execute via the registry against a fresh
     `DefaultExecutionContext::default_dev()`. Relative `path`/`source`/
     `destination` args are resolved against the CWD, and `cwd` is injected into
     config for tools that accept it.
   - Log the call and result to the session transcript; print a ✓/✗ line with
     a result summary and elapsed time.
   - Tool results over 30,000 chars are truncated before being fed back to the
     model.

If `max_tool_rounds` is hit, the turn ends with
`(Max tool-call rounds reached. Please continue or rephrase.)`.

### System prompt

`build_system_prompt` injects the CWD, platform, hard tool-use rules, safety
guardrails (never `rm -rf /`, never touch `.env`/secrets, never `git push`/
`reset --hard`/`push --force`, stop after 5 failed attempts), and an
autonomy-specific section (Assisted L1 → one tool/turn; Copilot L2 → multi-tool;
Autopilot L3 → autonomous; Self-Driving L4 → goal-pursuing).

### Slash commands

Handled by `handle_slash` (input starting with `/`):

| Command | Behavior |
|---|---|
| `/help` | Show the command list |
| `/quit`, `/exit`, `/q` | Save session and exit |
| `/clear` | Reset the conversation, keeping only the system prompt |
| `/compact` | Force context compaction toward ~40K tokens |
| `/tokens` | Show session token usage and estimated context size |
| `/tools` | List exposed tools grouped by category prefix |
| `/session` | Show current session id, message/checkpoint counts |
| `/sessions` | List up to 15 saved sessions (●active / ○closed), marking the current one |
| `/checkpoint [label]` | Record a checkpoint at the current message index |

Unknown slash commands print `Unknown command: <x>. Type /help`. Pressing
Ctrl+C while the model is thinking interrupts; EOF (Ctrl+D) or `/quit` closes
the session.

---

## 8. Session persistence and transcript storage

`session_storage.rs` persists every interactive session under:

```
~/.datamirai/sessions/<session_id>/
  manifest.json      # metadata
  transcript.jsonl   # append-only event log
```

> The `.datamirai` directory name is **intentional legacy naming**, kept for
> backward compatibility. `HOME` is used to locate it (falls back to `/tmp`).

**Session id:** `ses_<unix_seconds>_<8-hex-random>` (the randomness is a cheap
time-seeded xorshift, no extra deps). **Checkpoint id:** `chk_<6-hex>`.

**`manifest.json`** holds `id, provider, model, cwd, created_at, updated_at,
message_count, checkpoint_count, status` (`active` → `closed` on exit). Each
appended entry bumps `updated_at` and `message_count`.

**`transcript.jsonl`** — one JSON object per line, each with `ts`, `role`, and
optional `content` + flattened metadata. Roles written:

| Role | When | Notes |
|---|---|---|
| `user` | user message | |
| `assistant` | final assistant text | |
| `tool_call` | a tool invocation | metadata: `tool`, `args`, `round` |
| `tool_result` | a tool's result | content truncated at 5,000 chars; metadata: `tool`, `round` |
| `checkpoint` | `/checkpoint` | metadata: `checkpoint_id`, `label`, `message_index` |

Helpers exist to `read_transcript`, `read_manifest`, `list_sessions` (newest
first), `list_checkpoints`, and `rebuild_messages` (filters to
`user`/`assistant`/`system`).

> **No resume command.** Sessions are written and listable (`/sessions`), and
> `rebuild_messages` can reconstruct history, but there is currently **no CLI
> command to reopen a previous session** — each `mirai` launch starts a fresh
> conversation.

---

## 9. Message compaction behavior

To keep the conversation within the model's context, `terminal.rs` estimates
tokens as `content.len() / 4` (plus tool-call argument lengths) and compacts in
two passes (`compact_messages`):

1. **Truncate old tool results.** For messages before the last 12, any `tool`
   message longer than 500 chars is cut to 200 chars + `... (compacted)`.
2. **Drop the middle.** If still over budget and there are more than 13
   messages, keep the system prompt, insert a single
   `(Earlier conversation was compacted to save context. Continue from here.)`
   placeholder, and keep the **last 12** messages.

Triggers:
- **Automatic:** after every user message, with an **80,000**-token budget.
- **Manual:** `/compact` runs the same logic with a tighter **40,000**-token
  budget and prints `Compacted: ~<before> → ~<after> tokens`.

`/clear` is the hard reset — it drops everything except the system prompt.

---

## 10. How CLI execution maps into engine execution

The two modes reach the engine very differently:

| | `mirai run` (and `serve`) | Interactive terminal |
|---|---|---|
| Spec source | `AgentSpec` from a YAML file | None — free-form chat |
| Orchestration | `GraphRunner` + `RegistryExecutor` over a validated graph | Hand-rolled `agentic_loop` |
| Tool invocation | Engine walks nodes; tools run inside the graph | LLM emits function calls; CLI runs them directly via `ToolRegistry` |
| Execution context | Built per run with resolved LLM + in-memory DB/storage | A fresh `DefaultExecutionContext::default_dev()` **per tool call** |
| LLM interface | `LLMResource` (via `AdapterBridgeLLMResource`) | `LLMAdapter` directly (`stream_with_messages` / `call_with_messages`) |
| Triggers / outputs | `trigger/*` and `output/response` are real graph nodes | Those tool types are excluded from the chat schema |

In short: **`run`/`serve` are graph execution; interactive mode is a
tool-calling loop around a raw adapter.** Built-in tool implementations are
shared between both paths through `register_all_builtin_tools` and the
`ToolRegistry`.

---

## 11. Common failure modes and troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `Error: live agents must be started with \`mirai play\`` | The spec is `agent_type: Live`, and `mirai run` refuses it — **but no `play` subcommand exists yet** | Change the agent to a non-live type, or run it through `mirai serve` / the HTTP API |
| `Error: Cannot connect to provider. Is it running?` (interactive) | Ollama (or the configured provider) isn't reachable | Start Ollama (`ollama serve`); check `OLLAMA_BASE_URL` |
| `Error: Model timed out…` (interactive) | Model too large / context too big | Pick a smaller model or a smaller context window in the wizard |
| `Warning: Provider 'claude' requires API key…` | Missing key for a cloud provider | Set `--api-key` or the provider's env var (`ANTHROPIC_API_KEY`, etc.) |
| `input validation failed: missing required input: …` | `--input` is missing a field the spec declares as required | Add the field to the `--input` JSON, or check `mirai describe <file>` |
| `Graph validation failed: …` | Malformed graph (bad edges, missing nodes) | Run `mirai validate <file>` and fix the reported error |
| `mirai run` prints non-JSON noise mixed with JSON | Provider/soul/trace lines go to **stderr** | Redirect: `mirai run a.yaml 2>/dev/null` to capture clean stdout JSON |
| Setup wizard arrow keys don't work | No TTY (piped/non-interactive shell) | Run `mirai` in a real terminal; the wizard needs raw-mode input |
| Data/storage didn't persist between `mirai run` calls | `run` uses in-memory DB/storage | Use `mirai serve` for a longer-lived context |
| `rag search` returns nothing / errors on embeddings | Provider has no embeddings support/model | Use a provider/model that supports embeddings |
| `mirai agent load/list` does nothing useful | Both are unimplemented stubs | Don't depend on them yet |

---

## 12. Examples for local development

```bash
# 0. Build
cargo build --release
alias mirai=./target/release/mirai

# 1. Validate before running
mirai validate examples/hello-world.yaml

# 2. Run locally against Ollama (default provider)
mirai run examples/hello-world.yaml --input '{"query": "What is Rust?"}'

# 3. Deterministic run with the mock provider (no LLM, great for tests)
mirai run examples/hello-world.yaml --provider mock

# 4. Inspect a tool's contract before wiring it into a graph
mirai tools system/bash

# 5. Scaffold a new agent from a template, then run it
mirai templates
mirai new --template <id> --name scratch-agent --provider ollama
mirai run scratch-agent.yaml

# 6. See the execution trace and metrics
mirai run examples/hello-world.yaml --trace

# 7. Benchmark a run (writes benchmarks.jsonl)
MIRAI_BENCHMARK=1 mirai run examples/hello-world.yaml

# 8. Start the dev server (no auth) and hit it
mirai serve --port 3000
#   → see docs/backend/API.md for the request/response contracts

# 9. Open the interactive coding terminal
mirai
#   pick Local → Ollama → a model → context window → autonomy
#   then chat; use /help, /tools, /tokens, /compact, /quit
```

---

*Source of truth: `cli/src/*.rs` at v0.7.0. If the CLI changes, update this doc
and the command tables above to match the code.*
