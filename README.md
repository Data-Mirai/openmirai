# OpenMirai

[![CI](https://github.com/Gabo-TheCreator/openmirai/actions/workflows/ci.yml/badge.svg)](https://github.com/Gabo-TheCreator/openmirai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Gabo-TheCreator/openmirai?include_prereleases&sort=semver)](https://github.com/Gabo-TheCreator/openmirai/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

> **Decentralize your AI agents.** One binary. Any LLM. Your machine. Your rules.

OpenMirai is a Rust-native engine that runs agentic workflows defined as simple YAML graphs. No cloud lock-in, no heavy runtime, no vendor handcuffs — the engine runs wherever you do: your laptop, your server, your edge. A single portable binary with zero runtime dependencies.

A drop-in alternative to LangGraph, CrewAI, and Google ADK — without tying your agents to someone else's cloud.

**50 built-in tools** · **7 LLM providers** · **712 tests** · **MIT license**

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

# Run an agent
./target/release/mirai run examples/hello-world.yaml

# With a specific provider
./target/release/mirai run examples/hello-world.yaml --provider claude --api-key $ANTHROPIC_API_KEY

# Start the HTTP server
./target/release/mirai serve --port 3000
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
Engine = Rust binary   (FFI, WASM, CLI — 712 tests)
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
| License | MIT | MIT | Apache-2.0 |

The point isn't "more features" — it's **where and how it runs**: yours, portable, and not chained to a cloud.

## Built-in Tools

| Category | Tools |
|----------|-------|
| **Trigger** | webhook, manual, schedule, event, heartbeat |
| **AI** | llm_call, embeddings, transcribe, claude_code |
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
- **Checkpoint/resume** — pause and resume agent execution
- **SSE streaming** — real-time execution events via Server-Sent Events
- **Security scanner** — prompt injection detection with configurable sensitivity
- **MCP support** — Model Context Protocol for external tool servers
- **Sub-agents** — compose agents that call other agents (max depth 3)

## HTTP Server

```bash
# Start with auth (recommended for production)
MIRAI_API_KEY=your-secret mirai serve --port 3000

# Or without auth (development only)
mirai serve --port 3000
```

API endpoints: `/api/v1/agents`, `/api/v1/graphs`, `/api/v1/sessions`, `/api/v1/tools`, `/health`

## Contributing

OpenMirai is open source and built in the open. Contributions are welcome — new tools, LLM providers, docs, examples, bug fixes.

- Start with **[CONTRIBUTING.md](CONTRIBUTING.md)**
- Pick up a [`good first issue`](https://github.com/Gabo-TheCreator/openmirai/labels/good%20first%20issue)
- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system map
- Be kind — see our [Code of Conduct](CODE_OF_CONDUCT.md)

## License

MIT
