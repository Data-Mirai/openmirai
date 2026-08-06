# OpenMirai — Architecture

## 1. Stack

| Layer | Technology | Version |
|---|---|---|
| Core / Runtime | Rust | 2021 edition |
| HTTP Server | Axum + Tower | 0.8 |
| Database | SQLite (rusqlite, bundled) | 0.35 |
| Vector Search | SQLite FTS5 | built-in |
| LLM | 7 providers via trait adapters | — |
| Agent Specs | YAML-only (serde_yaml) | — |
| SDKs | Python (openmirai) + TypeScript (openmirai) | 3.10+ / Node 18+ |

## 2. Distribution

Single compiled binary: `mirai`

| Mode | Usage | What it includes |
|---|---|---|
| **CLI** | `mirai run agent.yaml` | Execute agents from terminal |
| **Server** | `mirai serve --port 3000` | HTTP API with SSE streaming |
| **Library** | `cargo add openmirai-engine` | Embeddable Rust crate |
| **Python SDK** | `pip install openmirai` | Thin wrapper over HTTP API or CLI |

No Docker, no Node.js, no Python runtime required for the engine itself. One binary, any platform.

## 3. Module Architecture

```
engine/src/
  core/                  # Heart of the engine
    runner/              # GraphRunner — DAG traversal with hooks, retry, fan-out
      graph_runner.rs    # Main execution loop (run_from + 10 submethods)
      types.rs           # ExecutionResult, TraceEntry, RetryPolicy, etc.
      traits.rs          # HookHandler, CheckpointCallback, ToolExecutor
      helpers.rs         # Timestamps, hook timeouts
    agent_spec.rs        # YAML parsing, validation, input/output contracts
    graph.rs             # GraphDef, NodeDef, EdgeDef, conditions
    context.rs           # ExecutionContext trait (ports for DB, LLM, Storage, Vector)
    state.rs             # ExecutionState — thread-safe shared state
    schema.rs            # Table schema definitions
    auth.rs              # RBAC permission matrix
    events.rs            # Event emission (broadcast channels)
    value_type.rs        # Unified ValueType enum
    well_known.rs        # Named constants (zero magic strings)

  adapters/              # Concrete implementations (hexagonal architecture)
    adapter_bridge.rs    # LLMAdapter → LLMResource bridge
    context.rs           # DefaultExecutionContext (builder pattern)
    sqlite_db.rs         # SQLite DBResource
    ollama_llm.rs        # Ollama LLMResource
    mock_llm.rs          # Mock for tests only
    in_memory_db.rs      # In-memory DBResource
    in_memory_storage.rs # In-memory StorageResource
    local_storage.rs     # Filesystem StorageResource
    simple_vector.rs     # SQLite FTS5 VectorResource

  llm/                   # LLM provider adapters
    adapter.rs           # LLMAdapter trait + NormalizedResponse
    error.rs             # Shared error mapping
    claude.rs            # Anthropic Claude
    gemini.rs            # Google Gemini
    ollama.rs            # Local Ollama
    openai_compat.rs     # OpenAI-compatible (base for Groq, NVIDIA, OpenRouter)
    groq.rs              # Groq (thin wrapper)
    nvidia.rs            # NVIDIA NIM (thin wrapper)
    openrouter.rs        # OpenRouter (thin wrapper)

  tools/                 # Tool system
    base.rs              # ToolSpec, ToolField, FieldType, validation
    registry.rs          # ToolRegistry + RegistryExecutor
    builtin/
      ai/                # llm_call, embeddings, transcribe
      data/              # db_read, db_write, storage_*, vault_*, web_scrape, rag_search
      filesystem/        # read_file, write_file, edit_file, glob, grep, tree, etc.
      logic/             # condition, switch, loop, merge, wait, human_input, deadline
      system/            # bash, process_list, sandbox_exec
      git/               # status, diff, log, commit
      output/            # response
      agent/             # run_agent (sub-agent execution)
      trigger/           # webhook, manual, schedule, event, heartbeat
      mcp/               # MCP protocol calls

  server/                # HTTP API (feature-gated: optional)
    mod.rs               # Router, serve(), auth middleware, graceful shutdown
    state.rs             # AppState, LLMFactory, request/response types
    handlers.rs          # All HTTP endpoint handlers
    helpers.rs           # Agent execution helpers, RAG search
    tests.rs             # API integration tests

  intelligence/          # AI-powered meta-features
    context_compiler.rs  # 5-phase prompt assembly
    reflector.rs         # LLM-powered trace analysis
    suggester.rs         # Graph improvement suggestions
    memory_flusher.rs    # Auto-persist before context compression
    playbook.rs          # Rule-based prompt injection
    tracer.rs            # Passive execution tracing

  memory/                # Short-term + long-term memory
  energy/                # Cost tracking per operation
  search/                # Hybrid vector + FTS search
  render/                # Markdown → HTML + Chart.js
  mcp/                   # Model Context Protocol client
  vault/                 # Markdown note vault with frontmatter
  runtime/               # AgentRuntime + Scheduler
  triggers/              # Trigger type definitions

  soul.rs                # SOUL.md personality system
  universe.rs            # Multi-agent routing (keyword, round-robin, LLM, explicit)
  security.rs            # Prompt injection scanner
  sandbox.rs             # Code execution sandbox
  streaming.rs           # SSE event types
  templates.rs           # 10 pre-built agent templates
  observability.rs       # Trace spans + metrics
  eval.rs                # Evaluation framework (format, latency, LLM-as-judge)
  benchmark.rs           # JSONL benchmark logging
  rag.rs                 # RAG pipeline with chunking strategies
```

