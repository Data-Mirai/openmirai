# PRD-004 Implementation Log

## Branch: prd/PRD-004

## Phase 1: Foundation (DONE)
- InputType enum (text, number, boolean, json, file) with matches() method
- InputFieldSpec, OutputFieldSpec structs with serde
- AgentSpec: added Optional inputs/outputs fields
- SharedState.get_field: nested traversal (trigger.payload.question)
- YAML-only: removed from_json/to_json, simplified from_file/to_file
- Trigger output rename: user_input→payload (with backward compat alias)
- Fixed tool count assertions (pre-existing: 11 data tools, 48 total)

### Files modified:
- engine/src/core/agent_spec.rs — new types, modified AgentSpec, removed JSON methods
- engine/src/core/state.rs — nested traversal in get_field
- engine/src/tools/builtin/trigger.rs — output key rename, config key rename
- engine/src/server/app.rs — injection key mock_payload→payload
- cli/src/main.rs — injection key mock_payload→payload
- engine/src/db/repositories.rs — added inputs/outputs to test constructors
- engine/src/intelligence/context_compiler.rs — added inputs/outputs to test constructors
- engine/src/runtime/agent_runtime.rs — added inputs/outputs to test constructors
- engine/src/tools/builtin/mod.rs — fixed tool count assertion
- engine/src/tools/builtin/data.rs — fixed tool count assertion

## Phase 2: Capa 1 — Input Validation (DONE)
- validate_agent_inputs() function in agent_spec.rs
- CLI --input validates against spec.inputs before injection
- HTTP POST /api/agents/{id}/execute validates trigger_data (422 on failure)
- Defaults applied for optional fields

## Phase 3: Capa 2 — Runner Hardening (DONE)
- validate_node_inputs() in tools/base.rs checks ToolSpec.inputs
- FieldType.matches() for runtime type checking
- catch_unwind in RegistryExecutor wraps tool.execute() — panics become ToolError
- Node validation runs before every tool.execute() call

### Files modified:
- engine/src/tools/base.rs — validate_node_inputs(), FieldType.matches()
- engine/src/tools/registry.rs — catch_unwind + validation gate in RegistryExecutor

## Phase 4: Interfaces (DONE)
- `mirai describe <agent.yaml>` — shows inputs/outputs contract
- `GET /api/agents/{id}/schema` — returns contract as JSON
- Both work with specs that have/don't have inputs/outputs

### Files modified:
- cli/src/main.rs — cmd_describe()
- engine/src/server/app.rs — get_agent_schema()

## Test Results
- 655 tests passed, 0 failed
- Full regression clean
