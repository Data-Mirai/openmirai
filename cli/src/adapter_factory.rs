//! Create LLM adapters from provider/key/url configuration.

use std::time::Duration;

use openmirai_engine::llm::{LLMAdapter, OllamaAdapter, OpenAICompatAdapter};

/// Create the right adapter for the given provider string.
///
/// Returns a boxed trait object that the terminal can use for any provider.
pub fn create_adapter(
    provider: &str,
    api_key: &str,
    base_url: &str,
) -> Box<dyn LLMAdapter> {
    match provider {
        "ollama" => {
            let url = if base_url.is_empty() {
                std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string())
            } else {
                base_url.to_string()
            };
            Box::new(OllamaAdapter::new(&url, Duration::from_secs(300)))
        }
        "claude" | "anthropic" => {
            let key = resolve_key(api_key, "ANTHROPIC_API_KEY");
            if key.is_empty() {
                eprintln!("Warning: Provider 'claude' requires API key. Set --api-key or ANTHROPIC_API_KEY env var.");
            }
            let url = if base_url.is_empty() {
                "https://api.anthropic.com/v1"
            } else {
                base_url
            };
            Box::new(openmirai_engine::llm::ClaudeAdapter::with_options(
                &key,
                url,
                Duration::from_secs(120),
            ))
        }
        "gemini" | "google" => {
            let key = resolve_key(api_key, "GOOGLE_API_KEY");
            if key.is_empty() {
                eprintln!("Warning: Provider 'gemini' requires API key. Set --api-key or GOOGLE_API_KEY env var.");
            }
            let url = if base_url.is_empty() {
                "https://generativelanguage.googleapis.com/v1beta"
            } else {
                base_url
            };
            Box::new(openmirai_engine::llm::GeminiAdapter::with_options(
                &key,
                url,
                Duration::from_secs(120),
            ))
        }
        "groq" => {
            let key = resolve_key(api_key, "GROQ_API_KEY");
            Box::new(openmirai_engine::llm::groq::new(&key))
        }
        "nvidia" => {
            let key = resolve_key(api_key, "NVIDIA_API_KEY");
            Box::new(openmirai_engine::llm::nvidia::new(&key))
        }
        "openrouter" => {
            let key = resolve_key(api_key, "OPENROUTER_API_KEY");
            Box::new(openmirai_engine::llm::openrouter::new(&key))
        }
        "openai" => {
            let key = resolve_key(api_key, "OPENAI_API_KEY");
            let url = if base_url.is_empty() {
                "https://api.openai.com/v1"
            } else {
                base_url
            };
            Box::new(OpenAICompatAdapter::new(
                &key,
                url,
                "openai",
                "gpt-4o",
                std::collections::HashMap::new(),
                Duration::from_secs(120),
            ))
        }
        _ => {
            // Treat unknown providers as OpenAI-compatible with a custom base_url.
            let key = if api_key.is_empty() { "none" } else { api_key };
            let url = if base_url.is_empty() {
                "http://localhost:11434/v1"
            } else {
                base_url
            };
            Box::new(OpenAICompatAdapter::new(
                key,
                url,
                provider,
                "",
                std::collections::HashMap::new(),
                Duration::from_secs(120),
            ))
        }
    }
}

/// Default model for each known provider.
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "ollama" => "qwen3:8b",
        "groq" => "qwen-qwq-32b",
        "nvidia" => "meta/llama-3.3-70b-instruct",
        "openai" => "gpt-4o",
        "claude" | "anthropic" => "claude-sonnet-4-20250514",
        "gemini" | "google" => "gemini-2.5-flash",
        "openrouter" => "meta-llama/llama-3.3-70b-instruct",
        _ => "qwen3:8b",
    }
}

/// Resolve an API key: explicit value > env var > empty string.
fn resolve_key(explicit: &str, env_name: &str) -> String {
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    std::env::var(env_name).unwrap_or_default()
}
