pub mod benchmark;
pub mod core;
pub mod db;
pub mod energy;
pub mod intelligence;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod observability;
pub mod render;
pub mod resources;
pub mod security;
pub mod runtime;
pub mod sandbox;
pub mod search;
pub mod server;
pub mod tools;
pub mod triggers;
pub mod vault;

// Re-export core types explicitly (no glob).
pub use crate::core::agent_spec::{
    AgentConfig, AgentEdgeSpec, AgentGraphSpec, AgentHookSpec, AgentMcpServerSpec, AgentNodeSpec,
    AgentResourceRef, AgentRetryConfig, AgentSpec, AgentSpecError, AgentTriggerSpec, AgentType,
};
pub use crate::core::auth::{AuthError, Permission, PermissionEvaluator, Resource, SingleUserAuth};
pub use crate::core::context::{
    AuthContext, DBResource, ExecutionContext, LLMResource, LLMResponse, ResourceError, Role,
    StorageResource, TokenUsage,
};
pub use crate::core::events::{EventEmitter, EventType, ExecutionEvent};
pub use crate::core::graph::{ComparisonOp, EdgeCondition, EdgeDef, GraphDef, GraphError, NodeDef};
pub use crate::core::runner::{
    BackoffStrategy, Checkpoint, CheckpointCallback, ExecutionResult, ExecutionStatus, FailureMode,
    GraphRunner, HookHandler, HookResult, InterruptInfo, RetryPolicy, RunnerError, ToolError,
    ToolExecutor, TraceEntry, TraceStatus, TranscriptEntry,
};
pub use crate::core::schema::{ColumnDef, ColumnType, SchemaError, TableSchema};
pub use crate::core::state::{SharedState, StateError};

// Re-export tool-system types for convenience.
pub use crate::tools::{
    FieldType, RegistryExecutor, Tool, ToolFactory, ToolField, ToolRegistry, ToolSpec,
};

// Re-export memory types.
pub use crate::memory::{
    InMemoryBackend, LongTermEntry, LongTermMemory, MemoryBackend, ShortTermEntry,
    ShortTermMemory, SqliteBackend,
};

// Re-export trigger types.
pub use crate::triggers::{TriggerConfig, TriggerDef, TriggerEvent, TriggerType};

// Re-export db types.
pub use crate::db::{
    AgentRecord, DbError, InMemoryAgentRepo, InMemoryGraphRepo, InMemorySessionRepo, Repository,
    SessionRecord, SessionStatus,
};

// Re-export runtime types.
pub use crate::runtime::{AgentRuntime, RuntimeAgentRecord, RuntimeAgentStatus, RuntimeError, Scheduler};

// Re-export resource types.
pub use crate::resources::{
    AdapterBridgeLLMResource, InMemoryDBResource, InMemoryStorageResource, MockLLMResource,
    OllamaLLMResource, SimpleExecutionContext,
};
