# Graph Engineering

> The workflow graph is the artifact you engineer — not a by-product of the code that happens
> to build it.

Most agent frameworks ask you to *program* a graph: you import a library, instantiate a builder,
register Python functions as nodes, wire edges with method calls, and compile. The graph exists
only while that process is alive. To read it, you read the code. To move it, you move the code —
along with its interpreter, its dependency tree and the framework itself.

Graph Engineering inverts that. The graph is a **plain-text file** that fully describes the
workflow: its nodes, its edges, the data that flows along them, the conditions that route it, the
retries, and the input/output contract. You author it, diff it, review it in a pull request,
validate it in CI, tag it with a version, and hand it to a runtime that executes it. It is
source, but declarative — and it does not carry a language or a framework on its back.

OpenMirai is an engine built for that model. YAML in, execution out.

---

## The four properties

A graph is an engineerable artifact when all four hold. Drop any one and you are back to
programming a framework.

### 1. Complete — the file is the program, not its configuration

Everything the run needs is in the file. There is no host function to implement, no class to
subclass, no decorator to remember. Below is a complete routing agent — the entire program, lifted
from [`examples/conditional-routing.yaml`](../examples/conditional-routing.yaml) (only two
`temperature` / `max_tokens` lines trimmed for width):

```yaml
name: conditional-router
version: v1
description: "Classify input and route to the right handler"

inputs:
  query:
    type: text
    required: true
    description: "The user's message to classify and handle"

graph:
  nodes:
    - id: start
      tool_type: trigger/manual

    - id: classify
      tool_type: ai/llm_call
      config:
        prompt: |
          Classify the following user message into exactly one category.
          Reply with ONLY the category name, nothing else.
          Categories: support, billing, technical
        temperature: 0.0
        max_tokens: 20

    - id: check
      tool_type: logic/condition
      config:
        operator: equals
        value: support

    - id: support_handler
      tool_type: ai/llm_call
      config:
        prompt: "You are a friendly support agent. Help the user with their issue."

    - id: default_handler
      tool_type: ai/llm_call
      config:
        prompt: "You are a helpful assistant. Answer the user's question."

    - id: respond
      tool_type: output/response

  edges:
    - source: start
      target: classify
      data_map:
        context: "start.payload.query"

    - source: classify
      target: check
      data_map:
        field: "classify.response"

    - source: check
      target: support_handler
      condition: { field: result, op: equals, value: true }

    - source: check
      target: default_handler
      condition: { field: result, op: equals, value: false }

    - source: support_handler
      target: respond

    - source: default_handler
      target: respond
```

```bash
mirai run examples/conditional-routing.yaml --input '{"query": "My order has not arrived"}'
```

Note what is *in* the file that usually lives in code: the branch predicate (`condition`), the
data plumbing between nodes (`data_map`), the LLM parameters, and the declared input contract.
Nothing is imported. Nothing is registered at startup.

### 2. Runtime-independent — one binary, no language underneath

The engine is a single self-contained executable with no runtime dependencies: no interpreter, no
`node_modules`, no virtualenv, no Docker Compose, no cluster. The same file runs on a laptop, in
a CI job, on a server, on an edge box, or embedded in an app — driven by the CLI, the HTTP API, an
SDK, or the Rust crate directly.

```
Graph  = YAML file      (portable, versionable, language-agnostic)
Engine = single binary  (CLI · HTTP · Rust crate · Python SDK)
Host   = your app       (any language that can run a process or call HTTP)
```

The graph declares **what** happens. The engine decides **how** it runs.

### 3. Checkable — the graph is a CI gate before it is a run

Because the workflow is data, it can be verified without executing it. `mirai validate` parses the
spec against the typed schema and rejects duplicate node or edge ids, edges pointing at nodes that
do not exist, self-loops, cycles, empty graphs, and incoherent schedules — no API keys, no network,
no model calls:

```bash
mirai validate examples/conditional-routing.yaml
# ✓ Valid agent spec: 'conditional-router' (6 nodes, 6 edges)
```

```yaml
# .github/workflows/graphs.yml
- run: cargo build --release
- run: |
    for f in agents/*.yaml; do
      ./target/release/mirai validate "$f" || exit 1
    done
```

Be precise about what this catches: `validate` checks the **graph**, not the tool registry. A node
whose `tool_type` is not registered still validates and fails at execution with `tool not found`.
For a keyless smoke test that actually walks the graph, run it against the mock provider:

```bash
mirai run agents/my-graph.yaml --provider mock --input '{"query": "ping"}'
```

The input contract (`inputs:` — types, `required`, defaults) is enforced when the run starts, so a
missing or mistyped input fails before the first node executes rather than somewhere in the middle.
A broken graph fails the pull request, not production. Every file under [`examples/`](../examples)
is held to this gate.

