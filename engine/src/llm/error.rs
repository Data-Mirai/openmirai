//! Shared error-mapping helpers for LLM adapters.

use super::LLMError;

/// Map a `reqwest::Error` to the appropriate `LLMError` variant.
///
/// Used by every HTTP-based LLM adapter (Claude, Gemini, Ollama, OpenAI-compat).
pub(crate) fn map_reqwest_error(err: reqwest::Error) -> LLMError {
    // Security: strip the URL before stringifying. Some providers (Gemini)
    // put the API key in the query string, and reqwest's Display includes
    // the full URL — without this, the key would leak into error logs.
    let err = err.without_url();
    if err.is_timeout() {
        LLMError::Timeout
    } else if err.is_connect() {
        LLMError::ConnectionError(err.to_string())
    } else if let Some(status) = err.status() {
        LLMError::RequestFailed {
            status: status.as_u16(),
            body: err.to_string(),
        }
    } else {
        LLMError::ConnectionError(err.to_string())
    }
}
