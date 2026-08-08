# OpenMirai

[![CI](https://github.com/Data-Mirai/openmirai/actions/workflows/ci.yml/badge.svg)](https://github.com/Data-Mirai/openmirai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Data-Mirai/openmirai?include_prereleases&sort=semver)](https://github.com/Data-Mirai/openmirai/releases)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

> **Decentralize your AI agents.** One binary. Any LLM. Your machine. Your rules.

OpenMirai is a Rust-native engine that runs agentic workflows defined as simple YAML graphs. No cloud lock-in, no heavy runtime, no vendor handcuffs — the engine runs wherever you do: your laptop, your server, your edge. A single portable binary with zero runtime dependencies.

A drop-in alternative to LangGraph, CrewAI, and Google ADK — without tying your agents to someone else's cloud.

**52 built-in tools** · **7 LLM providers** · **906 tests** · **Apache-2.0 license**

## Why decentralized?

Most agent platforms run *their* runtime, on *their* cloud, against *their* preferred model. OpenMirai inverts that:

- **Runs anywhere** — one self-contained binary, zero runtime dependencies. Your machine, your server, your edge.
- **Any LLM** — 7 providers today, local Ollama included. No vendor lock-in.
- **Your data stays yours** — agents execute where you put them; nothing phones home.
- **Embeddable** — drop the engine into any app via CLI, HTTP, or as a Rust crate.

> Decentralizing AI agents means taking power back from closed platforms and handing it to whoever builds.

## Quick Start

**Requires [Rust](https://rustup.rs) 1.80+ via `rustup`.** Ubuntu's `apt install cargo` is too old (it can't parse this repo's lockfile) — install with rustup, then `cargo build` auto-selects the pinned toolchain:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # if you don't have rustup
rustup update stable
```

```bash
# Build from source
cargo build --release

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
Agent  = YAML config  (portable, versionable, language-agnostic)
Engine = Rust binary   (FFI, WASM, CLI — 906 tests)
Host   = Your app      (Python, Swift, Go, JavaScript — anything)
```

The agent defines **what** to do. The engine decides **how** to run it.

## What's different

| | OpenMirai | LangGraph / CrewAI | Google ADK |
|---|---|---|---|
| Runtime | Single binary, zero deps | Python runtime + deps | Python runtime + deps |
| Run on your own machine/edge | ✅ first-class | ⚠️ needs Python env | ⚠️ GCP-oriented |
| LLM choice | 7 providers, local-first | Provider-agnostic | Gemini-first |
| Agent format | Portable YAML | Python code | Python code |
| Embed in any app | CLI · HTTP · Rust crate | Python library | Python library |
| License | Apache-2.0 | MIT | Apache-2.0 |

The point isn't "more features" — it's **where and how it runs**: yours, portable, and not chained to a cloud.

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
- **Session orchestration** — run and coordinate several live coding sessions from one engine (see below)

Partially wired, so you know before you build on them: `GraphRunner::resume()` exists and is
tested at the crate level but is not reachable from the CLI or HTTP yet, and `agent/run_agent`
validates and guards against cycles but returns a placeholder — nested agent execution is meant
to happen in the app layer, not inside the engine.

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
