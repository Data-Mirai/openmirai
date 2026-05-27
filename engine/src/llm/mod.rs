//! LLM adapter layer — abstract interface and provider implementations.
//!
//! Every LLM provider adapter returns [`NormalizedResponse`]. The engine never
//! sees raw provider-specific formats.

mod adapter;
mod claude;
mod error;
mod gemini;
pub mod groq;
pub mod nvidia;
mod ollama;
mod openai_compat;
pub mod openrouter;

pub use adapter::{
    FunctionCall, LLMAdapter, LLMError, Message, ModelInfo, NormalizedChunk, NormalizedResponse,
    OnTokenFn, TokenUsage, ToolCall, ToolCallRequest,
};
pub use claude::ClaudeAdapter;
pub use gemini::GeminiAdapter;
pub use ollama::OllamaAdapter;
pub use openai_compat::OpenAICompatAdapter;
