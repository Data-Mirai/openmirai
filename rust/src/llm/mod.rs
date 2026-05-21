//! LLM adapter layer — abstract interface and provider implementations.
//!
//! Every LLM provider adapter returns [`NormalizedResponse`]. The engine never
//! sees raw provider-specific formats.

mod adapter;
pub mod groq;
pub mod nvidia;
mod ollama;
mod openai_compat;
pub mod openrouter;

pub use adapter::{
    FunctionCall, LLMAdapter, LLMError, Message, ModelInfo, NormalizedChunk, NormalizedResponse,
    TokenUsage, ToolCall, ToolCallRequest,
};
pub use ollama::OllamaAdapter;
pub use openai_compat::OpenAICompatAdapter;