### 4. Reviewable — a diff a human can read

A changed prompt, a rerouted edge, a raised `max_tokens` — each is a one-line diff with a blast
radius you can see. There is no indirection between "what the reviewer reads" and "what the engine
runs", because they are the same bytes. Graphs live in git next to the code that calls them, and
roll back the same way.

---

## How this compares, honestly

Declarative workflow files are not new, and OpenMirai is not the only project with YAML. The
useful question is not *"does it have a YAML mode?"* but **how much of the workflow lives in the
file, and what does it take to run that file.**

| | Where the graph lives | What it takes to run it | Notes |
|---|---|---|---|
| **OpenMirai** | Complete YAML file | One static binary, zero runtime deps | Validate without running; embed via CLI/HTTP/crate/SDK |
| **LangGraph** | Python/JS code (`StateGraph`, `add_node`, `add_edge`, `compile`) | Python or Node runtime + dependency tree | No declarative graph format executed by the runtime; the code *is* the serialization format |
| **Google ADK — Agent Config** | YAML, partially | ADK runtime (Python/Java/Go) | Real declarative mode, but **experimental**: Gemini models only, a fixed allow-list of tools, and anything programmable still requires Python or Java |
| **CrewAI** | `agents.yaml` / `tasks.yaml` + `crew.py` | Python runtime | The YAML configures roles and tasks; assembling the crew still needs `@CrewBase` / `@agent` / `@task` in Python |
| **Dify** | Exportable DSL (YAML) | A Dify deployment | Portable *between Dify instances* — the file means nothing outside the platform |
| **n8n** | Exportable workflow JSON | An n8n instance (Node) | Same shape of portability: instance-to-instance, not runtime-independent |

Three honest points about that table:

1. **"Declarative" is table stakes now.** ADK, CrewAI, Dify and n8n all have a file. The
   difference is completeness (how much still needs code) and independence (how much still needs
   *their* runtime).
2. **OpenMirai's YAML is a documented, validated, versioned schema — not a ratified industry
   standard.** There is no committee behind it and no second implementation. The claim is
   narrower and checkable: it is ordinary YAML, it holds the whole workflow, and the one thing
   needed to run it is a dependency-free binary you can drop anywhere.
3. **You give things up.** LangGraph's Python ecosystem, LangSmith-grade visual tracing, and
   dynamic runtime fan-out (`Send`) are real advantages OpenMirai does not match today — the open
   gaps are tracked in [ROADMAP-PARITY.md](ROADMAP-PARITY.md). Graph Engineering buys portability,
   reviewability and a small blast radius. If your workflow is mostly bespoke Python, a code-first
   framework is the honest recommendation.

---

## When Graph Engineering pays off

- **You ship the same workflow to more than one place** — laptop, CI, server, edge, a customer's
  machine — and you do not want to ship a Python environment with it.
- **Your host app is not Python.** Swift, Go, Rust, TypeScript, a shell script: if it can run a
  process or call HTTP, it can run a graph.
- **Non-authors need to read the workflow.** A reviewer, an operator, or a teammate can follow a
  YAML graph without learning a framework's object model.
- **Changes must be auditable.** Prompts and routing are configuration you can diff, tag and roll
  back — not lines buried in a service.
- **You want a static gate.** Validating a hundred graphs in CI costs nothing and needs no keys.

And when it does not: heavy bespoke logic per node, or a hard dependency on Python-only libraries.
The engine's escape hatch is MCP and the built-in tool catalog — but if most of the work is custom
code, write custom code.

---

## Where to go next

- [USAGE.md](../USAGE.md) — the spec in practice: nodes, edges, `data_map`, conditions, contracts
- [docs/SCOPE.md](SCOPE.md) — what the engine deliberately does *not* do
- [docs/ARCHITECTURE.md](ARCHITECTURE.md) — how the engine executes a graph
- [examples/](../examples) — runnable graphs, smallest first
- [examples/showcase/](../examples/showcase) — graphs pulled out of real apps

## References

- LangGraph graph API (code-first `StateGraph`): <https://docs.langchain.com/oss/python/langgraph/graph-api>
- `langgraph-gen` (YAML spec → Python/TS *stubs*, not executed at runtime): <https://github.com/langchain-ai/langgraph-gen-py>
- Google ADK Agent Config, incl. the experimental limitations: <https://adk.dev/agents/config/>
- CrewAI YAML configuration: <https://docs.crewai.com/en/concepts/agents>
- Dify DSL export/import: <https://docs.dify.ai/en/use-dify/workspace/app-management>
- n8n workflow export/import: <https://docs.n8n.io/workflows/export-import/>
