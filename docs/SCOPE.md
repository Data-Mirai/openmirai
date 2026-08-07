# Scope — what OpenMirai is, and what it is not

OpenMirai is a **workflow execution engine**. You write a workflow as a YAML graph, and the
engine runs it: resolves inputs, walks the DAG, calls tools and LLMs, retries what fails,
records what happened, and hands you the result. Anywhere — laptop, server, edge.

That is the whole job.

## In scope

- **Executing YAML workflows**: DAG traversal, conditional branching, concurrent fan-out/fan-in,
  retry with backoff, failure modes, iteration limits.
- **Tools**: the built-in catalog plus anything reachable over MCP.
- **LLM access**: several providers behind one interface, local models included. No lock-in.
- **Execution state**: where a run is, pausing it, resuming it, cancelling it.
- **Telemetry of a run**: trace per node, timings, tokens, cost, events, and persistence so a
  run survives a restart.
- **Ways to drive it**: CLI, HTTP API, SDKs, or embedded as a Rust crate.

## Out of scope

These are **not** engine concerns, and pull requests adding them will be redirected:

- **Agents as first-class entities** — identity, roles, teams, delegation.
- **Goals / objectives** and their lifecycle.
- **Agent memory** — long-term recall, learning across runs, knowledge that outlives an execution.
- **Multi-user collaboration**, workspaces, projects, tenancy, billing.

The engine has no opinion about who or what asked for a workflow to run, or what should be
remembered afterwards. It runs the graph and reports back.

## Why the line is drawn here

A workflow engine that does one thing well is easy to reason about, easy to embed, and easy to
replace — which is exactly what makes it safe to depend on. The moment an engine also decides
what an agent is, how it remembers, and how a team coordinates, it stops being infrastructure
and becomes a framework you have to adopt wholesale.

Everything above the line belongs to the layer that builds on top. At Data Mirai that layer is
**DataMirai**, but the boundary is the point: you can build your own layer on this engine, and
nothing in here assumes ours.

## Where the line actually falls, in practice

| Question | Answer |
|---|---|
| "Pause this run and resume it tomorrow" | Engine — it is execution state |
| "How many tokens did node 4 burn?" | Engine — it is run telemetry |
| "Remember that this client prefers Meta over TikTok" | Not the engine |
| "This agent learned something; apply it next time" | Not the engine |
| "Show me what everyone on the team is working on" | Not the engine |

Rule of thumb: if the capability makes sense to someone who **only wants to run a YAML graph**
and has never heard the word *agent*, it belongs here. Otherwise it belongs one layer up.
