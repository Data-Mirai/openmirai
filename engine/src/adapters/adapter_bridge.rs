//! Bridge between `LLMAdapter` (llm module) and `LLMResource` (execution context).
//!
//! This is the [Adapter pattern](https://en.wikipedia.org/wiki/Adapter_pattern):
//! it allows any provider adapter (Ollama, OpenAI, Claude, Groq, etc.) to be
//! used as an `LLMResource` in the `ExecutionContext`.
//!
//! See `LLMAdapter` doc comment for the rationale behind the two-layer architecture.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::core::context::{LLMResource, LLMResponse, ResourceError, TokenUsage};
use crate::llm::{LLMAdapter, Message};

/// Wraps any `LLMAdapter` into a `LLMResource` for use in ExecutionContext.
pub struct AdapterBridgeLLMResource {
    adapter: Arc<dyn LLMAdapter>,
    default_model: String,
    /// Optional embed base URL (for providers that support embeddings via a
    /// separate endpoint, e.g. OpenAI /v1/embeddings).
    embed_base_url: Option<String>,
    embed_api_key: Option<String>,
}

impl AdapterBridgeLLMResource {
    pub fn new(adapter: Box<dyn LLMAdapter>, default_model: impl Into<String>) -> Self {
        Self {
            adapter: Arc::from(adapter),
            default_model: default_model.into(),
            embed_base_url: None,
            embed_api_key: None,
        }
    }

    pub fn with_embed(mut self, base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        self.embed_base_url = Some(base_url.into());
        self.embed_api_key = Some(api_key.into());
        self
    }
}

