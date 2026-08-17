# OpenMirai

[![CI](https://github.com/Data-Mirai/openmirai/actions/workflows/ci.yml/badge.svg)](https://github.com/Data-Mirai/openmirai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Data-Mirai/openmirai?include_prereleases&sort=semver)](https://github.com/Data-Mirai/openmirai/releases)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

> **Graph Engineering.** Your agent workflow is a YAML file, not a Python program. One binary runs it anywhere.

OpenMirai is a Rust-native engine for **Graph Engineering**: the workflow graph is the artifact you
engineer, not a by-product of the code that happens to build it. A graph is one plain YAML file
holding the *whole* workflow — nodes, edges, data flow, routing conditions, retries, input contract.
You diff it in a pull request, validate it in CI, tag it in git, and hand it to a runtime.

That runtime is a single self-contained binary with **zero runtime dependencies**: no interpreter,
no virtualenv, no `node_modules`, no Docker Compose, no cluster. The same file runs on your laptop,
in a CI job, on a server, at the edge, or embedded in your app — driven by CLI, HTTP, an SDK, or the
Rust crate.

An alternative to LangGraph, CrewAI and Google ADK for the case where you want the graph to be a
portable artifact instead of code living inside someone else's language runtime.

→ **[docs/graph-engineering.md](docs/graph-engineering.md)** — the four properties that make a graph
engineerable, and an honest comparison with the alternatives (including where they win).

**It runs workflows. That is the whole job.** Agents as entities, goals, long-term memory and
team collaboration are deliberately *not* here — they belong to the layer you build on top.
See **[docs/SCOPE.md](docs/SCOPE.md)** for where the line falls and why.

**52 built-in tools** · **7 LLM providers** · **879 tests** · **Apache-2.0 license**

## Why Graph Engineering?

Most frameworks make you *program* a graph: import a library, register functions as nodes, wire
edges with method calls, compile. The graph then exists only while that process is alive — to read
it you read the code, and to move it you move the interpreter and the dependency tree with it. Most
platforms go further and run *their* runtime, on *their* cloud, against *their* preferred model.
OpenMirai inverts both:

- **The graph is data** — a complete YAML file. Nothing to import, no host function to implement, no decorator to remember.
- **Reviewable and versionable** — a rerouted edge or a changed prompt is a one-line diff a human can read. `mirai validate` gates it in CI, with no keys and no network.
- **Runs anywhere** — one self-contained binary, zero runtime dependencies. Your machine, your server, your edge.
- **Any LLM** — 7 providers today, local Ollama included. No vendor lock-in.
- **Your data stays yours** — graphs execute where you put them; nothing phones home.
- **Embeddable** — drop the engine into any app via CLI, HTTP, an SDK, or as a Rust crate.

> The graph declares **what** happens. The engine decides **how** it runs. Neither belongs to a cloud.

## Quick Start

**Requires [Rust](https://rustup.rs) 1.80+ via `rustup`.** Ubuntu's `apt install cargo` is too old (it can't parse this repo's lockfile) — install with rustup, then `cargo build` auto-selects the pinned toolchain:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # if you don't have rustup
rustup update stable
```

```bash
# Build from source
cargo build --release

# Check a graph without running it — no keys, no network (this is your CI gate)
./target/release/mirai validate examples/hello-world.yaml

# Run an agent (hello-world targets local Ollama — `ollama pull qwen3:8b` first,
# or pass --provider with your own key, as in the next command)
./target/release/mirai run examples/hello-world.yaml

# With a specific provider
./target/release/mirai run examples/hello-world.yaml --provider claude --api-key $ANTHROPIC_API_KEY

# Start the HTTP server
./target/release/mirai serve --port 3000

# Open the visual editor — a mini-IDE for an agent in your browser
./target/release/mirai edit examples/hello-world.yaml
```

## How It Works

An agent is a YAML file. The engine executes it.

```yaml
name: hello-world
version: v1
graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: think
      tool_type: ai/llm_call
      config:
        prompt: "Answer the user's question concisely."
    - id: respond
      tool_type: output/response
  edges:
    - source: start
      target: think
    - source: think
      target: respond
```

```bash
mirai run hello.yaml --input '{"query": "What is Rust?"}'
```

## Architecture

```
Graph  = YAML file      (portable, versionable, language-agnostic)
Engine = single binary   (CLI · HTTP · Rust crate · Python SDK)
Host   = your app        (Python, Swift, Go, JavaScript — anything that runs a process or calls HTTP)
```

The graph defines **what** to do. The engine decides **how** to run it.

## What's different

Declarative agent files are no longer rare — ADK, CrewAI, Dify and n8n all have one. The question
worth asking is **how much of the workflow lives in the file, and what it takes to run that file.**

| | OpenMirai | LangGraph | Google ADK | CrewAI |
|---|---|---|---|---|
| Where the graph lives | Complete YAML file | Python/JS code (`StateGraph`) | YAML *(Agent Config, experimental)* | `agents.yaml` + `crew.py` |
| Code needed to run it | None | The graph *is* the code | Python/Java for anything programmable | `@CrewBase` / `@agent` / `@task` |
| What it takes to execute | One static binary, zero deps | Python or Node runtime + deps | ADK runtime (Python/Java/Go) | Python runtime + deps |
| Static check of the artifact | `mirai validate` — a binary, no keys, no runtime | At `compile()`, inside Python | On agent load, inside ADK | On crew load, inside Python |
| LLM choice | 7 providers, local-first | Provider-agnostic | Gemini-only in Agent Config | Provider-agnostic |
| Embed in any app | CLI · HTTP · Rust crate · SDK | Python/JS library | Python/Java/Go library | Python library |
| License | Apache-2.0 | MIT | Apache-2.0 | MIT |

The point isn't "more features" — it's **what the artifact is and where it runs**. And the trade is
real: LangGraph's Python ecosystem, LangSmith-grade visual tracing and dynamic runtime fan-out are
advantages OpenMirai does not match today (tracked in
[docs/ROADMAP-PARITY.md](docs/ROADMAP-PARITY.md)). If most of your workflow is bespoke Python, use a
code-first framework. Full breakdown: **[docs/graph-engineering.md](docs/graph-engineering.md)**.

## Built-in Tools

| Category | Tools |
|----------|-------|
| **Trigger** | webhook, manual, schedule, event, heartbeat |
| **AI** | llm_call, embeddings, transcribe, tts, image_edit, claude_code |
| **Logic** | condition, switch, loop, merge, wait, human_input, deadline |
| **Data** | db_read, db_write, entity_query, entity_upsert, storage_read, storage_write, vault_read, vault_write, web_scrape, html_to_markdown, rag_search |
| **Filesystem** | read_file, write_file, edit_file, glob_files, grep_files, list_dir, tree, copy, move, delete, mkdir, file_info |
| **System** | bash, process_list, sandbox_exec |
| **Git** | status, diff, log, commit |
| **Output** | response |
| **Agent** | run_agent |
| **MCP** | call |
| **State** | memory |

## LLM Providers

| Provider | Config |
|----------|--------|
| **Ollama** (default) | Local, no API key needed |
| **Claude** | `--provider claude --api-key $ANTHROPIC_API_KEY` |
| **OpenAI** | `--provider openai --api-key $OPENAI_API_KEY` |
| **Gemini** | `--provider gemini --api-key $GOOGLE_API_KEY` |
| **Groq** | `--provider groq --api-key $GROQ_API_KEY` |
| **NVIDIA NIM** | `--provider nvidia --api-key $NVIDIA_API_KEY` |
| **OpenRouter** | `--provider openrouter --api-key $OPENROUTER_API_KEY` |

## Python SDK

```bash
pip install openmirai
```

```python
from openmirai import Engine, Agent

engine = Engine(provider="ollama")
agent = Agent.from_file("my-agent.yaml")
result = engine.run(agent, input={"query": "hello"})
print(result.output)
```

## Project Structure

```
engine/        Core library (openmirai-engine crate)
cli/           CLI binary (mirai)
sdks/python/   Python SDK (openmirai)
examples/      Ready-to-run agent examples
docs/          Technical documentation
```

## Key Features

- **Graph execution** with conditional branching, fan-out/fan-in, and retry with backoff
- **Input/output contracts** — typed validation with defaults, required fields, and clear error messages
- **Structured output** — JSON schema enforcement on LLM responses with auto-retry
- **Soul system** — give agents personality via SOUL.md files
- **Universe** — multi-agent routing with keyword, round-robin, or LLM-based strategies
- **Energy tracking** — metered cost accounting per operation
- **Hook system** — 7 interception points for execution control
- **SSE streaming** — real-time execution events via Server-Sent Events
- **Security scanner** — prompt injection detection with configurable sensitivity
- **MCP support** — Model Context Protocol for external tool servers
- **Run lifecycle** — a run checkpoints as it advances: it can pause for a human, be cancelled in
  flight, and resume from where it stopped — even across a process restart (see below)
- **Session orchestration** — run and coordinate several live coding sessions from one engine (see below)

Partially wired, so you know before you build on it: `agent/run_agent` validates and guards
against cycles but returns a placeholder — nested agent execution is meant to happen in the app
layer, not inside the engine.

## Run Lifecycle

Every execution is a **run** with state on disk. It stops when a graph asks a human something,
resumes from its checkpoint without re-running the nodes that already had effects, and can be
cancelled mid-flight (cooperatively, at a node boundary).

```bash
mirai runs list                                # id, agent, state, current node, start time
mirai runs list --status paused                # the ones waiting on a human
mirai runs show <run_id>                       # where it stopped, why, and its node trace
mirai runs resume <run_id> --response "yes"    # answer and continue from the next node
mirai runs cancel <run_id>                     # stop one in flight
mirai runs list --json | jq '.[].id'           # every subcommand takes --json
```

Same over HTTP: `GET /api/v1/sessions?status=paused`, `POST /api/v1/sessions/{id}/resume`,
`POST /api/v1/sessions/{id}/cancel`. Runs are persisted to SQLite (`--db-path`), so a run that
was paused before a restart is still there — and still resumable — afterwards.

## Session Orchestration

The engine can spawn and coordinate several live coding sessions at once — one per `tmux`
window — sending them messages, reading their output, and stopping or restarting them.

```bash
mirai serve --port 3000                        # the CLI talks to a running server
mirai sessions spawn --project ~/code/my-app --objective "add the retry test"
mirai sessions list                            # what's running
mirai sessions send <id> "run the tests"       # talk to it
mirai sessions output <id>                     # read it back
mirai sessions watch                           # live terminal dashboard
mirai sessions stop <id>
```

**Requirements (not bundled):** [`tmux`](https://github.com/tmux/tmux) must be installed, and
the session command defaults to the [`claude`](https://claude.com/claude-code) CLI, which you
provide and authenticate yourself. Unix-only — on other platforms these commands short-circuit
with a clear error. Everything else in OpenMirai runs without them.

## HTTP Server

```bash
# Start with auth (recommended for production)
MIRAI_API_KEY=your-secret mirai serve --host 0.0.0.0 --port 3000

# Local development — binds 127.0.0.1 by default
mirai serve --port 3000
```

API endpoints: `/api/v1/agents`, `/api/v1/graphs`, `/api/v1/sessions`, `/api/v1/tools`, `/health`

The server binds **loopback by default**, and refuses to start on a non-loopback host without
an API key: the agent-execute and orchestrator endpoints amount to remote command execution for
anyone who can reach the port. CORS is loopback-only for the same reason.

Runs are persisted to a local SQLite database the engine creates on first use at
`~/.openmirai/engine.db` (override with `--db-path` / `MIRAI_DB_PATH`), so executions survive a
restart. A write failure there never fails the request.

## Contributing

OpenMirai is open source and built in the open. Contributions are welcome — new tools, LLM providers, docs, examples, bug fixes.

- Start with **[CONTRIBUTING.md](CONTRIBUTING.md)**
- Pick up a [`good first issue`](https://github.com/Data-Mirai/openmirai/labels/good%20first%20issue)
- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system map
- Be kind — see our [Code of Conduct](CODE_OF_CONDUCT.md)

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
