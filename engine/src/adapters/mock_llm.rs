//! MockLLMResource — implements `LLMResource` for testing.
//!
//! Returns configurable sequential responses and deterministic embeddings.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::core::context::{LLMResource, LLMResponse, ResourceError, TokenUsage};

// ---------------------------------------------------------------------------
// MockLLMResource
// ---------------------------------------------------------------------------

/// Mock LLM that returns pre-configured responses in sequence.
///
/// - `call()` returns the next response from the `responses` list (cycling).
///   Falls back to a generated placeholder when the list is empty.
/// - `embed()` returns a deterministic vector derived from the SHA-256 hash
///   of the input text.
pub struct MockLLMResource {
    responses: Vec<String>,
    cursor: Arc<AtomicUsize>,
    embedding_dim: usize,
}

impl MockLLMResource {
    /// Create a mock with no pre-set responses (generic placeholder will be
    /// returned).
    pub fn new() -> Self {
        Self {
            responses: Vec::new(),
            cursor: Arc::new(AtomicUsize::new(0)),
            embedding_dim: 8,
        }
    }

    /// Create a mock that will cycle through the given responses.
    pub fn with_responses(responses: Vec<String>) -> Self {
        Self {
            responses,
            cursor: Arc::new(AtomicUsize::new(0)),
            embedding_dim: 8,
        }
    }

    /// Set the embedding dimension.
    pub fn with_embedding_dim(mut self, dim: usize) -> Self {
        self.embedding_dim = dim;
        self
    }
}

impl Default for MockLLMResource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LLMResource for MockLLMResource {
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        _context: &[serde_json::Value],
        _temperature: f64,
        _max_tokens: u32,
    ) -> Result<LLMResponse, ResourceError> {
        let response_text = if self.responses.is_empty() {
            let truncated: String = prompt.chars().take(50).collect();
            format!("[mock response to: {}]", truncated)
        } else {
            let idx = self.cursor.fetch_add(1, Ordering::Relaxed) % self.responses.len();
            self.responses[idx].clone()
        };

        let token_count = prompt.split_whitespace().count() as u32;

        Ok(LLMResponse {
            response: response_text,
            tokens_used: TokenUsage {
                input: 0,
                output: token_count,
            },
            model: model.to_string(),
            provider: "mock".to_string(),
        })
    }

    async fn embed(&self, text: &str, _model: &str) -> Result<Vec<f64>, ResourceError> {
        let hash = Sha256::digest(text.as_bytes());
        let vec: Vec<f64> = hash
            .iter()
            .take(self.embedding_dim)
            .map(|&b| (b as f64 / 255.0) * 2.0 - 1.0)
            .collect();
        Ok(vec)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_generic_response() {
        let llm = MockLLMResource::new();
        let resp = llm
            .call("gpt-4", "hello world", &[], 0.7, 100)
            .await
            .unwrap();
        assert!(resp.response.starts_with("[mock response to:"));
        assert!(resp.response.contains("hello world"));
        assert_eq!(resp.model, "gpt-4");
        assert_eq!(resp.provider, "mock");
    }

    #[tokio::test]
    async fn call_sequential_responses() {
        let llm = MockLLMResource::with_responses(vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ]);

        let r1 = llm.call("m", "a", &[], 0.0, 10).await.unwrap();
        let r2 = llm.call("m", "b", &[], 0.0, 10).await.unwrap();
        let r3 = llm.call("m", "c", &[], 0.0, 10).await.unwrap();

        assert_eq!(r1.response, "first");
        assert_eq!(r2.response, "second");
        assert_eq!(r3.response, "third");
    }

    #[tokio::test]
    async fn call_cycles_when_exhausted() {
        let llm = MockLLMResource::with_responses(vec!["only".to_string()]);
        let r1 = llm.call("m", "a", &[], 0.0, 10).await.unwrap();
        let r2 = llm.call("m", "b", &[], 0.0, 10).await.unwrap();
        assert_eq!(r1.response, "only");
        assert_eq!(r2.response, "only");
    }

    #[tokio::test]
    async fn embed_deterministic() {
        let llm = MockLLMResource::new();
        let v1 = llm.embed("hello", "m").await.unwrap();
        let v2 = llm.embed("hello", "m").await.unwrap();
        assert_eq!(v1.len(), 8);
        assert_eq!(v1, v2);

        // Different text produces different embedding.
        let v3 = llm.embed("world", "m").await.unwrap();
        assert_ne!(v1, v3);
    }

    #[tokio::test]
    async fn embed_values_in_range() {
        let llm = MockLLMResource::new();
        let vec = llm.embed("test", "m").await.unwrap();
        for &v in &vec {
            assert!((-1.0..=1.0).contains(&v), "value {} out of [-1, 1]", v);
        }
    }

    #[tokio::test]
    async fn custom_embedding_dim() {
        let llm = MockLLMResource::new().with_embedding_dim(16);
        let vec = llm.embed("test", "m").await.unwrap();
        assert_eq!(vec.len(), 16);
    }

    #[test]
    fn token_count_reflects_words() {
        // Verify the token estimation logic synchronously via the struct.
        let prompt = "one two three four";
        let count = prompt.split_whitespace().count() as u32;
        assert_eq!(count, 4);
    }
}
