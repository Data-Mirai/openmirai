//! OllamaLLMResource -- real LLM resource backed by a local Ollama instance.
//!
//! Timeouts: 300s for call, 30s for embed (REGLA-506).
//! If Ollama is not available, returns a descriptive error (REGLA-507).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::core::context::{LLMResource, LLMResponse, ResourceError, TokenUsage};

/// LLM resource that connects to a local Ollama instance.
pub struct OllamaLLMResource {
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl OllamaLLMResource {
    /// Create a new Ollama LLM resource.
    ///
    /// - `base_url`: Ollama API base (e.g. `http://localhost:11434`)
    /// - `default_model`: Model to use when none is specified (e.g. `llama3.2:3b`)
    pub fn new(base_url: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            default_model: default_model.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Default configuration for local development.
    pub fn default_local() -> Self {
        Self::new("http://localhost:11434", "llama3.2:3b")
    }

    fn ollama_error(msg: &str) -> ResourceError {
        ResourceError::Llm(format!(
            "Ollama error: {msg}. Ensure Ollama is running (ollama serve) \
             and has at least one model pulled (ollama pull llama3.2:3b)"
        ))
    }
}

#[async_trait]
impl LLMResource for OllamaLLMResource {
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: &[Value],
        temperature: f64,
        max_tokens: u32,
    ) -> Result<LLMResponse, ResourceError> {
        let model = if model.is_empty() {
            &self.default_model
        } else {
            model
        };

        // Build messages array
        let mut messages: Vec<Value> = context
            .iter()
            .cloned()
            .collect();
        messages.push(json!({
            "role": "user",
            "content": prompt,
        }));

        let body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            }
        });

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .timeout(Duration::from_secs(300))
            .json(&body)
            .send()
            .await
            .map_err(|e| Self::ollama_error(&e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(Self::ollama_error(&format!(
                "HTTP {status}: {body_text}"
            )));
        }

        let resp_json: Value = response
            .json()
            .await
            .map_err(|e| Self::ollama_error(&format!("invalid JSON response: {e}")))?;

        let response_text = resp_json["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let input_tokens = resp_json["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
        let output_tokens = resp_json["eval_count"].as_u64().unwrap_or(0) as u32;

        Ok(LLMResponse {
            response: response_text,
            tokens_used: TokenUsage {
                input: input_tokens,
                output: output_tokens,
            },
            model: model.to_string(),
            provider: "ollama".to_string(),
        })
    }

    async fn embed(
        &self,
        text: &str,
        model: &str,
    ) -> Result<Vec<f64>, ResourceError> {
        let model = if model.is_empty() {
            &self.default_model
        } else {
            model
        };

        let body = json!({
            "model": model,
            "input": text,
        });

        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .timeout(Duration::from_secs(30))
            .json(&body)
            .send()
            .await
            .map_err(|e| Self::ollama_error(&e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(Self::ollama_error(&format!(
                "HTTP {status}: {body_text}"
            )));
        }

        let resp_json: Value = response
            .json()
            .await
            .map_err(|e| Self::ollama_error(&format!("invalid JSON response: {e}")))?;

        // Ollama returns { "embeddings": [[...]] }
        let embeddings = resp_json["embeddings"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_array())
            .ok_or_else(|| Self::ollama_error("missing embeddings in response"))?;

        let vec: Vec<f64> = embeddings
            .iter()
            .filter_map(|v| v.as_f64())
            .collect();

        if vec.is_empty() {
            return Err(Self::ollama_error("empty embedding vector"));
        }

        Ok(vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_local_creates_instance() {
        let resource = OllamaLLMResource::default_local();
        assert_eq!(resource.base_url, "http://localhost:11434");
        assert_eq!(resource.default_model, "llama3.2:3b");
    }

    #[test]
    fn custom_config() {
        let resource = OllamaLLMResource::new("http://gpu-server:11434", "mistral:7b");
        assert_eq!(resource.base_url, "http://gpu-server:11434");
        assert_eq!(resource.default_model, "mistral:7b");
    }

    // Integration tests require a running Ollama instance, so they are not
    // included here. Use E2E tests with Playwright for real LLM validation.
}
