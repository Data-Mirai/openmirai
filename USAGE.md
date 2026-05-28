# How to Use Data Mirai Engine

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
        context: "start.user_input"

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
- **tool_type** — which tool to execute (see tool table below)
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

`data_map` maps **target input name** → **source.field**.

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
| `trigger/webhook` | Receives input from HTTP webhook |
| `trigger/schedule` | Fires on a time interval |

### AI
| Tool | What it does |
|------|-------------|
| `ai/llm_call` | Call any LLM (config: prompt, model, temperature, max_tokens) |
| `ai/embeddings` | Generate text embeddings |
| `ai/transcribe` | Speech-to-text |

### Logic
| Tool | What it does |
|------|-------------|
| `logic/condition` | Evaluate a boolean condition (config: field, op, value) |
| `logic/switch` | Multi-way routing |
| `logic/loop` | Repeat until condition |
| `logic/merge` | Combine inputs (pass-through) |
| `logic/wait` | Pause for N seconds |
| `logic/human_input` | Pause and wait for human decision |

### Data
| Tool | What it does |
|------|-------------|
| `data/db_read` | Read from SQLite |
| `data/db_write` | Write to SQLite |
| `data/storage_read` | Read a file from storage |
| `data/storage_write` | Write a file to storage |
| `data/web_scrape` | Fetch and parse a web page |
| `data/rag_search` | Semantic search with embeddings |

### Filesystem
| Tool | What it does |
|------|-------------|
| `filesystem/read_file` | Read a local file |
| `filesystem/write_file` | Write a local file |
| `filesystem/glob` | Find files by pattern |
| `filesystem/grep` | Search file contents |

### System
| Tool | What it does |
|------|-------------|
| `system/bash` | Execute a shell command |
| `system/sandbox_exec` | Run code in isolated sandbox |

### Output
| Tool | What it does |
|------|-------------|
| `output/response` | Format and return the final result |

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
  - id: check
    tool_type: logic/condition
    config:
      field: response
      op: contains
      value: "support"
  - id: support_handler
    tool_type: ai/llm_call
    config:
      prompt: "You are a support agent. Help the user."
  - id: general_handler
    tool_type: ai/llm_call
    config:
      prompt: "You are a general assistant."
edges:
  - source: trigger
    target: classify
  - source: classify
    target: check
  - source: check
    target: support_handler
    condition:
      field: result
      op: equals
      value: true
  - source: check
    target: general_handler
    condition:
      field: result
      op: equals
      value: false
```

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

If someone runs this agent without `question`, they get a clear error:
```
input validation failed: missing required input: question (The question to answer)
```

---

## 8. Deploying as a server

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
```

---

## Quick reference

```
agent.yaml = graph definition (nodes + edges)
mirai run   = execute locally
mirai serve = HTTP API
mirai validate = check syntax

node = one action (tool_type)
edge = connection between nodes
data_map = pass data between nodes
condition = route based on output values
```
