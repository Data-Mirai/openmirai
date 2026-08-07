# AgentSpec v1 Reference

`AgentSpec` is the declarative contract consumed by OpenMirai. This document
describes the fields accepted by version 0.7.0, their defaults, validation, and
whether the default CLI/server actually apply them. The machine-readable
companion is [`schemas/agent-spec.schema.json`](../schemas/agent-spec.schema.json).

## 1. Format and transport

- Files loaded by `mirai run`, `validate`, `describe`, and the Rust
  `AgentSpec::from_file()` API must end in `.yaml` or `.yml`.
- `POST /api/v1/agents/from-spec` carries the same structure as JSON because it
  is an HTTP JSON request.
- The current Serde model does not reject unknown keys. A misspelled field can
  therefore be ignored. Validate with `mirai validate` and do not treat the
  JSON Schema as runtime-enforced `additionalProperties: false` behavior.
- `version` is the AgentSpec format identifier and currently defaults to `v1`.
  It is not the same as the OpenMirai binary version or node tool version.

## 2. Complete example

The same file is available as
[`examples/support-summary.yaml`](../examples/support-summary.yaml).

```yaml
name: support-summary
description: Summarize a support request
version: v1
agent_type: managed
system_prompt: Be concise and preserve facts.

inputs:
  question:
    type: text
    required: true
    description: Support request to summarize
  max_words:
    type: number
    default: 120

outputs:
  answer:
    type: text
    description: Final response

graph:
  memory:
    persist: none
    keys:
      last_topic: null
  nodes:
    - id: start
      tool_type: trigger/manual
      config: {}
    - id: answer
      tool_type: ai/llm_call
      config:
        prompt: "Summarize in ${start.payload.max_words} words: ${start.payload.question}"
    - id: respond
      tool_type: output/response
      config: {}
  edges:
    - source: start
      target: answer
      data_map:
        prompt: "Summarize in ${start.payload.max_words} words: ${start.payload.question}"
    - source: answer
      target: respond
      data_map:
        message: answer.response

triggers:
  - type: manual

config:
  max_iterations: 50
  timeout_ms: 60000
  retry:
    max_retries: 3
    backoff: exponential
    on_failure: stop
  hooks: {}
  mcp_servers: []
  vault_refs: []

resources: []
metadata:
  owner: platform-team
```

## 3. Top-level fields

| Field | Type | Default | Current behavior |
|---|---|---|---|
| `name` | string | required | Graph name and agent identity. |
| `description` | string | `""` | Descriptive metadata. |
| `version` | string | `v1` | Copied to `GraphDef.version`; no negotiation is performed. |
| `agent_type` | `managed` or `live` | `managed` | Direct CLI rejects `live`; server execute/stream handle managed runs and play/stop is intended for live runs. |
| `system_prompt` | string | absent | Added to the execution context. |
| `soul` | path string | absent | SOUL prompt overrides `system_prompt`; load failure falls back to it. |
| `inputs` | map of input fields | absent | Validated by CLI run and synchronous HTTP execute. Streaming currently skips this validation. |
| `outputs` | map of output fields | absent | Descriptive only in v1; final state is not validated against it. |
| `graph` | graph object | empty graph | Nodes, edges, and optional agent KV memory declaration. Empty graphs fail validation. |
| `schedule` | schedule object | absent | Required for live agents and forbidden for managed agents. |
| `triggers` | trigger array | `[]` | Declarative metadata. Trigger tools/hosts perform the actual delivery. |
| `config` | config object | defaults below | Some fields are contracts but are not wired by the default CLI/server. |
| `resources` | resource-reference array | `[]` | Descriptive resource declarations; host wiring determines availability. |
| `metadata` | JSON object | `{}` | Passed to `GraphDef`; not interpreted by the runner. |

## 4. Input and output contracts

Supported field types are `text`, `number`, `boolean`, `json`, and `file`.
`file` validates as a string path/reference; it does not verify existence at
the AgentSpec boundary. `json` accepts an object or array, not JSON scalars.

Input fields support:

| Field | Required | Default | Meaning |
|---|---:|---|---|
| `type` | yes | — | One of the five types above. |
| `required` | no | `false` | Missing values produce a validation error. |
| `description` | no | `""` | Included in missing-field errors. |
| `default` | no | absent | Applied only to missing, non-required inputs. |

Unknown input-field properties are currently ignored by Serde. Output fields contain only `type` and
`description`; enforce structured LLM output with
`ai/llm_call.config.output_schema`, not top-level `outputs`.

## 5. Graph

### Nodes

