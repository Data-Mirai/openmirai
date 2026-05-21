//! Groq adapter — fast inference via OpenAI-compatible API.

use std::collections::HashMap;
use std::time::Duration;

use super::openai_compat::OpenAICompatAdapter;

/// Default base URL for the Groq API.
const BASE_URL: &str = "https://api.groq.com/openai/v1";

/// Default model for Groq.
const DEFAULT_MODEL: &str = "qwen-qwq-32b";

/// Build a [`OpenAICompatAdapter`] configured for Groq.
///
/// This is intentionally a thin factory — Groq uses the standard
/// OpenAI chat completions format with no extra headers or quirks.
pub fn new(api_key: &str) -> OpenAICompatAdapter {
    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "groq",
        DEFAULT_MODEL,
        HashMap::new(),
        Duration::from_secs(60),
    )
}

/// Build a Groq adapter with a custom timeout.
pub fn with_timeout(api_key: &str, timeout: Duration) -> OpenAICompatAdapter {
    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "groq",
        DEFAULT_MODEL,
        HashMap::new(),
        timeout,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LLMAdapter;

    #[test]
    fn groq_adapter_defaults() {
        let adapter = new("test-key");
        assert_eq!(adapter.provider_name(), "groq");
        assert_eq!(adapter.default_model(), DEFAULT_MODEL);
    }
}
