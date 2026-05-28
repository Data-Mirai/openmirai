use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ResourceError {
    #[error("database error: {0}")]
    Database(String),
    #[error("llm error: {0}")]
    Llm(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// AuthContext + Role
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Editor,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthContext {
    pub user_id: String,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub universe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
}

// ---------------------------------------------------------------------------
// LLMResponse + TokenUsage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub response: String,
    pub tokens_used: TokenUsage,
    pub model: String,
    pub provider: String,
}

// ---------------------------------------------------------------------------
// Resource traits
// ---------------------------------------------------------------------------

#[async_trait]
pub trait DBResource: Send + Sync {
    /// Execute a write query (INSERT, UPDATE, DELETE, DDL).
    async fn execute(
        &self,
        query: &str,
        params: &[serde_json::Value],
    ) -> Result<serde_json::Value, ResourceError>;

    /// Fetch a single row, returning `None` when no rows match.
    async fn fetch_one(
        &self,
        query: &str,
        params: &[serde_json::Value],
    ) -> Result<Option<serde_json::Value>, ResourceError>;

    /// Fetch all matching rows.
    async fn fetch_all(
        &self,
        query: &str,
        params: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, ResourceError>;
}

/// Domain-level LLM interface used by tools and the graph runner.
///
/// This is the simplified "port" side of the LLM abstraction. It exposes
/// only what the execution engine needs: prompt-based completion and
/// embeddings. Provider-specific details (message arrays, tool calling,
/// streaming) live in [`LLMAdapter`](crate::llm::LLMAdapter).
///
/// See `LLMAdapter` doc comment for the full two-layer architecture rationale.
#[async_trait]
pub trait LLMResource: Send + Sync {
    /// Send a prompt to the model and get a completion back.
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: &[serde_json::Value],
        temperature: f64,
        max_tokens: u32,
    ) -> Result<LLMResponse, ResourceError>;

    /// Produce an embedding vector for the given text.
    async fn embed(
        &self,
        text: &str,
        model: &str,
    ) -> Result<Vec<f64>, ResourceError>;

    /// Name of the underlying LLM provider (e.g. `"gemini"`, `"claude"`).
    fn provider_name(&self) -> &str {
        "unknown"
    }
}

#[async_trait]
pub trait StorageResource: Send + Sync {
    /// Read a blob by path.
    async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError>;

    /// Write (or overwrite) a blob at path.
    async fn put(&self, path: &str, data: &[u8]) -> Result<(), ResourceError>;

    /// Delete a blob at path.
    async fn delete(&self, path: &str) -> Result<(), ResourceError>;
}

/// Result from a vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchResult {
    pub id: String,
    pub score: f64,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait VectorResource: Send + Sync {
    /// Insert or update a document in the vector store.
    async fn upsert(
        &self,
        id: &str,
        text: &str,
        metadata: serde_json::Value,
    ) -> Result<(), ResourceError>;

    /// Search for similar documents. Returns up to `top_k` results.
    async fn search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<VectorSearchResult>, ResourceError>;

    /// Delete a document by ID.
    async fn delete(&self, id: &str) -> Result<(), ResourceError>;
}

// ---------------------------------------------------------------------------
// ExecutionContext
// ---------------------------------------------------------------------------

/// The execution context available to every block during graph execution.
///
/// Provides access to resources (DB, LLM, storage), auth info, and
/// session metadata.
pub trait ExecutionContext: Send + Sync {
    /// Database resource, if one has been configured.
    fn db(&self) -> Option<&dyn DBResource>;

    /// LLM resource (always available).
    fn llm(&self) -> &dyn LLMResource;

    /// Object storage resource, if one has been configured.
    fn storage(&self) -> Option<&dyn StorageResource>;

    /// Vector search resource, if one has been configured.
    fn vector(&self) -> Option<&dyn VectorResource>;

    /// Authentication / authorization context for this execution.
    fn auth(&self) -> &AuthContext;

    /// Unique identifier for the current session.
    fn session_id(&self) -> &str;

    /// Identifier of the node currently being executed, if any.
    fn node_id(&self) -> Option<&str>;

    /// Optional system prompt that should be prepended to LLM calls.
    fn system_prompt(&self) -> Option<&str>;

    /// Path to the scratch directory for this execution (PRD-010).
    /// Tools can write temporary files here. Cleaned up after execution.
    fn scratch_dir(&self) -> Option<&str> {
        None
    }
}

// ---------------------------------------------------------------------------
// InMemoryContext (available in tests across the crate)
// ---------------------------------------------------------------------------

/// Minimal in-memory implementation of [`ExecutionContext`] for testing tools
/// that don't need DB, LLM, or storage resources.
///
/// Available only in test builds (`#[cfg(test)]`).
#[cfg(test)]
pub struct InMemoryContext {
    session_id: String,
    auth: AuthContext,
    llm: StubLLM,
}

#[cfg(test)]
struct StubLLM;

#[cfg(test)]
#[async_trait]
impl LLMResource for StubLLM {
    async fn call(
        &self,
        _model: &str,
        _prompt: &str,
        _context: &[serde_json::Value],
        _temperature: f64,
        _max_tokens: u32,
    ) -> Result<LLMResponse, ResourceError> {
        Ok(LLMResponse {
            response: "stub".into(),
            tokens_used: TokenUsage { input: 0, output: 0 },
            model: "stub".into(),
            provider: "stub".into(),
        })
    }
    async fn embed(&self, _text: &str, _model: &str) -> Result<Vec<f64>, ResourceError> {
        Ok(vec![0.0; 4])
    }
}

#[cfg(test)]
impl InMemoryContext {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.into(),
            auth: AuthContext {
                user_id: "test-user".into(),
                role: Role::Owner,
                universe_id: None,
                environment_id: None,
            },
            llm: StubLLM,
        }
    }
}

#[cfg(test)]
impl ExecutionContext for InMemoryContext {
    fn db(&self) -> Option<&dyn DBResource> {
        None
    }
    fn llm(&self) -> &dyn LLMResource {
        &self.llm
    }
    fn storage(&self) -> Option<&dyn StorageResource> {
        None
    }
    fn vector(&self) -> Option<&dyn VectorResource> {
        None
    }
    fn auth(&self) -> &AuthContext {
        &self.auth
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn node_id(&self) -> Option<&str> {
        None
    }
    fn system_prompt(&self) -> Option<&str> {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_serde_roundtrip() {
        let role = Role::Editor;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"editor\"");
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Role::Editor);
    }

    #[test]
    fn auth_context_serde() {
        let ctx = AuthContext {
            user_id: "u-123".into(),
            role: Role::Owner,
            universe_id: Some("uni-1".into()),
            environment_id: None,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"user_id\":\"u-123\""));
        assert!(!json.contains("environment_id"));

        let back: AuthContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, Role::Owner);
        assert!(back.environment_id.is_none());
    }

    #[test]
    fn llm_response_serde() {
        let resp = LLMResponse {
            response: "Hello".into(),
            tokens_used: TokenUsage {
                input: 10,
                output: 5,
            },
            model: "gpt-4".into(),
            provider: "openai".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: LLMResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tokens_used.input, 10);
        assert_eq!(back.tokens_used.output, 5);
    }
}
