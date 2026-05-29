# Data Mirai Engine

[![CI](https://github.com/gabo-the-creator/data-mirai-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/gabo-the-creator/data-mirai-engine/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/gabo-the-creator/data-mirai-engine?include_prereleases&sort=semver)](https://github.com/gabo-the-creator/data-mirai-engine/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

Open-source agent execution engine. One binary. Any LLM. Your rules.

A Rust-native engine that runs agentic workflows defined as simple YAML graphs. Alternative to LangGraph, CrewAI, and Google ADK — compiled to a single portable binary with zero runtime dependencies.

**48+ built-in tools** | **7 LLM providers** | **665 tests** | **MIT license**

## Quick Start

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
Engine = Rust binary   (FFI, WASM, CLI — 665 tests)
Host   = Your app      (Python, Swift, Go, JavaScript — anything)
```

The agent defines **what** to do. The engine decides **how** to run it.

## Built-in Tools

| Category | Tools |
|----------|-------|
| **Trigger** | webhook, manual, schedule, event, heartbeat |
| **AI** | llm_call, embeddings, transcribe |
| **Logic** | condition, switch, loop, merge, wait, human_input, deadline |
| **Data** | db_read, db_write, db_query, storage_read, storage_write, storage_delete, vault_read, vault_write, entity_store, web_scrape, rag_search |
| **Filesystem** | read_file, write_file, edit_file, glob, grep, list_dir, tree, copy, move, delete, mkdir, file_info |
| **System** | bash, process_list, sandbox_exec |
| **Git** | status, diff, log, commit |
| **Output** | response |
| **Agent** | run_agent |
| **MCP** | mcp_call, mcp_discover |

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
pip install datamirai
```

```python
from datamirai import Engine, Agent

engine = Engine(provider="ollama")
agent = Agent.from_file("my-agent.yaml")
result = engine.run(agent, input={"query": "hello"})
print(result.output)
```

## Project Structure

```
engine/        Core library (datamirai-engine crate)
cli/           CLI binary (mirai)
sdks/python/   Python SDK (datamirai)
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

## License

MIT
