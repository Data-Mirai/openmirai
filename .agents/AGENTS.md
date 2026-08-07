# OpenMirai Developer Guidelines & Rules

This project is OpenMirai, a Rust-native agentic execution engine that runs YAML-defined DAG workflows. This rules document serves as the guide for any AI coding agent modifying, refactoring, or extending the repository.

---

## 1. System Architecture Overview

OpenMirai is built using a clean **Hexagonal Architecture (Ports and Adapters)**, separating the core execution logic from external system services and specific LLM providers.

```
                  ┌──────────────────────────────────────────────┐
                  │                 CLI / Server                 │
                  └──────────────────────┬───────────────────────┘
                                         │
                                         ▼
                  ┌──────────────────────────────────────────────┐
                  │          openmirai-engine (Core)             │
                  │  ┌───────────────┐        ┌───────────────┐  │
                  │  │  GraphRunner  ├───────►│  ToolRegistry │  │
                  │  └───────┬───────┘        └───────┬───────┘  │
                  │          │                        │          │
                  │          ▼                        ▼          │
                  │  ┌───────────────┐        ┌───────────────┐  │
                  │  │  Ports (Traits│        │  Built-in     │  │
                  │  │  context.rs)  │        │  Tools        │  │
                  │  └───────┬───────┘        └───────────────┘  │
                  └──────────┼───────────────────────────────────┘
                             │
                             ▼
                  ┌──────────────────────────────────────────────┐
                  │            Concrete Adapters                 │
                  │  ┌───────────────┐        ┌───────────────┐  │
                  │  │ sqlite_db.rs  │        │ ollama_llm.rs │  │
                  │  └───────────────┘        └───────────────┘  │
                  └──────────────────────────────────────────────┘
```

### Core Components
1. **`core/agent_spec.rs`**: Defines the declarative syntax of agents in YAML. Houses type validations, configuration parsing, retry specifications, and memory profiles.
2. **`core/graph.rs`**: Defines structural representation of a DAG (`GraphDef`, `NodeDef`, `EdgeDef`), checks structural validity (no duplicate nodes/edges, self-loop detection, valid references), and provides entry/exit node traversals.
3. **`core/runner/graph_runner.rs`**: The main execution engine. Traverses the graph, intercepts executions via hooks, interacts with the memory backend, resolves input templates, checks iterations, and manages checkpoints.
4. **`core/context.rs`**: Defines resource ports (`ExecutionContext`, `DBResource`, `LLMResource`, `StorageResource`, `VectorResource`) for hexagonal isolation.
5. **`adapters/`**: Houses production-ready adapters implementing the traits in `context.rs` (e.g. `LocalStorageResource`, `SqliteDBResource`, `AdapterBridgeLLMResource`).
6. **`llm/`**: The provider adapter layer (Claude, OpenAI, Gemini, Ollama, Groq, NVIDIA NIM, OpenRouter) implementing the inner `LLMAdapter` trait.
7. **`tools/`**: Declares and maintains tools, schema validation contracts, categories, and executes specific built-in operations.

---

## 2. Design Patterns Applied

All modifications must adhere to the design patterns established in the engine:

* **Ports and Adapters (Hexagonal)**: The runner and tools must only interact with interfaces defined in `core/context.rs`. Never instantiate raw databases or make direct HTTP calls inside tools.
* **Strategy Pattern (LLM and Tools)**: Tools are decoupled from executors and registered in a `ToolRegistry` mapped by a string key (e.g. `logic/condition`, `ai/llm_call`). The runner uses `ToolExecutor` to find and delegate executions dynamically. Similarly, `LLMAdapter` uses a strategy pattern to map model calls to provider-specific endpoints.
* **Observer Pattern (SSE and Events)**: Execution progression is broadcasted via an `EventEmitter` (based on Tokio broadcast channels) and a streaming channel `StreamEvent` for Server-Sent Events.
* **Memento / State Pattern (Checkpoints)**: Execution state is serialized to a `Checkpoint` struct after each successful node block run, enabling full pause/resume capabilities.
* **Pipeline / Chain of Responsibility (Intelligence/Compilers & Scanners)**: Compilation of execution contexts, scanning prompt injections via `security.rs` and formatting outputs are handled as multi-phase pipeline steps.

---

## 3. Developer Workflow Guidelines

### A. Implementing a New Tool
1. Create or open the relevant file in `engine/src/tools/builtin/` (e.g., `logic.rs`, `ai.rs`, or a submodule in `data/`).
2. Declare the input and output contracts using `ToolField` and `FieldType` enum via the category macros (e.g. `logic_tool!`, `ai_tool!`, `data_tool!`).
3. Implement the `Tool` trait, defining the async `execute` function.
4. Register the new factory in the corresponding register function inside `register_all_builtin_tools` (found in `engine/src/tools/builtin/mod.rs`).
5. Write unit tests at the bottom of the file using `#[tokio::test]`. Ensure tests use a mock execution context.

### B. Developing a New LLM Provider
1. Add the adapter implementation inside `engine/src/llm/` (e.g., `engine/src/llm/my_provider.rs`).
2. Implement the `LLMAdapter` trait from `engine/src/llm/adapter.rs`.
3. Normalize responses to the common `NormalizedResponse` type.
4. Integrate the new adapter inside `cli/src/adapter_factory.rs` and add the parameter mappings to CLI and server startup systems.

---

## 4. Coding Standards & Constraints

* **Preserve Unrelated Comments**: When making updates, retain all existing docstrings, logic comments, and architecture annotations.
* **Error Handling**: Use `thiserror` for library-level custom errors and map intermediate errors explicitly. Never use `.unwrap()` or `.expect()` in non-test production code.
* **Typing & Validation**: Validate all inputs at the boundary of tools using `validate_node_inputs`.
* **Zero External Dependencies in Tests**: All unit/integration tests (`cargo test`) must run locally and quickly, with no dependency on external services or network endpoints. Mock APIs and DB resources using in-memory or mock equivalents.
* **No `cd` Commands**: If compiling or running script/cargo checks, always pass the proper directory path via `Cwd` or execute relative commands.
