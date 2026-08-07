# How to Use OpenMirai

## The concept in 30 seconds

An **agent** is a YAML file. It defines a graph of nodes connected by edges. Each node does one thing (call an LLM, read a file, evaluate a condition). The engine walks the graph node by node, passing data between them.

```
YAML file → mirai run → output
```

That's it. No Python. No Docker. No config files. One binary, one YAML.

---

## 1. Your first agent

Create `my-agent.yaml`:

```yaml
name: my-first-agent
version: v1
description: "Answer a question using an LLM"

# Declare what the host must provide
inputs:
  question:
    type: text
    required: true
    description: "The question to answer"

graph:
  nodes:
    - id: start
      tool_type: trigger/manual

    - id: think
      tool_type: ai/llm_call
      config:
        prompt: "Answer the user's question in 2 sentences."

    - id: done
      tool_type: output/response

  edges:
    - source: start
      target: think
      data_map:
        context: "start.payload.question"

    - source: think
      target: done
```

Run it:

```bash
mirai run my-agent.yaml --input '{"question": "What is Rust?"}'
```

What happens:
1. `start` receives your input → produces `user_input`
2. `think` gets the question via `data_map` → calls the LLM → produces `response`
3. `done` formats the final output

---

## 2. The anatomy of a node

```yaml
- id: my_node                    # unique name
  tool_type: ai/llm_call         # what this node does
  config:                         # settings for this tool
    prompt: "Summarize this text"
    temperature: 0.5
    max_tokens: 256
```

Every node has:
- **id** — unique name you reference in edges
- **tool_type** — which tool to execute (see tool catalog below)
- **config** — settings specific to that tool

---

## 3. Connecting nodes (edges)

```yaml
edges:
  - source: node_a
    target: node_b
```

That's a simple connection. Node B runs after Node A.

### Passing data between nodes

```yaml
edges:
  - source: node_a
    target: node_b
    data_map:
      prompt_text: "node_a.response"    # node_b receives node_a's response as "prompt_text"
```

`data_map` maps **target input name** → **source.field**. Supports nested dot notation: `"node_a.payload.nested.field"`.

### Conditional routing

```yaml
edges:
  - source: classifier
    target: happy_path
    condition:
      field: result
      op: equals
      value: true

  - source: classifier
    target: sad_path
    condition:
      field: result
      op: equals
      value: false
```

Only ONE conditional edge fires (first match wins). If NO condition matches, unconditional edges are the fallback.

### Available operators

| Operator | What it does | Example |
|----------|-------------|---------|
| `equals` | exact match | `value: "active"` |
| `not_equals` | not equal | `value: "deleted"` |
| `greater_than` | > | `value: 80` |
| `less_than` | < | `value: 0` |
| `greater_or_equal` | >= | `value: 18` |
| `less_or_equal` | <= | `value: 100` |
| `contains` | string/array contains | `value: "error"` |
| `in` | value is in array | `value: ["active", "pending"]` |

Short forms also work: `eq`, `neq`, `gt`, `lt`, `gte`, `lte`.

---

## 4. Tool catalog (what nodes can do)

### Triggers (entry points)

| Tool | What it does |
|------|-------------|
| `trigger/manual` | Receives input from CLI or API call |
| `trigger/webhook` | Graph entry for a host-delivered webhook payload; current webhook routing is a placeholder |
| `trigger/schedule` | Graph entry for host-delivered schedule data; cron is not implemented |
| `trigger/event` | Graph entry for an event supplied by a host |
| `trigger/heartbeat` | Graph entry for a host-delivered heartbeat |

### AI

| Tool | What it does |
|------|-------------|
| `ai/llm_call` | Call any LLM (config: prompt, model, temperature, max_tokens, system_prompt, output_schema) |
| `ai/embeddings` | Generate text embeddings |
| `ai/transcribe` | Speech-to-text |
| `ai/claude_code` | Agentic code generation via Claude (requires an Anthropic API key) |

### Logic