#[async_trait]
impl LLMResource for AdapterBridgeLLMResource {
    fn provider_name(&self) -> &str {
        self.adapter.provider_name()
    }

    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: &[Value],
        temperature: f64,
        max_tokens: Option<u32>,
    ) -> Result<LLMResponse, ResourceError> {
        let model = if model.is_empty() {
            &self.default_model
        } else {
            model
        };

        // Convert context Values to Message structs.
        // Values without a "role" field are silently skipped (e.g. __user_media carrier).
        let mut messages: Vec<Message> = context
            .iter()
            .filter_map(|v| {
                let role = v.get("role")?.as_str()?.to_string();
                let content = v.get("content").and_then(|c| c.as_str()).map(String::from);
                let media = v.get("media").and_then(|m| {
                    serde_json::from_value::<Vec<crate::llm::media::MediaContent>>(m.clone()).ok()
                });
                Some(Message {
                    role,
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                    media,
                })
            })
            .collect();

        // Extract media to attach to the user prompt message.
        // Tools pass media via a carrier entry (no role → skipped above).
        let user_media = context.iter().find_map(|v| {
            v.get(crate::llm::media::USER_MEDIA_KEY).and_then(|m| {
                serde_json::from_value::<Vec<crate::llm::media::MediaContent>>(m.clone()).ok()
            })
        });

        // Add the user prompt (with optional media attachment).
        messages.push(Message {
            role: "user".to_string(),
            content: Some(prompt.to_string()),
            tool_calls: None,
            tool_call_id: None,
            media: user_media,
        });

        let result = self
            .adapter
            .call_with_messages(model, messages, None, temperature as f32, max_tokens)
            .await
            .map_err(|e| ResourceError::Llm(e.to_string()))?;

        Ok(LLMResponse {
            response: result.response,
            tokens_used: TokenUsage {
                input: result.tokens_used.input,
                output: result.tokens_used.output,
            },
            model: result.model,
            provider: result.provider,
        })
    }

    async fn embed(&self, text: &str, model: &str) -> Result<Vec<f64>, ResourceError> {
        // If we have an embed URL configured, use it (OpenAI-compatible embed API)
        if let Some(ref base_url) = self.embed_base_url {
            let client = reqwest::Client::new();
            let api_key = self.embed_api_key.as_deref().unwrap_or("");

            let embed_model = if model.is_empty() {
                "text-embedding-3-small"
            } else {
                model
            };

            let body = json!({
                "model": embed_model,
                "input": text,
            });

            let mut req = client.post(format!("{base_url}/embeddings"));
            if !api_key.is_empty() {
                req = req.bearer_auth(api_key);
            }

            let response = req
                .json(&body)
                .send()
                .await
                .map_err(|e| ResourceError::Llm(format!("Embed request failed: {e}")))?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(ResourceError::Llm(format!("Embed HTTP {status}: {text}")));
            }

            let resp: Value = response
                .json()
                .await
                .map_err(|e| ResourceError::Llm(format!("Embed parse error: {e}")))?;

            let embedding = resp["data"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|d| d["embedding"].as_array())
                .ok_or_else(|| ResourceError::Llm("Missing embedding in response".into()))?;

            let vec: Vec<f64> = embedding.iter().filter_map(|v| v.as_f64()).collect();
            if vec.is_empty() {
                return Err(ResourceError::Llm("Empty embedding vector".into()));
            }
            return Ok(vec);
        }

        // For Ollama, use the /api/embed endpoint directly
        if self.adapter.provider_name() == "ollama" {
            // Delegate to a direct HTTP call since OllamaAdapter doesn't have embed
            let client = reqwest::Client::new();
            let embed_model = if model.is_empty() {
                &self.default_model
            } else {
                model
            };

            let body = json!({
                "model": embed_model,
                "input": text,
            });

            let response = client
                .post("http://localhost:11434/api/embed")
                .json(&body)
                .send()
                .await
                .map_err(|e| ResourceError::Llm(format!("Ollama embed failed: {e}")))?;

            let resp: Value = response
                .json()
                .await
                .map_err(|e| ResourceError::Llm(format!("Ollama embed parse: {e}")))?;

            let embeddings = resp["embeddings"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_array())
                .ok_or_else(|| ResourceError::Llm("Missing Ollama embeddings".into()))?;

            let vec: Vec<f64> = embeddings.iter().filter_map(|v| v.as_f64()).collect();
            return Ok(vec);
        }

        Err(ResourceError::Llm(format!(
            "Embeddings not supported for provider '{}'. Configure embed_base_url.",
            self.adapter.provider_name()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LLMError, ModelInfo, NormalizedResponse};

    struct FakeAdapter;

    #[async_trait]
    impl LLMAdapter for FakeAdapter {
        fn provider_name(&self) -> &str {
            "fake"
        }

        async fn call(
            &self,
            _model: &str,
            _prompt: &str,
            _context: Option<&str>,
            _temperature: f32,
            _max_tokens: Option<u32>,
        ) -> Result<NormalizedResponse, LLMError> {
            unimplemented!()
        }

        async fn call_with_messages(
            &self,
            model: &str,
            messages: Vec<Message>,
            _tools: Option<Vec<Value>>,
            _temperature: f32,
            _max_tokens: Option<u32>,
        ) -> Result<NormalizedResponse, LLMError> {
            let last_msg = messages
                .last()
                .and_then(|m| m.content.as_deref())
                .unwrap_or("no content");
            Ok(NormalizedResponse {
                response: format!("echo: {last_msg}"),
                tokens_used: TokenUsage {
                    input: 5,
                    output: 3,
                },
                model: model.to_string(),
                provider: "fake".to_string(),
                tool_calls: vec![],
            })
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn bridge_call_forwards_to_adapter() {
        let bridge = AdapterBridgeLLMResource::new(Box::new(FakeAdapter), "test-model");
        let context = vec![json!({"role": "system", "content": "You are helpful."})];
        let result = bridge
            .call("test-model", "hello", &context, 0.7, Some(100))
            .await
            .unwrap();
        assert_eq!(result.response, "echo: hello");
        assert_eq!(result.provider, "fake");
        assert_eq!(result.model, "test-model");
    }

    #[tokio::test]
    async fn bridge_uses_default_model_when_empty() {
        let bridge = AdapterBridgeLLMResource::new(Box::new(FakeAdapter), "my-default");
        let result = bridge.call("", "test", &[], 0.5, Some(50)).await.unwrap();
        assert_eq!(result.model, "my-default");
    }

    #[tokio::test]
    async fn embed_returns_error_when_not_configured() {
        let bridge = AdapterBridgeLLMResource::new(Box::new(FakeAdapter), "test");
        let result = bridge.embed("hello", "").await;
        assert!(result.is_err());
    }
}
