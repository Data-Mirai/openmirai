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

// TokenUsage is re-exported from core::context (canonical location).
pub use crate::core::context::TokenUsage;

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
    /// Optional media attachments for multimodal messages (PRD-009).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<super::media::MediaContent>>,
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

    /// PRD-018: the provider stopped generating because the output hit the
    /// token limit (`finishReason == MAX_TOKENS`). Partial text is NEVER
    /// returned as success — a silent cut is the worst product failure.
    #[error("Generation truncated by token limit: {0}")]
    Truncated(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Callback type for streaming token events.
///
/// Receives each text delta as it arrives from the provider.
pub type OnTokenFn = dyn Fn(&str) + Send + Sync;

/// Abstract base for LLM provider adapters.
///
/// Each concrete adapter normalizes input/output for a specific provider
/// (Claude, Gemini, Ollama, OpenAI-compatible, etc.).
///
/// # Two-layer LLM architecture
///
/// The engine has two LLM traits by design:
///
/// ```text
/// LLMAdapter (this trait)              LLMResource (core/context.rs)
/// ├── Provider-specific HTTP details   ├── Simplified domain interface
/// ├── Messages + tool calling          ├── Prompt-in, response-out
/// ├── Streaming support                ├── Embeddings
/// ├── Model listing                    └── Used by tools + runner
/// └── Implemented per-provider
///           │
///           └── AdapterBridgeLLMResource (adapters/adapter_bridge.rs)
///               bridges LLMAdapter → LLMResource
/// ```
///
/// **Why two traits?** `LLMAdapter` is infrastructure (how to talk to a
/// provider's HTTP API). `LLMResource` is domain (what a tool needs to
/// call an LLM). Keeping them separate means tools never deal with
/// provider-specific details, and adding a new provider doesn't touch
/// the execution engine.
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
        max_tokens: Option<u32>,
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
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError>;

    /// Send a full conversation via streaming, calling `on_token` for each
    /// text delta received.
    ///
    /// The default implementation falls back to the non-streaming
    /// [`call_with_messages`]. Providers that support streaming override this.
    async fn stream_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: Option<u32>,
        on_token: Option<&OnTokenFn>,
    ) -> Result<NormalizedResponse, LLMError> {
        // Suppress unused-variable warning for providers that don't stream.
        let _ = on_token;
        self.call_with_messages(model, messages, tools, temperature, max_tokens)
            .await
    }

    /// List models available from this provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal adapter that records whether `call_with_messages` was called,
    /// used to verify that the default `stream_with_messages` falls back.
    struct StubAdapter;

    #[async_trait]
    impl LLMAdapter for StubAdapter {
        fn provider_name(&self) -> &str {
            "stub"
        }

        async fn call(
            &self,
            _model: &str,
            _prompt: &str,
            _context: Option<&str>,
            _temperature: f32,
            _max_tokens: Option<u32>,
        ) -> Result<NormalizedResponse, LLMError> {
            unimplemented!("not needed for this test")
        }

        async fn call_with_messages(
            &self,
            model: &str,
            _messages: Vec<Message>,
            _tools: Option<Vec<Value>>,
            _temperature: f32,
            _max_tokens: Option<u32>,
        ) -> Result<NormalizedResponse, LLMError> {
            Ok(NormalizedResponse {
                response: "fallback_response".to_string(),
                tokens_used: TokenUsage {
                    input: 10,
                    output: 20,
                },
                model: model.to_string(),
                provider: "stub".to_string(),
                tool_calls: vec![],
            })
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn default_stream_with_messages_falls_back_to_call_with_messages() {
        let adapter = StubAdapter;
        let messages = vec![Message {
            role: "user".into(),
            content: Some("test".into()),
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }];

        let result = adapter
            .stream_with_messages("test-model", messages, None, 0.7, Some(100), None)
            .await
            .unwrap();

        // Verify it used the call_with_messages fallback.
        assert_eq!(result.response, "fallback_response");
        assert_eq!(result.provider, "stub");
        assert_eq!(result.model, "test-model");
        assert_eq!(result.tokens_used.input, 10);
        assert_eq!(result.tokens_used.output, 20);
    }
}