| Tool | What it does |
|------|-------------|
| `logic/condition` | Evaluate a boolean condition (input `field` = value to test, via data_map; config: operator, value) |
| `logic/switch` | Multi-way routing (switch/case) |
| `logic/loop` | Repeat until condition met |
| `logic/merge` | Combine inputs from multiple branches (fan-in) |
| `logic/wait` | Pause for N seconds |
| `logic/human_input` | Pause and wait for human decision |
| `logic/deadline` | Timeout guard — fail if exceeded |

### Data

| Tool | What it does |
|------|-------------|
| `data/db_read` | Read from SQLite |
| `data/db_write` | Write to SQLite |
| `data/entity_query` | Query/retrieve structured entities |
| `data/entity_upsert` | Store/update structured entities |
| `data/storage_read` | Read a file from configured storage (filesystem or in-memory) |
| `data/storage_write` | Write a file to configured storage (filesystem or in-memory) |
| `data/vault_read` | Read a secret from the vault |
| `data/vault_write` | Write a secret to the vault |
| `data/web_scrape` | Fetch and parse a web page |
| `data/html_to_markdown` | Convert HTML into clean Markdown |
| `data/rag_search` | Semantic search with embeddings |

### Filesystem

| Tool | What it does |
|------|-------------|
| `filesystem/read_file` | Read a local file |
| `filesystem/write_file` | Write a local file |
| `filesystem/edit_file` | Edit a local file (patch) |
| `filesystem/glob_files` | Find files by pattern |
| `filesystem/grep_files` | Search file contents |
| `filesystem/list_dir` | List directory contents |
| `filesystem/tree` | Tree view of directory |
| `filesystem/copy` | Copy file or directory |
| `filesystem/move` | Move/rename file or directory |
| `filesystem/delete` | Delete file or directory |
| `filesystem/mkdir` | Create directory |
| `filesystem/file_info` | Get file metadata (size, modified, etc.) |

### System

| Tool | What it does |
|------|-------------|
| `system/bash` | Execute a shell command |
| `system/process_list` | List running processes |
| `system/sandbox_exec` | Run code in isolated sandbox |

### Git

| Tool | What it does |
|------|-------------|
| `git/status` | Show working tree status |
| `git/diff` | Show changes |
| `git/log` | Show commit history |
| `git/commit` | Create a commit |

### Output

| Tool | What it does |
|------|-------------|
| `output/response` | Format and return the final result |

### Agent

| Tool | What it does |
|------|-------------|
| `agent/run_agent` | Execute another agent as a sub-agent (max nesting depth: 3) |

### MCP (Model Context Protocol)

| Tool | What it does |
|------|-------------|
| `mcp/call` | Call a tool on an MCP server |

### State

| Tool | What it does |
|------|-------------|
| `state/memory` | Persist agent memory between cycles (only keys declared in `graph.memory`) |

---

## 5. Common patterns

### Pattern: Ask LLM → Return answer

```yaml
nodes:
  - id: trigger
    tool_type: trigger/manual
  - id: llm
    tool_type: ai/llm_call
    config:
      prompt: "You are a helpful assistant."
  - id: out
    tool_type: output/response
edges:
  - source: trigger
    target: llm
    data_map:
      context: "trigger.user_input"
  - source: llm
    target: out
```

### Pattern: Classify → Route to different handlers

```yaml
nodes:
  - id: trigger
    tool_type: trigger/manual
  - id: classify
    tool_type: ai/llm_call
    config:
      prompt: "Classify as: support, billing, or technical. Reply with ONE word."
  - id: support_handler
    tool_type: ai/llm_call
    config:
      prompt: "You are a support agent. Help the user."
  - id: default_handler
    tool_type: ai/llm_call
    config:
      prompt: "You are a general assistant."
  - id: out
    tool_type: output/response
edges:
  - source: trigger
    target: classify
  - source: classify
    target: support_handler
    condition:
      field: response
      op: contains
      value: "support"
  - source: classify
    target: default_handler
  - source: support_handler
    target: out
  - source: default_handler
    target: out
```

Note: the unconditional edge to `default_handler` acts as fallback when the condition doesn't match.