| Field | Required | Default | Meaning |
|---|---:|---|---|
| `id` | yes | — | Unique state and trace key. |
| `tool_type` | yes | — | Registry key such as `ai/llm_call`. |
| `version` | no | `1.0.0` | Tool-node version metadata; dispatch currently uses `tool_type`. |
| `config` | no | `{}` | Tool configuration and fallback inputs. |
| `position` | no | `{x: 0, y: 0}` | Editor coordinates. |

### Edges

`source` and `target` must reference nodes. `id` is optional and generated as
`source__target`, with a numeric suffix for repeated pairs. `data_map` maps a
target input name to either a direct `node.field` reference or a template that
contains `${node.field}` expressions. No implicit whole-output pass-through is
performed by the current runner.

An optional condition has `field`, `op`, and JSON `value`. Recommended
operators are `equals`, `not_equals`, `greater_than`, `less_than`,
`greater_or_equal`, `less_or_equal`, `in`, and `contains`; short aliases
`eq`, `neq`, `gt`, `lt`, `gte`, and `lte` are also accepted.

Validation checks empty graphs, duplicate IDs, missing references, and
unconditional self-loops. It does not perform general cycle detection or
reject disconnected components. The runner chooses the first entry node.

## 6. Agent KV memory

Memory is declared at `graph.memory`:

```yaml
graph:
  memory:
    persist: cycle
    keys:
      counter: 0
```

`persist` values are:

- `none`: reset to initial keys for every run;
- `cycle`: carry within a live play session; managed executions behave like
  `none`;
- `execution`: carry across executions until explicitly cleared.

All three modes are process-local in the default server. Even `execution` does
not survive restart or synchronize between replicas.

## 7. Live schedule

```yaml
agent_type: live
schedule:
  interval_seconds: 60
  max_cycles: 100
  on_cycle_error: continue
```

Exactly one of `interval_seconds` or `cron` must be specified.
`interval_seconds` must be at least one. `cron` exists in the data model but is
rejected in v1. `on_cycle_error` is `continue` or `stop`.

The current server creates a scheduler without starting it; read
[Live Agents](backend/LIVE_AGENTS.md) before relying on this feature.

## 8. Triggers

A trigger requires a free-form `type` string and can carry `path`, `method`,
`auth`, `interval_seconds`, `cron`, `event_type`, `source`, and `input_form`.
These declarations do not make events arrive by themselves. The current
webhook endpoint acknowledges requests but does not dispatch matching agents,
and cron scheduling is unavailable.

## 9. Agent config and wiring status

| Field | Default | Default CLI/server wiring |
|---|---|---|
| `max_iterations` | `50` | Declared but not applied; runner default is used. |
| `retry.max_retries` | `3` | Top-level policy not applied. Per-node `config.retry_policy` is applied. |
| `retry.backoff` | `exponential` | Same limitation. |
| `retry.on_failure` | `stop` | Same limitation. |
| `timeout_ms` | `60000` | Not applied. Synchronous HTTP has a separate fixed 300-second wrapper. |
| `hooks` | `{}` | Parsed but no hook-handler factory is wired by default. |
| `mcp_servers` | `[]` | Injected into `mcp/call` nodes. This is the correct MCP location. |
| `vault_refs` | `[]` | Parsed; no automatic vault-resource wiring follows from declaration alone. |

An MCP server has `name`, `transport` (`stdio` by default), optional
`command`, `args`, optional `url`, and optional `credential_ref`. See
[MCP](backend/MCP.md).

## 10. Resource references

Each item has `resource_type`, `name`, and an arbitrary `config` object. The
runner does not instantiate resources from this list. Embedding hosts must map
declarations to concrete `ExecutionContext` resources.

## 11. Validation and execution checklist

1. Use YAML files and keep `version: v1` explicit.
2. Run `mirai validate agent.yaml`.
3. Run `mirai describe agent.yaml` to inspect contracts.
4. Use explicit `data_map` for every cross-node value.
5. Confirm every `tool_type` with `mirai tools`.
6. Test cycles, branching, retries, and fan-out rather than assuming DAG semantics.
7. Treat `outputs`, hooks, top-level retry/timeout, resource references, cron,
   and live scheduling according to the wiring limitations above.

## 12. Sources and related documentation

- `engine/src/core/agent_spec.rs` — serialization and validation source.
- `engine/src/core/graph.rs` — graph and condition contract.
- [Executable complete example](../examples/support-summary.yaml)
- [System lifecycle](SYSTEM_LIFECYCLE.md)
- [Usage guide](../USAGE.md)
- [Built-in tools](backend/BUILTIN_TOOLS.md)
- [Live agents](backend/LIVE_AGENTS.md)
- [Compatibility](COMPATIBILITY.md)
