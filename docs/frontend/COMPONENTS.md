<!--
BLUEPRINT SEED — COMPONENTS.md
Responsable: → blueprint/agents/13-CURATOR.md

Estructura esperada:
1. Compuestos (nivel medio): Table, Form, Modal, Card, ListToolbar, etc.
2. Átomos (nivel micro): Button primary/secondary, Label title/body, Input, Icon, Spinner, Badge.

Pantallas/páginas NO viven aquí — viven en PANTALLAS.md (nivel macro).

Cada entrada: nombre, estado, responsabilidad, API (props + slots), estados visuales, tokens, usos[], creado-en

Reglas:
- Todo componente activo debe listar Usos[] (permite análisis de impacto)
- Cuando pasa a obsoleto, apuntar a su reemplazo
- No repetir tokens aquí — referenciar DESIGN-GUIDE.md
- Describir API + comportamiento, no el código
- Variantes (primary/secondary) van como un solo componente con prop `variant`
- Si un patrón se reutiliza 2+ veces y no está aquí → Curator lo reporta como inconsistencia
-->

# COMPONENTS.md

## Overview

OpenMirai is a **headless agentic engine** — there is no UI component library, no React/Vue components, and no frontend framework built into the core.

The closest analogs to reusable "components" in the OpenMirai ecosystem are:

1. **Built-in Tools** — the 52 composable graph nodes that form the core execution primitives
2. **SDK Clients** — thin language bindings (Python, TypeScript) that expose the engine API
3. **Agent YAML Patterns** — reusable workflow templates and graph compositions

This document points to each.

---

## 1. Built-in Tools (Graph Nodes)

Every agent is a directed graph of nodes. Each node executes a **tool**. Tools are the true reusable components — composable, modular, with clear inputs/outputs.

**All 52 built-in tools are documented in [README.md § Built-in Tools](../../README.md#built-in-tools) and [USAGE.md § Tool Catalog](../../USAGE.md#4-tool-catalog-what-nodes-can-do).**

### Tool Categories

| Category | Count | Purpose |
|----------|-------|---------|
| **Trigger** | 5 | Entry points: `webhook`, `manual`, `schedule`, `event`, `heartbeat` |
| **AI** | 4 | LLM interactions: `llm_call`, `embeddings`, `transcribe`, `claude_code` |
| **Logic** | 7 | Control flow: `condition`, `switch`, `loop`, `merge`, `wait`, `human_input`, `deadline` |
| **Data** | 11 | Storage/retrieval: db_read, db_write, storage_read, storage_write, vault_read, vault_write, entity_query, entity_upsert, web_scrape, html_to_markdown, rag_search |
| **Filesystem** | 12 | File operations: read_file, write_file, edit_file, glob_files, grep_files, list_dir, tree, copy, move, delete, mkdir, file_info |
| **System** | 3 | System execution: `bash`, `process_list`, `sandbox_exec` |
| **Git** | 4 | Version control: `status`, `diff`, `log`, `commit` |
| **Output** | 1 | Result formatting: `response` |
| **Agent** | 1 | Sub-agent composition: `run_agent` |
| **MCP** | 1 | External tool servers: `mcp/call` |
| **State** | 1 | Memory persistence: `memory` |

### Tool API Pattern

Every tool has a consistent node structure:

```yaml
- id: unique_node_id
  tool_type: category/tool_name
  config:
    # Tool-specific settings
    # See USAGE.md for each tool's config schema
```

Each tool defines:
- **Inputs** — what fields it accepts (via `data_map` from upstream nodes)
- **Config** — tool-specific parameters (temperature, paths, queries, etc.)
- **Outputs** — what fields downstream nodes can access

---

## 2. SDK Clients (Language Bindings)

### Python SDK

**Location:** `sdks/python/`  
**Install:** `pip install openmirai`  
**Docs:** [sdks/python/README.md](../../sdks/python/README.md)

The Python SDK is a thin client over the HTTP API. Use it to:
- Load agents from YAML
- Execute agents programmatically
- Stream execution events (Server-Sent Events)

```python
from openmirai import Engine, Agent

engine = Engine(provider="ollama")
agent = Agent.from_file("my-agent.yaml")
result = engine.run(agent, input={"query": "hello"})
print(result.output)
```

### TypeScript SDK

**Location:** `sdks/typescript/`  
**Install:** `npm install openmirai`  
**Docs:** [sdks/typescript/README.md](../../sdks/typescript/README.md)

The TypeScript SDK provides:
- Agent loading and execution
- Full streaming support
- Node 18+ compatibility

```typescript
import { Engine, Agent } from 'openmirai';

const engine = new Engine({ provider: 'ollama' });
const agent = Agent.fromFile('my-agent.yaml');
const result = await engine.run(agent, { input: { query: 'hello' } });
console.log(result.output);

// Stream execution
for await (const event of engine.stream(agent, { input: { query: 'hello' } })) {
  console.log(event.event, event.data);
}
```

---

## 3. Agent YAML Patterns (Workflow Reuse)

Common workflow patterns are documented as **YAML templates** in [USAGE.md § Common Patterns](../../USAGE.md#5-common-patterns):

- **Ask LLM → Return answer** — single-node reasoning
- **Classify → Route** — multi-way conditional branching
- **Read file → Analyze** — sequential data transformation
- **Sub-agent composition** — nested agent calls (max depth 3)

These patterns can be extracted into reusable agent templates and composed via `agent/run_agent` nodes.

---

## 4. Graph Composition Primitives

### Nodes as Components

Each node is a unit of composition:
- **Deterministic** — same input = same output
- **Stateless** (by default) — no internal state between calls
- **Typed** — clear input/output schema via `inputs` and `outputs` declarations
- **Retryable** — configurable retry logic per node

### Edges as Connectors

Edges connect nodes and pass data:
- **data_map** — field mapping between nodes (supports dot notation for nested access)
- **condition** — route based on node output values
- **fallback** — unconditional edge as default path

Example:
```yaml
edges:
  - source: classify
    target: support_handler
    condition:
      field: response
      op: contains
      value: "support"
  - source: classify
    target: default_handler    # fallback
```

---

## 5. No UI Components

**OpenMirai has no frontend, no UI framework, and no visual components.**

- ✅ Use the **HTTP API** (`/api/v1/*` endpoints) to build custom UIs
- ✅ Use the **Python or TypeScript SDKs** to embed agent execution in existing apps
- ✅ Use the **CLI** (`mirai run`, `mirai serve`) for terminal-based interaction
- ❌ Do not expect React components, Vue components, or any UI library

If you need a web frontend for your agents, you must build it separately (using your preferred framework) and call the engine's HTTP API.

---

## 6. Extending with MCP Servers

The **Model Context Protocol (MCP)** allows you to plug in external tool servers at runtime.

Instead of building custom UI components, extend the engine with MCP servers:

```yaml
mcp_servers:
  - name: my-tools
    transport: stdio
    command: npx
    args: ["-y", "@my-org/mcp-server"]

graph:
  nodes:
    - id: call_tool
      tool_type: mcp/call
      config:
        server_name: my-tools
        tool_name: search_documents
```

See [USAGE.md § MCP Servers](../../USAGE.md#10-mcp-servers-external-tools) for details.

---

## 7. Further Reading

- **Complete tool reference:** [USAGE.md](../../USAGE.md)
- **Builtin tools list:** [README.md § Built-in Tools](../../README.md#built-in-tools)
- **Architecture overview:** [docs/ARCHITECTURE.md](../ARCHITECTURE.md)
- **Agent examples:** `examples/`