### Pattern: Read file → Analyze with LLM

```yaml
nodes:
  - id: trigger
    tool_type: trigger/manual
  - id: read
    tool_type: filesystem/read_file
  - id: analyze
    tool_type: ai/llm_call
    config:
      prompt: "Analyze this text and provide a summary."
  - id: out
    tool_type: output/response
edges:
  - source: trigger
    target: read
    data_map:
      path: "trigger.file_path"
  - source: read
    target: analyze
    data_map:
      context: "read.content"
  - source: analyze
    target: out
```

### Pattern: Sub-agent composition

```yaml
nodes:
  - id: trigger
    tool_type: trigger/manual
  - id: research
    tool_type: agent/run_agent
    config:
      agent_file: "agents/researcher.yaml"
  - id: summarize
    tool_type: ai/llm_call
    config:
      prompt: "Summarize the research findings."
  - id: out
    tool_type: output/response
edges:
  - source: trigger
    target: research
    data_map:
      query: "trigger.payload.topic"
  - source: research
    target: summarize
    data_map:
      context: "research.response"
  - source: summarize
    target: out
```

---

## 6. CLI commands

```bash
# Run an agent
mirai run agent.yaml
mirai run agent.yaml --input '{"key": "value"}'
mirai run agent.yaml --provider ollama --model gemma4
mirai run agent.yaml --provider claude --api-key $ANTHROPIC_API_KEY

# Validate without running
mirai validate agent.yaml

# Start HTTP server
mirai serve --port 3000
MIRAI_API_KEY=secret mirai serve --port 3000

# Interactive mode (setup wizard + chat)
mirai

# Show version
mirai version
```

### LLM providers

| Provider | Flag | Needs API key? |
|----------|------|---------------|
| Ollama (default) | `--provider ollama` | No (local) |
| Claude | `--provider claude --api-key $ANTHROPIC_API_KEY` | Yes |
| OpenAI | `--provider openai --api-key $OPENAI_API_KEY` | Yes |
| Gemini | `--provider gemini --api-key $GOOGLE_API_KEY` | Yes |
| Groq | `--provider groq --api-key $GROQ_API_KEY` | Yes |
| NVIDIA NIM | `--provider nvidia --api-key $NVIDIA_API_KEY` | Yes |
| OpenRouter | `--provider openrouter --api-key $OPENROUTER_API_KEY` | Yes |

Provider resolution order: `--provider` flag → environment variable → auto-detect running Ollama → default Ollama.

---

## 7. Input validation (optional but recommended)

```yaml
name: validated-agent
version: v1

inputs:
  question:
    type: text
    required: true
    description: "The question to answer"
  language:
    type: text
    required: false
    default: "english"

outputs:
  answer:
    type: text
    description: "The generated answer"

graph:
  nodes: [...]
  edges: [...]
```

Supported types: `text`, `number`, `integer`, `boolean`, `array`, `object`, `file`.

If someone runs this agent without `question`, they get a clear error:
```
input validation failed: missing required input: question (The question to answer)
```

---

## 8. Structured output (LLM returns validated JSON)

Force the LLM to respond with a specific JSON structure:

```yaml
- id: extract
  tool_type: ai/llm_call
  config:
    prompt: "Extract the person's name and age from this text."
    output_schema:
      type: object
      properties:
        name:
          type: string
        age:
          type: integer
      required: ["name", "age"]
```

The engine injects schema instructions into the prompt and auto-retries if the LLM returns invalid JSON.

---

## 9. Retry and error handling

Agent-level retry fields are part of AgentSpec but the default CLI/server do
not currently apply them to `GraphRunner`. Configure the behavior that is
actually wired per node with `config.retry_policy`:

```yaml
name: resilient-agent
version: v1

graph:
  nodes:
    - id: unstable
      tool_type: ai/llm_call
      config:
        prompt: "Try this operation"
        retry_policy:
          max_retries: 3
          backoff: exponential    # or: linear, none
          on_failure: stop        # or: skip, route_to_error
  edges: [...]
```

