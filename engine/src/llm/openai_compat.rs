//! OpenAI-compatible base adapter — shared by OpenAI, Groq, NVIDIA NIM, OpenRouter.
//!
//! All these providers expose the same `/chat/completions` and `/models`
//! endpoints. The only differences are base URL, default model, provider name,
//! and (for OpenRouter) extra request headers.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::adapter::{
    LLMAdapter, LLMError, Message, ModelInfo, NormalizedResponse, TokenUsage, ToolCall,
};

// ---------------------------------------------------------------------------
// Adapter struct
// ---------------------------------------------------------------------------

/// Generic adapter for any provider that speaks the OpenAI chat completions API.
pub struct OpenAICompatAdapter {
    api_key: String,
    base_url: String,
    provider: String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    #[allow(dead_code)]
    timeout: Duration,
    client: Client,
}

impl OpenAICompatAdapter {
    /// Build a new adapter.
    ///
    /// * `api_key`        – Bearer token for the provider.
    /// * `base_url`       – Root URL **including** the `/v1` prefix for providers
    ///                      that need it (e.g. `https://api.groq.com/openai/v1`).
    /// * `provider`       – Logical name (`"openai"`, `"groq"`, ...).
    /// * `default_model`  – Fallback model if caller does not specify one.
    /// * `extra_headers`  – Additional per-request headers (e.g. OpenRouter referrer).
    /// * `timeout`        – HTTP request timeout.
    pub fn new(
        api_key: &str,
        base_url: &str,
        provider: &str,
        default_model: &str,
        extra_headers: HashMap<String, String>,
        timeout: Duration,
    ) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");

