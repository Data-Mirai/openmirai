//! Abstract LLM adapter interface and normalized data models.
//!
//! Every LLM provider adapter returns [`NormalizedResponse`]. The engine never
//! sees raw provider-specific formats.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Token usage counters for a single LLM call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}

/// A tool/function call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON string of the arguments object.
    pub arguments: String,
}

/// Unified response from any LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedResponse {
    pub response: String,
    pub tokens_used: TokenUsage,
    pub model: String,
    pub provider: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Single chunk from a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedChunk {
    pub delta: String,
    pub done: bool,
    pub tokens_used: Option<TokenUsage>,
}

/// Metadata about a model available in a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub supports_streaming: bool,
    pub supports_tools: bool,
}

/// A function call reference inside a [`ToolCallRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON string of the arguments object.
    pub arguments: String,
}

/// A tool call request as part of a [`Message`] (OpenAI-style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub function: FunctionCall,
}

/// A single message in a conversation (multi-turn / tool-calling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// One of: `"system"`, `"user"`, `"assistant"`, `"tool"`.
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    pub tool_call_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when calling an LLM provider.
#[derive(Debug, thiserror::Error)]
pub enum LLMError {
    #[error("Request failed (HTTP {status}): {body}")]
    RequestFailed { status: u16, body: String },

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Request timed out")]
    Timeout,

    #[error("Parse error: {0}")]
    ParseError(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstract base for LLM provider adapters.
///
/// Each concrete adapter normalizes input/output for a specific provider.
/// The engine only interacts via this interface.
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    /// Identifier of the provider (e.g. `"ollama"`, `"openai"`).
    fn provider_name(&self) -> &str;

    /// Send a simple prompt (with optional system context) and get a response.
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: Option<&str>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError>;

    /// Send a full conversation with optional tool definitions.
    ///
    /// This is the method used by the agentic loop. Unlike [`call`], it
    /// accepts a complete messages array and tool schemas, and returns
    /// [`NormalizedResponse`] with `tool_calls` populated when the LLM
    /// decides to invoke a tool.
    async fn call_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError>;

    /// List models available from this provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError>;
}
