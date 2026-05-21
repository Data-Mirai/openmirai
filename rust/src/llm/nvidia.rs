//! NVIDIA NIM adapter — 100+ models via OpenAI-compatible API.

use std::collections::HashMap;
use std::time::Duration;

use super::openai_compat::OpenAICompatAdapter;

/// Default base URL for the NVIDIA NIM API.
const BASE_URL: &str = "https://integrate.api.nvidia.com/v1";

/// Default model for NVIDIA NIM.
const DEFAULT_MODEL: &str = "meta/llama-3.3-70b-instruct";

/// Build a [`OpenAICompatAdapter`] configured for NVIDIA NIM.
///
/// NVIDIA NIM exposes the standard OpenAI `/chat/completions` and `/models`
/// endpoints with no extra headers required.
pub fn new(api_key: &str) -> OpenAICompatAdapter {
    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "nvidia_nim",
        DEFAULT_MODEL,
        HashMap::new(),
        Duration::from_secs(60),
    )
}

/// Build an NVIDIA NIM adapter with a custom timeout.
pub fn with_timeout(api_key: &str, timeout: Duration) -> OpenAICompatAdapter {
    OpenAICompatAdapter::new(
        api_key,
        BASE_URL,
        "nvidia_nim",
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
    fn nvidia_adapter_defaults() {
        let adapter = new("test-key");
        assert_eq!(adapter.provider_name(), "nvidia_nim");
        assert_eq!(adapter.default_model(), DEFAULT_MODEL);
    }
}