        Self {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            provider: provider.to_string(),
            default_model: default_model.to_string(),
            extra_headers,
            timeout,
            client,
        }
    }

    /// Returns the default model for this provider.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Build the standard headers (Authorization + Content-Type + extras).
    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

        let mut map = HeaderMap::new();
        map.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .expect("invalid api_key for header"),
        );
        map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (k, v) in &self.extra_headers {
            if let (Ok(name), Ok(val)) = (
                HeaderName::try_from(k.as_str()),
                HeaderValue::from_str(v),
            ) {
                map.insert(name, val);
            }
        }
        map
    }

    /// Convert our [`Message`] vec into the JSON array OpenAI expects.
    fn convert_messages(messages: &[Message]) -> Vec<Value> {
        messages
            .iter()
            .map(|msg| {
                let mut obj = json!({ "role": msg.role });

                let has_media = msg.media.as_ref().is_some_and(|m| !m.is_empty());
                if has_media {
                    let mut parts: Vec<Value> = Vec::new();
                    for mc in msg.media.as_ref().unwrap() {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", mc.mime_type, mc.data),
                            }
                        }));
                    }
                    if let Some(ref content) = msg.content {
                        if !content.is_empty() {
                            parts.push(json!({"type": "text", "text": content}));
                        }
                    }
                    obj["content"] = Value::Array(parts);
                } else if let Some(ref content) = msg.content {
                    obj["content"] = Value::String(content.clone());
                }

                if let Some(ref tool_calls) = msg.tool_calls {
                    let tcs: Vec<Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": tc.function.arguments,
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = Value::Array(tcs);
                }

                if let Some(ref tool_call_id) = msg.tool_call_id {
                    obj["tool_call_id"] = Value::String(tool_call_id.clone());
                }

                obj
            })
            .collect()
    }

}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMAdapter for OpenAICompatAdapter {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: Option<&str>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError> {
        let mut messages: Vec<Value> = Vec::new();
        if let Some(ctx) = context {
            messages.push(json!({ "role": "system", "content": ctx }));
        }
        messages.push(json!({ "role": "user", "content": prompt }));

        let payload = json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
            .map_err(super::error::map_reqwest_error)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LLMError::RequestFailed {
                status: status.as_u16(),
                body,
            });
        }

        let data: Value = resp
            .json()
            .await
            .map_err(|e| LLMError::ParseError(e.to_string()))?;

        let choice = data
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or("");

        let usage = data.get("usage").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: choice.to_string(),
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: data
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(model)
                .to_string(),
            provider: self.provider.clone(),
            tool_calls: vec![],
        })
    }

    async fn call_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError> {
        let openai_messages = Self::convert_messages(&messages);

        let mut payload = json!({
            "model": model,
            "messages": openai_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        if let Some(tool_defs) = tools {
            payload["tools"] = Value::Array(tool_defs);
            payload["tool_choice"] = json!("auto");
        }

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
            .map_err(super::error::map_reqwest_error)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LLMError::RequestFailed {
                status: status.as_u16(),
                body,
            });
        }

        let data: Value = resp
            .json()
            .await
            .map_err(|e| LLMError::ParseError(e.to_string()))?;

        let choice = data
            .pointer("/choices/0/message")
            .cloned()
            .unwrap_or(json!({}));

        let text = choice
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Parse tool_calls from the response.
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        if let Some(tcs) = choice.get("tool_calls").and_then(Value::as_array) {
            for tc in tcs {
                let id = tc
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let func = tc.get("function").cloned().unwrap_or(json!({}));
                let name = func
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let arguments = func
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }

        let usage = data.get("usage").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: text,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: data
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(model)
                .to_string(),
            provider: self.provider.clone(),
            tool_calls,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(super::error::map_reqwest_error)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LLMError::RequestFailed {
                status: status.as_u16(),
                body,
            });
        }

        let data: Value = resp
            .json()
            .await
            .map_err(|e| LLMError::ParseError(e.to_string()))?;

        let models_array = data
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let models: Vec<ModelInfo> = models_array
            .iter()
            .map(|m| {
                let id = m
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                ModelInfo {
                    id: id.clone(),
                    name: id,
                    context_window: None,
                    supports_streaming: true,
                    supports_tools: true,
                }
            })
            .collect();

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::adapter::{FunctionCall, ToolCallRequest};

    #[test]
    fn convert_messages_basic() {
        let msgs = vec![
            Message {
                role: "system".into(),
                content: Some("You are helpful.".into()),
                tool_calls: None,
                tool_call_id: None,
                media: None,
            },
            Message {
                role: "user".into(),
                content: Some("Hello".into()),
                tool_calls: None,
                tool_call_id: None,
                media: None,
            },
        ];
        let converted = OpenAICompatAdapter::convert_messages(&msgs);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[0]["content"], "You are helpful.");
        assert_eq!(converted[1]["role"], "user");
        assert_eq!(converted[1]["content"], "Hello");
    }

    #[test]
    fn convert_messages_with_tool_calls() {
        let msgs = vec![Message {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCallRequest {
                id: "call_abc".into(),
                function: FunctionCall {
                    name: "get_weather".into(),
                    arguments: r#"{"city":"London"}"#.into(),
                },
            }]),
            tool_call_id: None,
            media: None,
        }];
        let converted = OpenAICompatAdapter::convert_messages(&msgs);
        assert_eq!(converted.len(), 1);

        let tc = &converted[0]["tool_calls"][0];
        assert_eq!(tc["id"], "call_abc");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        // OpenAI format keeps arguments as a JSON string.
        assert_eq!(tc["function"]["arguments"], r#"{"city":"London"}"#);
    }

    #[test]
    fn convert_messages_tool_result() {
        let msgs = vec![Message {
            role: "tool".into(),
            content: Some(r#"{"temp": 20}"#.into()),
            tool_calls: None,
            tool_call_id: Some("call_abc".into()),
            media: None,
        }];
        let converted = OpenAICompatAdapter::convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "tool");
        assert_eq!(converted[0]["tool_call_id"], "call_abc");
        assert_eq!(converted[0]["content"], r#"{"temp": 20}"#);
    }

    #[test]
    fn default_model_accessor() {
        let adapter = OpenAICompatAdapter::new(
            "test-key",
            "http://localhost",
            "test",
            "gpt-4o-mini",
            HashMap::new(),
            Duration::from_secs(30),
        );
        assert_eq!(adapter.default_model(), "gpt-4o-mini");
        assert_eq!(adapter.provider_name(), "test");
    }

    #[test]
    fn extra_headers_included() {
        let mut extras = HashMap::new();
        extras.insert("X-Custom".to_string(), "value123".to_string());

        let adapter = OpenAICompatAdapter::new(
            "key",
            "http://localhost",
            "test",
            "model",
            extras,
            Duration::from_secs(30),
        );
        let headers = adapter.headers();
        assert_eq!(headers.get("X-Custom").unwrap(), "value123");
        assert!(headers.get("authorization").is_some());
        assert!(headers.get("content-type").is_some());
    }

    #[test]
    fn user_message_with_image_media_produces_content_array() {
        use crate::llm::media::MediaContent;
        let msgs = vec![Message {
            role: "user".into(),
            content: Some("Describe this".into()),
            tool_calls: None,
            tool_call_id: None,
            media: Some(vec![MediaContent {
                mime_type: "image/jpeg".into(),
                data: "dGVzdA==".into(),
                source_path: None,
            }]),
        }];
        let converted = OpenAICompatAdapter::convert_messages(&msgs);
        assert_eq!(converted.len(), 1);

        let content = converted[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image_url");
        assert!(content[0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "Describe this");
    }

    #[test]
    fn user_message_without_media_keeps_string_content() {
        let msgs = vec![Message {
            role: "user".into(),
            content: Some("Hello".into()),
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }];
        let converted = OpenAICompatAdapter::convert_messages(&msgs);
        // Without media, content is a string, not an array.
        assert_eq!(converted[0]["content"], "Hello");
    }
}
