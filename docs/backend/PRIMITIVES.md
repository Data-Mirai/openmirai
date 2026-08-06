<!--
BLUEPRINT SEED — PRIMITIVES.md
Responsable: → blueprint/agents/13-CURATOR.md

Estructura esperada:
1. Compuestos nivel medio (services, hooks compuestos, adapters, middlewares)
2. Átomos nivel micro (utils, helpers, formatters, validators, constantes)

Cada entrada: nombre, estado, responsabilidad, ubicación, API pública, dependencias, usos[], creado-en

Reglas:
- Todo primitive activo debe listar Usos[] (permite análisis de impacto)
- Cuando pasa a obsoleto, apuntar a su reemplazo
- Describir API y responsabilidad, NO el código
- Side-effects siempre explícitos
- Si un pattern se repite en 2+ módulos y no está aquí → Curator lo reporta
- Si cambia la API de un primitive activo, Curator reporta usos[] afectados ANTES del cambio
-->

# PRIMITIVES.md

Core engine primitives and reusable patterns for the OpenMirai agentic graph execution engine. These are the building blocks contributors use to extend tools, add LLM adapters, and integrate new resources.

---

## 1. Compuestos (Nivel Medio)

### Tool + ToolFactory Trait Pattern {#prim-tool-factory}

**Estado:** activo

**Responsabilidad:** Define the abstract contract for executable nodes in a graph and their factory constructors.

**Ubicación:** `engine/src/tools/registry.rs`

**API pública:**

```rust
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}

pub trait ToolFactory: Send + Sync {
    fn create(&self) -> Arc<dyn Tool>;
    fn spec(&self) -> &ToolSpec;
}
```

**Responsabilidad detallada:**
- `Tool::execute` is the core contract: accepts resolved inputs (from data flow), node-level config, and execution context. Returns output fields as `HashMap<String, Value>` or a `ToolError`. All tools must be `Send + Sync` for async execution in the runner.
- `ToolFactory` is a factory pattern: owns the `ToolSpec` (metadata, inputs/outputs/config schema) and creates fresh `Tool` instances on demand. The runner looks up a factory by `node.tool_type`, calls `create()` to get a tool, and invokes `execute()`.

**Side-effects:** None intrinsic to the trait; side-effects are tool-specific (file I/O, LLM calls, DB queries, etc.). All side-effects must be observable in the returned `HashMap<String, Value>` or documented in ToolSpec.

**Dependencias:**
- `ToolSpec` (base.rs) — metadata schema
- `ExecutionContext` (core/context.rs) — access to LLM, DB, storage, auth
- `ToolError` (runner/types.rs) — error reporting

**Usos:**
- All 52 builtin tools (logic, ai, filesystem, git, system, state, output, data, etc.)
- Custom tools in user YAML graphs (via registry lookup)
- RegistryExecutor bridges registry to GraphRunner