| `on_failure` | Behavior |
|--------------|----------|
| `stop` | Abort execution (default) |
| `skip` | Continue as if the node succeeded (empty output) |
| `route_to_error` | Follow an edge with `condition: { error: true }` |

---

## 10. MCP servers (external tools)

Connect to Model Context Protocol servers for additional tools:

```yaml
name: agent-with-mcp
version: v1

config:
  mcp_servers:
    - name: my-tools
      transport: stdio
      command: npx
      args: ["-y", "@my-org/mcp-server"]

graph:
  nodes:
    - id: trigger
      tool_type: trigger/manual
    - id: call_tool
      tool_type: mcp/call
      config:
        server_name: my-tools
        tool_name: search_documents
    - id: out
      tool_type: output/response
  edges:
    - source: trigger
      target: call_tool
      data_map:
        arguments: "trigger.payload"
    - source: call_tool
      target: out
```

Supported transports: `stdio` (local process) and `http` (remote server).

---

## 11. Deploying as a server

```bash
# Start with auth
MIRAI_API_KEY=my-secret mirai serve --port 3000

# Call the API
curl -X POST http://localhost:3000/api/v1/agents/from-spec \
  -H "Content-Type: application/json" \
  -H "X-API-Key: my-secret" \
  -d @agent.yaml

# Execute
curl -X POST http://localhost:3000/api/v1/agents/{id}/execute \
  -H "Content-Type: application/json" \
  -H "X-API-Key: my-secret" \
  -d '{"trigger_data": {"question": "hello"}}'

# Stream (Server-Sent Events)
curl -N http://localhost:3000/api/v1/agents/{id}/stream \
  -H "Content-Type: application/json" \
  -H "X-API-Key: my-secret" \
  -d '{"trigger_data": {"question": "hello"}}'
```

### SSE events

When streaming, you receive real-time events:

```
event: graph.started
data: {"graph_name": "my-agent", "node_count": 3}

event: node.started
data: {"node_id": "think", "tool_type": "ai/llm_call"}

event: node.token
data: {"node_id": "think", "token": "Hello"}

event: node.completed
data: {"node_id": "think", "duration_ms": 1500}

event: graph.completed
data: {"status": "Completed", "total_duration_ms": 2100}
```

---

## 12. Multimodal — audio, images, video (v0.5.0)

Send files directly to LLMs that support multimodal input.

**Transcribe audio:**
```yaml
inputs:
  audio_path:
    type: file
    required: true

graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: stt
      tool_type: ai/transcribe
      config:
        model: "gemini-2.5-flash"
    - id: done
      tool_type: output/response
  edges:
    - source: start
      target: stt
      data_map:
        file_path: "start.payload.audio_path"
    - source: stt
      target: done
```

**Analyze an image:**
```yaml
- id: vision
  tool_type: ai/llm_call
  config:
    prompt: "Describe this image."
    model: "gemini-2.5-flash"
# edge data_map: media_path: "start.payload.image_path"
```

Supported: `.m4a .mp3 .wav .ogg .flac .mov .mp4 .webm .png .jpg .webp .gif`

Providers: Gemini (audio+video+image), Claude (image), OpenAI/Groq (image), Ollama (image).

The engine reads the file, base64-encodes it, validates the MIME type against the provider, and sends it as native multimodal content. The agent YAML never touches base64 — just pass a file path.

---

## Quick reference

```
agent.yaml    = graph definition (nodes + edges)
mirai run     = execute locally
mirai serve   = HTTP API
mirai validate = check syntax
mirai tools   = list all tools with inputs/outputs/config

node       = one action (tool_type)
edge       = connection between nodes
data_map   = pass data between nodes (source.field → target input)
condition  = route based on output values
inputs     = typed input contract (validation at boundary)
outputs    = typed output contract (descriptive in AgentSpec v1)
config     = agent-level declarations; see AGENT_SPEC.md for current host wiring
config.mcp_servers = external tool servers (Model Context Protocol)
media_path = attach a file to an LLM call (v0.5.0)
output_files = declare files a bash command generates (v0.5.0)
```
