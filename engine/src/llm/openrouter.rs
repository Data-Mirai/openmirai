//! OpenRouter adapter — multi-provider routing via OpenAI-compatible API.

use std::collections::HashMap;
use std::time::Duration;

use super::openai_compat::OpenAICompatAdapter;

/// Default base URL for the OpenRouter API.
const BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Default model for OpenRouter.
const DEFAULT_MODEL: &str = "meta-llama/llama-3.3-70b-instruct";

/// Build a [`OpenAICompatAdapter`] configured for OpenRouter.
///
/// OpenRouter requires two extra headers (`HTTP-Referer` and `X-Title`) for
/// attribution. These are included automatically.
pub fn new(api_key: &str) -> OpenAICompatAdapter {
    let mut extra_headers = HashMap::new();
    extra_headers.insert("HTTP-Referer".to_string(), "https://datamirai.com".to_string());
    extra_headers.insert("X-Title".to_string(), "OpenMirai".to_string());

    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "openrouter",
        DEFAULT_MODEL,
        extra_headers,
        Duration::from_secs(60),
    )
}

/// Build an OpenRouter adapter with custom site URL, site name, and timeout.
pub fn with_options(
    api_key: &str,
    site_url: &str,
    site_name: &str,
    timeout: Duration,
) -> OpenAICompatAdapter {
    let mut extra_headers = HashMap::new();
    if !site_url.is_empty() {
        extra_headers.insert("HTTP-Referer".to_string(), site_url.to_string());
    }
    if !site_name.is_empty() {
        extra_headers.insert("X-Title".to_string(), site_name.to_string());
    }

    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "openrouter",
        DEFAULT_MODEL,
        extra_headers,
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
    fn openrouter_adapter_defaults() {
        let adapter = new("test-key");
        assert_eq!(adapter.provider_name(), "openrouter");
        assert_eq!(adapter.default_model(), DEFAULT_MODEL);
    }

    #[test]
    fn openrouter_custom_options() {
        let adapter = with_options(
            "test-key",
            "https://mysite.com",
            "My App",
            Duration::from_secs(120),
        );
        assert_eq!(adapter.provider_name(), "openrouter");
    }
}