**Creado en:** MVP (v0.1), refined with panic-catching (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### ToolSpec — Tool Metadata Schema {#prim-toolspec}

**Estado:** activo

**Responsabilidad:** Describes the static contract of a tool: name, inputs, outputs, config fields, and version.

**Ubicación:** `engine/src/tools/base.rs`

**API pública:**

```rust
pub struct ToolSpec {
    pub tool_type: String,          // e.g., "ai/llm_call", "filesystem/read_file"
    pub name: String,                // Human-readable name
    pub description: String,
    pub version: String,              // Semver, e.g., "1.0.0"
    pub category: String,             // e.g., "ai", "logic", "filesystem"
    pub inputs: Vec<ToolField>,       // Declared input schema
    pub outputs: Vec<ToolField>,      // Declared output schema
    pub config_fields: Vec<ToolField>, // Node-level configuration schema
}

pub struct ToolField {
    pub name: String,
    pub field_type: FieldType,  // String, Number, Boolean, Array, Object, Integer, File
    pub required: bool,
    pub description: Option<String>,
    pub default: Option<Value>,
}

pub enum FieldType {
    String, Number, Boolean, Array, Object, Integer, File
}
```

**Responsabilidad detallada:**
- Serializable metadata used by editors, API documentation, and the runner.
- `validate_node_inputs()` checks that resolved inputs match the declared schema: required fields present, types match, defaults applied for optional fields.
- `FieldType::File` is a special type representing FileRef (PRD-010) — a file reference object with `_type: "file_ref"`, `path`, `mime_type`, `size_bytes`.

**Side-effects:** None; purely metadata.

**Dependencias:**
- serde_json — JSON serialization

**Usos:**
- Editor (via /api/v1/tools) displays specs to compose graphs
- RegistryExecutor validates inputs before tool execution (PRD-004 Capa 2)
- Test harnesses verify schema correctness
- Backward-compatibility detection (schema versioning)

**Creado en:** MVP (v0.1), input validation (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### ToolRegistry + RegistryExecutor {#prim-registry}

**Estado:** activo

**Responsabilidad:** Central registry mapping tool_type strings to factories; RegistryExecutor bridges the registry to the graph runner.

**Ubicación:** `engine/src/tools/registry.rs`

**API pública:**

```rust
pub struct ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool_type: &str, factory: Box<dyn ToolFactory>);
    pub fn register_alias(&mut self, alias: &str, canonical: &str);
    pub fn get(&self, tool_type: &str) -> Option<&dyn ToolFactory>;
    pub fn list_tools(&self) -> Vec<&ToolSpec>;
}

pub struct RegistryExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self;
}

#[async_trait]
impl ToolExecutor for RegistryExecutor {
    async fn execute(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}
```

**Responsabilidad detallada:**
- `ToolRegistry` is a lookup table: `tool_type` (e.g., `"ai/llm_call"`) → `ToolFactory`. Supports aliases for backward compatibility (e.g., legacy `"fs/read_file"` → canonical `"filesystem/read_file"`).
- `RegistryExecutor` implements `ToolExecutor` for the runner: looks up the factory by node.tool_type, validates inputs against ToolSpec, creates a fresh Tool, and executes it. Catches tool panics (PRD-004) and converts them to ToolError.

**Side-effects:** 
- Reads from the registry (no mutations during execution).
- Delegates all execution side-effects to the Tool implementation.
- Catches panics and prevents process crashes.

**Dependencias:**
- ToolRegistry, ToolFactory, Tool, ToolSpec
- ExecutionContext
- ToolError, ToolExecutor trait

**Usos:**
- GraphRunner initializes RegistryExecutor in AppState
- All builtin tools registered at startup (engine/src/tools/builtin/mod.rs)
- Custom tools registered before `run()`
- Editor queries registry via /api/v1/tools endpoint

**Creado en:** MVP (v0.1), panic-catching (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### LLMAdapter — Provider Abstraction Layer {#prim-llmadapter}

**Estado:** activo

**Responsabilidad:** Abstract interface for LLM provider HTTP integration. Normalizes requests and responses across Claude, Gemini, Ollama, OpenAI-compatible, Groq, NVIDIA NIM, OpenRouter.

**Ubicación:** `engine/src/llm/adapter.rs`

**API pública:**

```rust
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    fn provider_name(&self) -> &str;
    
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: Option<&str>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError>;
    
    async fn call_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError>;
    
    async fn stream_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: u32,
        on_token: Option<&OnTokenFn>,
    ) -> Result<NormalizedResponse, LLMError>;
    
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError>;
}

pub struct NormalizedResponse {
    pub response: String,
    pub tokens_used: TokenUsage,
    pub model: String,
    pub provider: String,
    pub tool_calls: Vec<ToolCall>,
}

pub struct Message {
    pub role: String,           // "system", "user", "assistant", "tool"
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    pub tool_call_id: Option<String>,
    pub media: Option<Vec<MediaContent>>, // PRD-009: multimodal
}
```

**Responsabilidad detallada:**
- **Infrastructure layer**: handles provider-specific HTTP details, authentication, request/response translation.
- **Normalization**: every provider adapter returns `NormalizedResponse` — the engine never sees raw provider formats.
- **Agentic loop**: `call_with_messages()` accepts a full conversation and tool definitions; returns `tool_calls` when the LLM decides to invoke a tool.
- **Streaming**: `stream_with_messages()` emits tokens via callback; default implementation falls back to `call_with_messages()`.
- **Tool calling**: OpenAI-style tool calling unified across all providers (Claude, Gemini, Ollama, etc.).
- **Multimodal (PRD-009)**: Message.media carries base64-encoded images/audio/video for providers that support them.

**Side-effects:** 
- HTTP requests to provider APIs.
- May read files (via media attachment).

**Dependencias:**
- async_trait
- serde_json
- tokio (async)
- Provider-specific SDKs (anthropic, google-generativeai, ollama client, etc.)

**Usos:**
- Implementations: ClaudeAdapter, GeminiAdapter, OllamaAdapter, OpenAICompatAdapter, GroqAdapter, NVIDIAAdapter, OpenRouterAdapter
- AdapterBridgeLLMResource bridges LLMAdapter → LLMResource (domain layer)
- agentic loop in graph runner uses LLMResource, which delegates to LLMAdapter
- ai/llm_call tool uses LLMResource context

**Creado en:** MVP (v0.1), multimodal support (PRD-009)

**Reemplaza / Reemplazado por:** —

---

### LLMResource — Domain-Level LLM Interface {#prim-llmresource}

**Estado:** activo

**Responsabilidad:** Simplified "port" interface for tools and the runner. Exposes only what the execution engine needs: prompt completion and embeddings. Bridges the gap between infrastructure (LLMAdapter) and domain (tools + runner).

**Ubicación:** `engine/src/core/context.rs`

**API pública:**

```rust
#[async_trait]
pub trait LLMResource: Send + Sync {
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: &[serde_json::Value],
        temperature: f64,
        max_tokens: u32,
    ) -> Result<LLMResponse, ResourceError>;
    
    async fn embed(&self, text: &str, model: &str) -> Result<Vec<f64>, ResourceError>;
    
    fn provider_name(&self) -> &str;
}

pub struct LLMResponse {
    pub response: String,
    pub tokens_used: TokenUsage,
    pub model: String,
    pub provider: String,
}
```

**Responsabilidad detallada:**
- **Two-layer design**: `LLMAdapter` handles "how to talk to a provider" (HTTP, auth, message format). `LLMResource` handles "what a tool needs from an LLM" (call a model, get embeddings). Separation lets tools never see provider-specific details, and adding a provider doesn't touch the execution engine.
- **context array**: carries prompt enhancements, system prompts, media attachments. Passed through to the adapter bridge, which interprets entries.
- **Call method**: simple prompt-in, response-out. No message arrays or tool calling at this layer.
- **Embeddings**: vector generation for RAG, semantic search, clustering.

**Side-effects:** None intrinsic; delegated to the underlying LLMAdapter.

**Dependencias:**
- LLMAdapter (via adapter bridge)
- ExecutionContext provides LLMResource to tools

**Usos:**
- Injected into ExecutionContext; every tool accesses via `context.llm()`
- ai/llm_call tool uses it directly
- Text embedding tools (data module) use .embed()
- Test tools use StubLLM (in-memory test double)

**Creado en:** MVP (v0.1), two-layer architecture formalized (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### Two-Layer LLM Architecture {#prim-llm-architecture}

**Estado:** activo

**Responsabilidad:** Design pattern separating infrastructure (LLMAdapter) from domain (LLMResource), enabled by AdapterBridgeLLMResource.

**Ubicación:** `engine/src/llm/adapter.rs`, `engine/src/core/context.rs`, `engine/src/adapters/adapter_bridge.rs`

**Conceptual diagram:**

```
┌─────────────────────────────────────────────────────────┐
│  DOMAIN LAYER: Tools, GraphRunner                        │
│  └─ Use LLMResource (simple prompt-in, response-out)    │
│     └─ calls context.llm()                              │
└──────────────┬──────────────────────────────────────────┘
               │
        ┌──────▼────────────────────┐
        │ AdapterBridgeLLMResource  │
        │ (adapters/adapter_bridge) │
        │ Impl: LLMResource         │
        └──────┬────────────────────┘
               │
┌──────────────▼──────────────────────────────────────────┐
│  INFRASTRUCTURE LAYER: Provider Adapters                │
│  ├─ LLMAdapter trait (provider-specific HTTP)           │
│  ├─ ClaudeAdapter, GeminiAdapter, OllamaAdapter, etc.   │
│  └─ Handle: messages, tool calling, streaming, auth    │
└─────────────────────────────────────────────────────────┘
```

**Responsabilidad detallada:**
- **LLMAdapter** (infrastructure): knows how to call Anthropic API, Google Generative AI API, Ollama local endpoint, etc. Receives `Message[]`, tool definitions, handles streaming callbacks, normalizes responses.
- **AdapterBridgeLLMResource** (bridge): wraps an LLMAdapter. Converts simple LLMResource.call() (prompt string) into LLMAdapter.call_with_messages() (message array + tools). Extracts media from context, injects system prompts.
- **LLMResource** (domain): tools never see provider APIs. They call `context.llm().call(model, prompt, context, temp, max_tokens)` — uniform interface regardless of provider.

**Side-effects:** None; the pattern is transparent.

**Dependencias:**
- LLMAdapter, LLMResource, AdapterBridgeLLMResource
- ExecutionContext injection

**Usos:**
- Every LLMAdapter implementation (7 providers)
- AdapterBridgeLLMResource bridges each adapter into ExecutionContext
- Tools call context.llm() — never touch adapters
- Enables adding new providers without modifying tools or runner
- Supports feature flags per provider (e.g., streaming, tool calling)

**Creado en:** MVP (v0.1), formalized in PRD-004

**Reemplaza / Reemplazado por:** —

---

### ExecutionContext — Tool Runtime Environment {#prim-executioncontext}

**Estado:** activo

**Responsabilidad:** Trait providing tools access to runtime resources: LLM, DB, storage, vector search, auth, session metadata, scratch directory.

**Ubicación:** `engine/src/core/context.rs`

**API pública:**

```rust
pub trait ExecutionContext: Send + Sync {
    fn db(&self) -> Option<&dyn DBResource>;
    fn llm(&self) -> &dyn LLMResource;
    fn storage(&self) -> Option<&dyn StorageResource>;
    fn vector(&self) -> Option<&dyn VectorResource>;
    fn auth(&self) -> &AuthContext;
    fn session_id(&self) -> &str;
    fn node_id(&self) -> Option<&str>;
    fn system_prompt(&self) -> Option<&str>;
    fn scratch_dir(&self) -> Option<&str>;
}

pub trait DBResource: Send + Sync {
    async fn execute(&self, query: &str, params: &[Value]) -> Result<Value, ResourceError>;
    async fn fetch_one(&self, query: &str, params: &[Value]) -> Result<Option<Value>, ResourceError>;
    async fn fetch_all(&self, query: &str, params: &[Value]) -> Result<Vec<Value>, ResourceError>;
}

pub trait StorageResource: Send + Sync {
    async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError>;
    async fn put(&self, path: &str, data: &[u8]) -> Result<(), ResourceError>;
    async fn delete(&self, path: &str) -> Result<(), ResourceError>;
}

pub trait VectorResource: Send + Sync {
    async fn upsert(&self, id: &str, text: &str, metadata: Value) -> Result<(), ResourceError>;
    async fn search(&self, query: &str, top_k: usize) -> Result<Vec<VectorSearchResult>, ResourceError>;
    async fn delete(&self, id: &str) -> Result<(), ResourceError>;
}

pub struct AuthContext {
    pub user_id: String,
    pub role: Role,              // Owner, Admin, Editor, Viewer
    pub universe_id: Option<String>,
    pub environment_id: Option<String>,
}
```

**Responsabilidad detallada:**
- **Dependency injection pattern**: tools receive ExecutionContext instead of managing connections.
- **Resource portability**: DBResource/StorageResource/VectorResource are traits — implementations vary (SQLite, S3, Redis, etc.) without changing tools.
- **Auth info**: every execution has a user, role, and optional multi-tenant context.
- **Session metadata**: session_id for logging/auditing, node_id for debugging, system_prompt for LLM injection.
- **Scratch directory (PRD-010)**: temporary space for tools to write files (e.g., processed images, intermediate results). Cleaned up after execution.
- **Optional resources**: not all contexts have DB/storage/vector — tools check with .db().is_some(), gracefully degrade.

**Side-effects:** None intrinsic; delegated to resources.

**Dependencias:**
- LLMResource, DBResource, StorageResource, VectorResource
- AuthContext, Role

**Usos:**
- Passed to Tool::execute() by RegistryExecutor
- Passed to all HookHandler callbacks
- Passed to GraphRunner.run()
- Injected in AppState/middleware in CLI/HTTP server
- Test doubles: InMemoryContext (stub resources for unit tests)

**Creado en:** MVP (v0.1), extended with scratch_dir (PRD-010)

**Reemplaza / Reemplazado por:** —

---

### HookHandler — Seven Execution Interception Points {#prim-hookhandler}

**Estado:** activo

**Responsabilidad:** Extensibility interface for monitoring, modifying, and controlling graph execution. Seven async hooks with default implementations (no-op).

**Ubicación:** `engine/src/core/runner/traits.rs`

**API pública:**

```rust
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn on_graph_start(&self, graph: &GraphDef, ctx: &dyn ExecutionContext) -> HookResult;
    async fn on_graph_end(&self, graph: &GraphDef, state: &SharedState, ctx: &dyn ExecutionContext) -> HookResult;
    async fn pre_block_exec(&self, node: &NodeDef, inputs: &mut HashMap<String, Value>, ctx: &dyn ExecutionContext) -> HookResult;
    async fn post_block_exec(&self, node: &NodeDef, output: &mut HashMap<String, Value>, ctx: &dyn ExecutionContext) -> HookResult;
    async fn pre_llm_call(&self, node: &NodeDef, ctx: &dyn ExecutionContext) -> HookResult;
    async fn post_llm_call(&self, node: &NodeDef, response: &mut Value, ctx: &dyn ExecutionContext) -> HookResult;
    async fn on_error(&self, node: &NodeDef, error: &ToolError, ctx: &dyn ExecutionContext) -> HookResult;
}

pub enum HookResult {
    Continue,                          // Proceed normally.
    Skip,                              // Skip this block (pre_block_exec only).
    Abort(String),                     // Stop execution with reason.
    Retry,                             // Retry the block (on_error only).
    ModifiedInputs(HashMap<String, Value>), // Replace inputs (pre_block_exec only).
}
```

**Responsabilidad detallada:**

| Hook | Timing | Use Cases | Supported Outcomes |
|------|--------|-----------|-------------------|
| `on_graph_start` | Before any node executes | Initialize state, log session start | Continue, Abort |
| `on_graph_end` | After graph finishes (success or error) | Final cleanup, metrics export, webhook | Continue |
| `pre_block_exec` | Before node executes | Input validation, injection, filtering | Continue, Skip, Abort, ModifiedInputs |
| `post_block_exec` | After node succeeds | Output transformation, side-effect tracking | Continue, Abort |
| `pre_llm_call` | Before ai/llm_call starts | Prompt injection detection, logging | Continue, Abort |
| `post_llm_call` | After ai/llm_call returns | Response filtering, schema validation retries | Continue, Abort |
| `on_error` | After tool fails | Logging, alerting, automatic retry logic | Continue, Retry, Abort |

**Side-effects:** Hook implementations control side-effects (logging, webhooks, DB writes). Defaults are no-op.

**Dependencias:**
- GraphDef, NodeDef, ExecutionContext, SharedState, ToolError
- async_trait

**Usos:**
- Custom instrumentation (metrics, tracing, observability)
- Security hooks (prompt injection scanning, output validation)
- Retry logic (automatic retries on transient errors)
- State management (checkpoints, resume points)
- Testing (verification of node order, input/output validation)
- API server attaches hooks for SSE streaming, webhook integration

**Creado en:** MVP (v0.1), expanded to 7 hooks (PRD-004), LLM-specific hooks (PRD-009)

**Reemplaza / Reemplazado por:** —

---

### RetryPolicy + BackoffStrategy {#prim-retry}

**Estado:** activo

**Responsabilidad:** Configurable retry logic with three backoff strategies: None, Linear, Exponential.

**Ubicación:** `engine/src/core/runner/types.rs`

**API pública:**

```rust
#[derive(Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,              // 0 = no retries; default 3
    pub backoff: BackoffStrategy,      // default: Exponential
    pub initial_delay_secs: f64,       // default: 1.0
    pub on_failure: FailureMode,       // default: Stop
}

#[derive(Serialize, Deserialize)]
pub enum BackoffStrategy {
    None,          // No delay between retries
    Linear,        // delay = initial_delay * attempt
    Exponential,   // delay = initial_delay * 2^attempt
}

#[derive(Serialize, Deserialize)]
pub enum FailureMode {
    Stop,         // Halt execution (default)
    Skip,         // Skip this node, continue to next
    RouteToError, // Send to error handler node (if defined)
}
```

**Responsabilidad detallada:**
- **Default policy**: 3 retries, exponential backoff starting at 1s. Aligned with industry standard for transient failures (network, rate limits, temporary API outages).
- **Per-node override**: node.config can override via `"retry_policy"` field.
- **Backoff calculation**:
  - None: immediate retry
  - Linear: 1s, 2s, 3s, 4s, ...
  - Exponential: 1s, 2s, 4s, 8s, ... (grows unbounded until max_retries is exhausted)
- **Failure modes**: Stop (traditional), Skip (continue graph despite error), RouteToError (flow to error handler node if one exists).
- **Applied by GraphRunner** in the main execution loop.

**Side-effects:** None; controls timing of retries.

**Dependencias:**
- core/runner/types.rs
- Serialized in GraphDef/NodeDef

**Usos:**
- Per-node config in YAML graphs
- Global default in GraphRunner.with_default_retry_policy()
- LLM tools often need retries (rate limits, timeout)
- Filesystem tools retry on transient ENOENT
- Network tools (HTTP, git) use exponential backoff for resilience

**Creado en:** MVP (v0.1), formalized (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### CheckpointCallback + Checkpoint {#prim-checkpoint}

**Estado:** activo

**Responsabilidad:** Interface for persisting execution state at safe points (after each successful node). Enables resume/pause functionality.

**Ubicación:** `engine/src/core/runner/traits.rs`, `engine/src/core/runner/types.rs`

**API pública:**

```rust
#[async_trait]
pub trait CheckpointCallback: Send + Sync {
    async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError>;
}

pub struct Checkpoint {
    pub session_id: String,
    pub step: u32,
    pub node_id: String,
    pub state_snapshot: HashMap<String, HashMap<String, Value>>,
    pub cursor_node_id: Option<String>,
    pub timestamp: f64,
}
```

**Responsabilidad detallada:**
- **Safe point**: checkpoint is saved after a node succeeds and its outputs are recorded. Not during execution (to avoid partial state).
- **state_snapshot**: a snapshot of SharedState (all node outputs so far), enabling resume from this point.
- **Return value**: the implementation returns a checkpoint ID (for audit logs, webhook payloads, etc.).
- **Resume**: GraphRunner.resume() takes a saved state + resume_node_id, skips earlier nodes, continues from there.
- **Pause**: request_pause() sets a flag; runner checks it after each block and halts if set (graceful pause, not immediate).

**Side-effects:** 
- Implementations write to storage (DB, Redis, local file). Must be durable before returning.

**Dependencias:**
- SharedState, Checkpoint
- Typically backed by DBResource or StorageResource

**Usos:**
- HTTP server saves checkpoints for long-running graphs (pause/resume UI)
- Resume-from-interrupt workflow (human_input tool waits, then resume)
- Audit log: who paused, when, from which node
- Disaster recovery: restore state if runner crashes
- Test harnesses verify checkpoint correctness

**Creado en:** MVP (v0.1), extended with pause/resume (PRD-005)

**Reemplaza / Reemplazado por:** —

---

### FileRef + data_map — File Passing and Data Routing {#prim-fileref-datamap}

**Estado:** activo

**Responsabilidad:** Standardized file references (FileRef) as first-class graph data, and data_map for flexible input routing.

**Ubicación:** `engine/src/llm/media.rs`, `engine/src/core/graph.rs`

**API pública:**

```rust
// FileRef as a JSON object:
{
    "_type": "file_ref",
    "path": "/absolute/path/to/file.png",
    "mime_type": "image/png",
    "size_bytes": 1024
}

// Helper functions:
pub fn create_file_ref(file_path: &str, base_dir: Option<&str>) -> Option<Value>;
pub fn is_file_ref(value: &Value) -> bool;
pub fn resolve_file_input(value: &Value) -> String; // Extracts path

// EdgeDef with data_map:
pub struct EdgeDef {
    pub source: String,
    pub target: String,
    pub condition: Option<EdgeCondition>,
    pub data_map: Option<HashMap<String, String>>, // Maps source output → target input
}

// If data_map is None and edge is unconditional:
// entire source output passed through as inputs (backward compat).
```

**Responsabilidad detallada:**

- **FileRef (PRD-010)**: When a tool produces a file (e.g., ai/llm_call generates an image), it returns a FileRef object instead of raw bytes. Subsequent tools can pass FileRef as an input; the runtime resolves the path before execution.
  - Avoids inline-encoding large blobs in JSON.
  - Enables tools to receive file metadata (MIME type, size) for validation.
  - Media files (images, audio, video) attached to LLM prompts via FileRef.

- **data_map (FEAT-034 / API-03)**: Explicitly maps output fields from a source node to input fields of a target node.
  - Example: `{"response": "user_prompt", "model": "model_used"}` means source.response → target.user_prompt.
  - Allows selective routing (ignore some outputs, transform field names).
  - When data_map is None and edge is unconditional, entire source output passed (backward compatible).
  - Reduces coupling between nodes; target node doesn't care about source's full output schema.

**Side-effects:** 
- create_file_ref reads file metadata (size, MIME from extension).
- resolve_file_input may read file content (when needed by tool).

**Dependencias:**
- serde_json, std::path
- FieldType::File (base.rs)

**Usos:**
- Image generation tools output FileRef for images
- Filesystem tools (read_file) output FileRef
- ai/llm_call accepts FileRef in media_path config
- data_map in edges for flexible routing
- Scratch directory (PRD-010) stores temp files, referenced via FileRef

**Creado en:** FileRef (PRD-010), data_map (FEAT-034)

**Reemplaza / Reemplazado por:** —

---

### GraphRunner — Main Execution Engine {#prim-graphrunner}

**Estado:** activo

**Responsabilidad:** Sequential cursor that traverses a validated DAG, executing nodes one at a time with conditional branching, retry with backoff, hooks, checkpoints, human-input interrupts, pause/resume, and event streaming.

**Ubicación:** `engine/src/core/runner/graph_runner.rs`

**API pública:**

```rust
pub struct GraphRunner {
    pub fn new(executor: Box<dyn ToolExecutor>) -> Self;
    pub fn with_event_emitter(self, emitter: EventEmitter) -> Self;
    pub fn with_hook_handler(self, handler: Box<dyn HookHandler>) -> Self;
    pub fn with_checkpoint_callback(self, cb: Box<dyn CheckpointCallback>) -> Self;
    pub fn with_max_iterations(self, max: u32) -> Self;
    pub fn with_default_retry_policy(self, policy: RetryPolicy) -> Self;
    pub fn with_stream_tx(self, tx: mpsc::Sender<StreamEvent>) -> Self;
    pub fn request_pause(&self);
    pub fn pause_flag(&self) -> Arc<AtomicBool>;
    
    pub async fn run(&self, graph: &GraphDef, context: &dyn ExecutionContext) -> Result<ExecutionResult, RunnerError>;
    pub async fn run_with_state(&self, graph: &GraphDef, context: &dyn ExecutionContext, state: SharedState) -> Result<ExecutionResult, RunnerError>;
    pub async fn resume(&self, graph: &GraphDef, context: &dyn ExecutionContext, state: SharedState, resume_node_id: &str) -> Result<ExecutionResult, RunnerError>;
}

pub struct ExecutionResult {
    pub status: ExecutionStatus,       // Completed, Failed, Timeout, Interrupted
    pub state: SharedState,             // Final state (all node outputs)
    pub trace: Vec<TraceEntry>,         // Execution log (node, status, duration, error)
    pub transcript: Vec<TranscriptEntry>, // Human-readable log entries
    pub error: Option<String>,
    pub interrupt_node_id: Option<String>,
    pub interrupt_info: Option<InterruptInfo>,
}
```

**Responsabilidad detallada:**

**Algorithm (from docstring):**
1. Validate graph structure.
2. Find entry node (first node with no incoming edges).
3. Walk graph following edges, executing each node via ToolExecutor.
4. Support conditional branching, retry with backoff, hooks, checkpoints, human-input interrupts, pause/resume.
5. Return ExecutionResult with status, final state, trace, transcript.

**Builder pattern**: chain .with_*() methods to configure executor, hooks, checkpoints, retry policy, event emitters, stream TX.

**Execution flow**:
- on_graph_start hook (may abort).
- For each node:
  - Check pause flag, max iterations, human_input interrupts.
  - Resolve inputs from incoming edges + data_map.
  - Call pre_block_exec hook (may skip, modify inputs, abort).
  - Execute tool with retry logic (backoff per RetryPolicy).
  - Call post_block_exec hook (may abort).
  - Record in trace.
  - On error: call on_error hook (may retry, abort); apply failure_mode (Stop, Skip, RouteToError).
  - Advance cursor to next node(s) (fan-out on conditionals, sequential on unconditionals).
- on_graph_end hook.
- Return ExecutionResult.

**Side-effects:**
- Calls hooks (logging, instrumentation).
- Saves checkpoints.
- Emits events (graph start/end, node start/end, errors).
- Streams events via SSE if stream_tx is attached.

**Dependencias:**
- ToolExecutor, HookHandler, CheckpointCallback
- GraphDef, NodeDef, EdgeDef, EdgeCondition
- ExecutionContext, SharedState
- RetryPolicy, ToolError, ExecutionResult

**Usos:**
- Core of the execution engine
- API server (/api/v1/graphs/{id}/run) wraps it with context injection
- CLI (`mirai run`) creates runner and calls it
- Test harnesses verify graph execution
- HTTP streaming (SSE) attached to runner for real-time updates

**Creado en:** MVP (v0.1), extended with hooks (PRD-004), checkpoints (PRD-005), human_input (PRD-006), pause/resume

**Reemplaza / Reemplazado por:** —

---

## 2. Átomos (Nivel Micro)

### field() — ToolField Constructor {#prim-field}

**Estado:** activo

**Responsabilidad:** Single canonical constructor for ToolField, used by all tool modules to prevent typos and inconsistency.

**Ubicación:** `engine/src/tools/base.rs`

**API pública:**

```rust
pub fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField
```

**Responsabilidad detallada:**
- Builds a ToolField with the given name, type, required flag, and description.
- Converts empty description to `None` (avoid redundant fields in JSON).
- Default value always None (set explicitly if needed).
- Used by all tool macros and tool definitions to ensure consistency.

**Side-effects:** None.

**Dependencias:** FieldType, ToolField

**Usos:** Every tool macro (ai_tool!, logic_tool!, system_tool!, etc.) uses field() to define inputs/outputs.

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### FieldType::matches() — Type Validation {#prim-fieldtype-matches}

**Estado:** activo

**Responsabilidad:** Check if a serde_json::Value matches a declared FieldType.

**Ubicación:** `engine/src/tools/base.rs`

**API pública:**

```rust
impl FieldType {
    pub fn matches(&self, value: &Value) -> bool;
}
```

**Responsabilidad detallada:**
- String matches Value::String.
- Number matches any JSON number (int or float).
- Boolean matches Value::Bool.
- Array matches Value::Array.
- Object matches Value::Object.
- Integer matches i64 or u64 (not floats).
- File matches FileRef objects (calls crate::llm::media::is_file_ref).

**Side-effects:** None.

**Dependencias:** FieldType, serde_json::Value, media.rs

**Usos:** validate_node_inputs() uses matches() to check resolved inputs against ToolSpec.

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### validate_node_inputs() — Schema Validation {#prim-validate-inputs}

**Estado:** activo

**Responsabilidad:** Validate resolved inputs against a tool's ToolSpec, applying defaults and catching type mismatches.

**Ubicación:** `engine/src/tools/base.rs`

**API pública:**

```rust
pub fn validate_node_inputs(
    inputs: &HashMap<String, Value>,
    tool_spec: &ToolSpec,
    node_id: &str,
) -> Result<HashMap<String, Value>, Vec<String>>;
```

**Responsabilidad detallada:**
- For each ToolSpec.inputs field:
  - If present: check type matches via FieldType::matches().
  - If missing and required: error.
  - If missing and optional: apply default if available.
- Return enriched inputs (with defaults) or list of error messages.
- Error messages include node_id for debugging.

**Side-effects:** None.

**Dependencias:** FieldType, ToolSpec, ToolField

**Usos:** RegistryExecutor calls validate_node_inputs() before executing a tool (PRD-004 Capa 2).

**Creado en:** MVP (v0.1), formalized (PRD-004)

**Reemplaza / Reemplazado por:** —

---

### is_file_ref() — FileRef Detection {#prim-is-fileref}

**Estado:** activo

**Responsabilidad:** Check if a serde_json::Value is a valid FileRef object.

**Ubicación:** `engine/src/llm/media.rs`

**API pública:**

```rust
pub fn is_file_ref(value: &serde_json::Value) -> bool;
```

**Responsabilidad detallada:**
- Checks for `_type: "file_ref"` discriminator.
- Checks for `path` field (string).
- Returns true only if both are present and correct.

**Side-effects:** None.

**Dependencias:** serde_json

**Usos:**
- FieldType::File matching (base.rs)
- Data flow validation
- Tool input resolution

**Creado en:** PRD-010

**Reemplaza / Reemplazado por:** —

---

### resolve_file_input() — FileRef Path Extraction {#prim-resolve-file}

**Estado:** activo

**Responsabilidad:** Extract file path from a tool input that may be a string, FileRef, or other value.

**Ubicación:** `engine/src/llm/media.rs`

**API pública:**

```rust
pub fn resolve_file_input(value: &serde_json::Value) -> String;
```

**Responsabilidad detallada:**
- Value::String → return as-is (backward compat).
- FileRef object → extract and return path field.
- Null → return empty string.
- Other → stringify the value.

**Side-effects:** None.

**Dependencias:** serde_json

**Usos:** Tools receiving file inputs (e.g., filesystem/read_file, ai/llm_call with media_path).

**Creado en:** PRD-010

**Reemplaza / Reemplazado por:** —

---

### create_file_ref() — FileRef Constructor {#prim-create-fileref}

**Estado:** activo

**Responsabilidad:** Build a FileRef JSON value from a file path, resolving relative paths and detecting MIME type.

**Ubicación:** `engine/src/llm/media.rs`

**API pública:**

```rust
pub fn create_file_ref(file_path: &str, base_dir: Option<&str>) -> Option<serde_json::Value>;
```

**Responsabilidad detallada:**
- Resolve path (absolute or relative via base_dir).
- Check file exists.
- Detect MIME type from extension.
- Read file size.
- Return FileRef JSON: `{ "_type": "file_ref", "path": ..., "mime_type": ..., "size_bytes": ... }`.
- Return None if file not found.

**Side-effects:** Reads file metadata and attributes (no content read).

**Dependencias:** std::path, std::fs, mime detection

**Usos:** Tools producing file outputs (filesystem, image generation, etc.).

**Creado en:** PRD-010

**Reemplaza / Reemplazado por:** —

---

### read_media_file() — Media Validation & Base64 Encoding {#prim-read-media}

**Estado:** activo

**Responsabilidad:** Read a media file, validate (size, extension, provider support), and return base64-encoded MediaContent.

**Ubicación:** `engine/src/llm/media.rs`

**API pública:**

```rust
pub fn read_media_file(file_path: &str, provider_name: &str) -> Result<MediaContent, String>;

pub struct MediaContent {
    pub mime_type: String,
    pub data: String,           // Base64
    pub source_path: Option<String>,
}
```

**Validation checks:**
- File exists.
- File not empty.
- File size ≤ 20 MB.
- Extension recognized.
- MIME type supported by provider.

**Side-effects:** Reads file content (base64 encoding consumes memory).

**Dependencias:** std::fs, base64, mime detection

**Usos:** ai/llm_call tool (media_path config) uses read_media_file() to attach images/audio to prompts.

**Creado en:** PRD-009

**Reemplaza / Reemplazado por:** —

---

### TokenUsage — Token Accounting {#prim-tokenusage}

**Estado:** activo

**Responsabilidad:** Standardized structure for input/output token counts.

**Ubicación:** `engine/src/core/context.rs` (canonical), re-exported from `engine/src/llm/adapter.rs`

**API pública:**

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}
```

**Responsabilidad detallada:**
- Simple counter pair for billing, rate limiting, cost analysis.
- Returned by every LLMAdapter and LLMResponse.
- Aggregated in ExecutionResult and trace logs.

**Side-effects:** None.

**Dependencias:** serde

**Usos:**
- All LLM adapters populate this.
- Traced in execution results.
- Used for billing and cost estimation.
- Tested in adapter harnesses.

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### ComparisonOp — Edge Condition Operators {#prim-comparisonop}

**Estado:** activo

**Responsabilidad:** Enumeration of comparison operators for conditional edges.

**Ubicación:** `engine/src/core/graph.rs`

**API pública:**

```rust
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ComparisonOp {
    Eq, Neq, Gt, Lt, Gte, Lte, In, Contains,
}

// Deserialization accepts multiple forms:
// - Full: "equals", "not_equals", "greater_than", "less_than", "greater_or_equal", "less_or_equal", "in", "contains"
// - Short: "eq", "neq", "gt", "lt", "gte", "lte"
// - PascalCase: "Eq", "Neq", "Gt", "Lt", "Gte", "Lte"
```

**Responsabilidad detallada:**
- Evaluated in EdgeCondition by GraphRunner to decide which edge to follow.
- Flexible deserialization for YAML/JSON readability.
- Supports numeric (>, <), equality, containment, membership checks.

**Side-effects:** None.

**Dependencias:** serde

**Usos:** Conditional branching in graphs (edges with conditions).

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### TraceEntry + TraceStatus — Execution Logging {#prim-traceentry}

**Estado:** activo

**Responsabilidad:** Immutable log of each node execution (node ID, tool type, status, duration, retries, errors).

**Ubicación:** `engine/src/core/runner/types.rs`

**API pública:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraceStatus {
    Ok,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub node_id: String,
    pub tool_type: String,
    pub status: TraceStatus,
    pub duration_ms: u64,
    pub retries: u32,
    pub error: Option<String>,
}
```

**Responsabilidad detallada:**
- One entry per node execution (even if skipped or retried).
- duration_ms measures wall-clock time (useful for perf profiling).
- retries counts retry attempts (0 = first try, 1+ = retries).
- error populated only if status != Ok.
- Serialized in ExecutionResult.trace for audit, debugging, perf analysis.

**Side-effects:** None; purely metadata.

**Dependencias:** serde

**Usos:**
- ExecutionResult.trace contains Vec<TraceEntry>.
- Displayed in API responses, CLI output, logs.
- Analyzed for performance bottlenecks (slow nodes).
- Auditing (which nodes failed, how many retries).

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### TranscriptEntry — Human-Readable Logging {#prim-transcriptentry}

**Estado:** activo

**Responsabilidad:** Human-readable log entries generated during execution (e.g., "LLM call started", "Tool X failed").

**Ubicación:** `engine/src/core/runner/types.rs`

**API pública:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub entry_type: String,  // e.g., "node_start", "node_end", "error", "log"
    pub message: String,      // Human-readable text
    pub timestamp: f64,
    pub node_id: Option<String>,
    pub metadata: HashMap<String, Value>,
}
```

**Responsabilidad detallada:**
- Structured but readable logs (unlike trace, which is machine-centric).
- entry_type classifies the log (node_start, node_end, error, llm_call, etc.).
- message is human-friendly prose.
- metadata carries context (e.g., LLM model used, error code).
- Serialized in ExecutionResult.transcript.

**Side-effects:** None; purely metadata.

**Dependencias:** serde

**Usos:**
- Displayed in UI logs / SSE streams.
- Audit trails.
- Debugging (understanding what happened in plain English).
- Exported for external logging systems.

**Creado en:** MVP (v0.1), extended in PRD-005

**Reemplaza / Reemplazado por:** —

---

### SharedState — Mutable Graph Execution State {#prim-sharedstate}

**Estado:** activo

**Responsabilidad:** Thread-safe mutable state for graph execution, storing outputs from each executed node.

**Ubicación:** `engine/src/core/state.rs`

**API pública:**

```rust
pub struct SharedState {
    // Internal: HashMap<node_id, HashMap<output_field, Value>>
    pub fn new() -> Self;
    pub fn get(&self, node_id: &str) -> Option<Arc<HashMap<String, Value>>>;
    pub fn set(&self, node_id: &str, outputs: HashMap<String, Value>);
    pub fn clone_all(&self) -> HashMap<String, HashMap<String, Value>>;
}
```

**Responsabilidad detallada:**
- Shared state dictionary: node_id → its outputs (HashMap<String, Value>).
- Thread-safe (Arc<RwLock<...>>) for concurrent reads during data flow resolution.
- Each node writes its output to state after execution.
- Subsequent nodes read from state to resolve their inputs.
- Checkpoints snapshot the entire state for resume.
- Final state returned in ExecutionResult.

**Side-effects:** Mutable; shared across async tasks.

**Dependencias:** Arc, RwLock

**Usos:**
- GraphRunner uses it as the central state dict.
- Data flow resolution (edges read from state).
- Checkpointing and resume.
- Memory tool (state/memory) reads from state.
- Test harnesses verify state contents.

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

### HookResult Enum — Hook Control Flow {#prim-hookresult}

**Estado:** activo

**Responsabilidad:** Return value from hook methods to control execution flow.

**Ubicación:** `engine/src/core/runner/types.rs`

**API pública:**

```rust
pub enum HookResult {
    Continue,                          // Proceed normally
    Skip,                              // Skip this block (pre_block_exec only)
    Abort(String),                     // Stop execution with reason
    Retry,                             // Retry the block (on_error only)
    ModifiedInputs(HashMap<String, Value>), // Replace inputs (pre_block_exec only)
}
```

**Responsabilidad detallada:**
- Continue: default, no-op.
- Skip: used only in pre_block_exec; node is skipped, cursor advances.
- Abort: halt execution with error message. Returns from runner.
- Retry: used only in on_error; attempt to execute node again.
- ModifiedInputs: used only in pre_block_exec; replace inputs and proceed.

**Side-effects:** None; controls flow in GraphRunner.

**Dependencias:** HashMap, Value

**Usos:** Returned by all HookHandler methods.

**Creado en:** MVP (v0.1)

**Reemplaza / Reemplazado por:** —

---

## Summary Table

| Primitive | Type | Location | State | Use Case |
|-----------|------|----------|-------|----------|
| Tool + ToolFactory | Trait Pattern | tools/registry.rs | Active | All executable nodes |
| ToolSpec | Schema | tools/base.rs | Active | Node metadata & validation |
| ToolRegistry + RegistryExecutor | Registry | tools/registry.rs | Active | Node lookup & execution |
| LLMAdapter | Trait | llm/adapter.rs | Active | Provider HTTP integration |
| LLMResource | Trait | core/context.rs | Active | Domain-level LLM interface |
| Two-Layer LLM | Architecture | llm/ + core/ | Active | Separation of concerns |
| ExecutionContext | Trait | core/context.rs | Active | Tool runtime environment |
| HookHandler | Trait | core/runner/traits.rs | Active | Execution instrumentation |
| RetryPolicy | Config | core/runner/types.rs | Active | Resilience + backoff |
| CheckpointCallback | Trait | core/runner/traits.rs | Active | State persistence & resume |
| FileRef + data_map | Pattern | llm/media.rs + core/graph.rs | Active | File passing + data routing |
| GraphRunner | Engine | core/runner/graph_runner.rs | Active | Main execution loop |
| field() | Helper | tools/base.rs | Active | ToolField construction |
| FieldType::matches() | Validator | tools/base.rs | Active | Type checking |
| validate_node_inputs() | Validator | tools/base.rs | Active | Schema validation |
| is_file_ref() | Detector | llm/media.rs | Active | FileRef detection |
| resolve_file_input() | Extractor | llm/media.rs | Active | FileRef path extraction |
| create_file_ref() | Constructor | llm/media.rs | Active | FileRef creation |
| read_media_file() | Reader | llm/media.rs | Active | Media validation & encoding |
| TokenUsage | Data | core/context.rs | Active | Token accounting |
| ComparisonOp | Enum | core/graph.rs | Active | Conditional operators |
| TraceEntry + TraceStatus | Log | core/runner/types.rs | Active | Machine-centric logging |
| TranscriptEntry | Log | core/runner/types.rs | Active | Human-readable logging |
| SharedState | State | core/state.rs | Active | Execution state dict |
| HookResult | Enum | core/runner/types.rs | Active | Hook control flow |