## 4. Two-Layer LLM Architecture

```
LLMAdapter (llm/adapter.rs)              LLMResource (core/context.rs)
├── Provider-specific HTTP details       ├── Simplified domain interface
├── Messages + tool calling              ├── Prompt-in, response-out
├── Streaming support                    ├── Embeddings
├── Model listing                        └── Used by tools + runner
└── Implemented per-provider
          │
          └── AdapterBridgeLLMResource (adapters/adapter_bridge.rs)
              bridges LLMAdapter → LLMResource
```

LLMAdapter = infrastructure (how to talk to a provider).
LLMResource = domain (what a tool needs to call an LLM).
Separate concerns — adding a provider never touches the execution engine.

## 5. Resource Interfaces (Ports)

ExecutionContext provides abstract access to resources:

| Resource | Trait | Adapters |
|---|---|---|
| Relational DB | `DBResource` | SqliteDBResource, InMemoryDBResource |
| LLM | `LLMResource` | AdapterBridgeLLMResource, OllamaLLMResource, MockLLMResource |
| Object Storage | `StorageResource` | LocalStorageResource, InMemoryStorageResource |
| Vector Search | `VectorResource` | SimpleVectorResource (SQLite FTS5) |

## 6. Naming Conventions

| Context | Convention |
|---|---|
| Rust files | snake_case |
| Rust structs/enums | PascalCase |
| Rust functions | snake_case |
| Tool types | `category/tool_name` (e.g., `ai/llm_call`, `filesystem/read_file`) |
| Agent specs | YAML-only, `.yaml` or `.yml` extension |
| Comparison operators | Full words recommended: `equals`, `greater_than`, `contains` |

## 7. Testing

| Type | Tool | Count |
|---|---|---|
| Unit + Integration | `cargo test` | 717 tests |

All tests run with `cargo test`. No external services required (SQLite bundled, mocks for LLM in test-only code).

Rule: **Mocks only in `#[cfg(test)]` unit tests.** Server, CLI, and integration paths use real implementations.

## 8. HTTP API

Base URL: `/api/v1/`

| Endpoint | Method | Description |
|---|---|---|
| `/health` | GET | Health check (no auth) |
| `/version` | GET | Version info (no auth) |
| `/api/v1/agents` | GET, POST | List / create agents |
| `/api/v1/agents/from-spec` | POST | Create agent from YAML spec |
| `/api/v1/agents/{id}/execute` | POST | Execute agent (sync) |
| `/api/v1/agents/{id}/stream` | POST | Execute agent (SSE streaming) |
| `/api/v1/agents/{id}/spec` | GET | Get agent spec |
| `/api/v1/agents/{id}/schema` | GET | Get input/output schema |
| `/api/v1/graphs` | GET, POST | List / create graphs |
| `/api/v1/graphs/{id}` | GET, DELETE | Get / delete graph |
| `/api/v1/sessions` | GET | List sessions |
| `/api/v1/sessions/{id}` | GET | Get session result |
| `/api/v1/tools` | GET | List registered tools |
| `/api/v1/templates` | GET | List agent templates |
| `/api/v1/rag/search` | POST | RAG search with embeddings |
| `/api/v1/eval` | POST | Evaluate session with LLM judge |
| `/api/v1/universe/message` | POST | Route message to agent |
| `/api/v1/universe/groupchat` | POST | Multi-agent debate |
| `/api/v1/metrics` | GET | Server metrics |
| `/webhooks/{path}` | POST | Webhook receiver |

Authentication: `X-API-Key` header (optional, configured via `MIRAI_API_KEY`).

## 9. Relation to Ecosystem

```
openmirai-engine (open source, MIT)     Mirai Local (free desktop app)
┌──────────────────────────────┐       ┌─────────────────────────────┐
│ Graph execution engine        │       │ Implements Engine            │
│ 50 built-in tools             │◄──────│ Desktop UI for agents        │
│ 7 LLM providers              │  uses │ Local-first, no cloud needed │
│ HTTP API + CLI                │       └─────────────────────────────┘
│ YAML agent specs              │
│ Soul + Universe               │       Mirai Cloud (paid SaaS)
│ Memory + Energy tracking      │       ┌─────────────────────────────┐
│ MCP + Security + Sandbox      │       │ Implements Engine            │
└──────────────────────────────┘       │ 24/7 infrastructure          │
                                        │ Auto-scaling, multi-tenant   │
                                        └─────────────────────────────┘
```

Engine is the core. Local and Cloud consume it as a dependency.
