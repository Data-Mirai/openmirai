//! Ollama adapter — local LLM inference via HTTP API.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

use super::adapter::{
    LLMAdapter, LLMError, Message, ModelInfo, NormalizedResponse, OnTokenFn, TokenUsage, ToolCall,
};

// ---------------------------------------------------------------------------
// Adapter struct
// ---------------------------------------------------------------------------

/// Adapter for Ollama running locally (or on a custom host).
pub struct OllamaAdapter {
    base_url: String,
    /// Stored for streaming-specific timeouts that differ from `client` defaults.
    #[allow(dead_code)]
    timeout: Duration,
    client: Client,
}

impl OllamaAdapter {
    /// Create a new adapter with the given base URL and timeout.
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            timeout,
            client,
        }
    }

    /// Create an adapter pointing to the default local Ollama instance.
    pub fn default_local() -> Self {
        Self::new("http://localhost:11434", Duration::from_secs(60))
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Strip `<think>...</think>` blocks from the response.
    ///
    /// Handles:
    /// - Normal paired tags: `<think>reasoning</think>`
    /// - Nested tags: `<think>...<think>inner</think>...</think>`
    /// - Malformed / unclosed: `<think>reasoning without closing tag`
    ///
    /// Strategy: iteratively remove the innermost `<think>...</think>` pair
    /// (one whose body contains no nested `<think>`) until none remain, then
    /// truncate at any remaining unclosed `<think>`.
    fn clean_response(text: &str) -> String {
        let mut cleaned = text.to_string();

        // Iteratively remove innermost <think>...</think> pairs.
        // An "innermost" pair is one where the content between <think> and
        // </think> does not itself contain another <think>.
        // Find the last <think> that appears before the first </think>.
        // That is guaranteed to be innermost.
        while let Some(close_pos) = cleaned.find("</think>") {
            // Search backwards from close_pos for the nearest <think>.
            let search_region = &cleaned[..close_pos];
            let open_pos = match search_region.rfind("<think>") {
                Some(p) => p,
                None => break, // orphaned </think> — leave it
            };
            let end = close_pos + "</think>".len();
            cleaned.replace_range(open_pos..end, "");
        }

        // Remove any remaining unclosed <think> tag to end of string.
        if let Some(pos) = cleaned.find("<think>") {
            cleaned.truncate(pos);
        }

        cleaned.trim().to_string()
    }

    /// Convert OpenAI-format messages to Ollama-compatible format.
    ///
    /// Ollama differences:
    /// - assistant tool_calls: no `id`/`type`, arguments is a dict (not JSON string)
    /// - tool results: no `tool_call_id`
    fn convert_messages_for_ollama(messages: &[Message]) -> Vec<Value> {
        let mut converted: Vec<Value> = Vec::with_capacity(messages.len());

        for msg in messages {
            if msg.role == "assistant" {
                if let Some(ref tool_calls) = msg.tool_calls {
                    let ollama_tcs: Vec<Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            let args: Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                            json!({
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": args,
                                }
                            })
                        })
                        .collect();

                    converted.push(json!({
                        "role": "assistant",
                        "content": msg.content.as_deref().unwrap_or(""),
                        "tool_calls": ollama_tcs,
                    }));
                    continue;
                }
            }

            if msg.role == "tool" {
                // Ollama does not use tool_call_id.
                converted.push(json!({
                    "role": "tool",
                    "content": msg.content.as_deref().unwrap_or(""),
                }));
                continue;
            }

            // system, user, plain assistant — pass through.
            let mut obj = json!({ "role": msg.role });
            if let Some(ref content) = msg.content {
                obj["content"] = Value::String(content.clone());
            }
            // Ollama multimodal: images field for user messages (PRD-009).
            if msg.role == "user" {
                if let Some(ref media_list) = msg.media {
                    let images: Vec<Value> = media_list
                        .iter()
                        .map(|mc| Value::String(mc.data.clone()))
                        .collect();
                    if !images.is_empty() {
                        obj["images"] = Value::Array(images);
                    }
                }
            }
            converted.push(obj);
        }

        converted
    }

    /// Query `/api/show` to get the context_length of a model.
    async fn get_model_context_length(&self, model_name: &str) -> Option<u32> {
        let url = format!("{}/api/show", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&json!({ "name": model_name }))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let data: Value = resp.json().await.ok()?;
        let model_info = data.get("model_info")?.as_object()?;
        for (key, value) in model_info {
            if key.ends_with(".context_length") {
                return value.as_u64().map(|v| v as u32);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMAdapter for OllamaAdapter {
    fn provider_name(&self) -> &str {
        "ollama"
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
            "stream": false,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        });

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
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

        let raw_text = data
            .pointer("/message/content")
            .and_then(Value::as_str)
            .unwrap_or("");
        let cleaned = Self::clean_response(raw_text);

        let tokens_input = data
            .get("prompt_eval_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = data.get("eval_count").and_then(Value::as_u64).unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: cleaned,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: data
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(model)
                .to_string(),
            provider: self.provider_name().to_string(),
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
        let ollama_messages = Self::convert_messages_for_ollama(&messages);

        let mut payload = json!({
            "model": model,
            "messages": ollama_messages,
            "stream": false,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        });

        if let Some(tool_defs) = tools {
            payload["tools"] = Value::Array(tool_defs);
        }

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
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

        let message = data.get("message").cloned().unwrap_or(json!({}));

        let raw_text = message.get("content").and_then(Value::as_str).unwrap_or("");
        let cleaned = Self::clean_response(raw_text);

        // Parse tool_calls from the response message.
        // Ollama returns arguments as a dict (not JSON string) and has no tool_call_id.
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        if let Some(tcs) = message.get("tool_calls").and_then(Value::as_array) {
            for (i, tc) in tcs.iter().enumerate() {
                let func = tc.get("function").cloned().unwrap_or(json!({}));
                let name = func
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = func.get("arguments").cloned().unwrap_or(json!({}));
                let arguments = if args.is_object() || args.is_array() {
                    serde_json::to_string(&args).unwrap_or_default()
                } else {
                    args.to_string()
                };

                tool_calls.push(ToolCall {
                    id: format!("call_{i}"),
                    name,
                    arguments,
                });
            }
        }

        let tokens_input = data
            .get("prompt_eval_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = data.get("eval_count").and_then(Value::as_u64).unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: cleaned,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: data
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(model)
                .to_string(),
            provider: self.provider_name().to_string(),
            tool_calls,
        })
    }

    async fn stream_with_messages(
        &self,
        model: &str,
        messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: u32,
        on_token: Option<&OnTokenFn>,
    ) -> Result<NormalizedResponse, LLMError> {
        let ollama_messages = Self::convert_messages_for_ollama(&messages);

        let mut payload = json!({
            "model": model,
            "messages": ollama_messages,
            "stream": true,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        });

        if let Some(tool_defs) = tools {
            payload["tools"] = Value::Array(tool_defs);
        }

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
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

        let mut accumulated = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut tokens_input: u32 = 0;
        let mut tokens_output: u32 = 0;
        let mut final_model = model.to_string();

        // Buffer for incomplete JSON lines split across chunks.
        let mut line_buffer = String::new();

        let mut stream = resp.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| LLMError::ConnectionError(e.to_string()))?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            line_buffer.push_str(&chunk_str);

            // Process complete lines from the buffer.
            while let Some(newline_pos) = line_buffer.find('\n') {
                let line: String = line_buffer.drain(..=newline_pos).collect();
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let data: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Extract delta text.
                if let Some(delta) = data.pointer("/message/content").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        accumulated.push_str(delta);
                        if let Some(cb) = on_token {
                            cb(delta);
                        }
                    }
                }

                // Extract tool calls from any chunk that has them.
                if let Some(tcs) = data
                    .pointer("/message/tool_calls")
                    .and_then(Value::as_array)
                {
                    for (i, tc) in tcs.iter().enumerate() {
                        let func = tc.get("function").cloned().unwrap_or(json!({}));
                        let name = func
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let args = func.get("arguments").cloned().unwrap_or(json!({}));
                        let arguments = if args.is_object() || args.is_array() {
                            serde_json::to_string(&args).unwrap_or_default()
                        } else {
                            args.to_string()
                        };
                        tool_calls.push(ToolCall {
                            id: format!("call_{}", tool_calls.len() + i),
                            name,
                            arguments,
                        });
                    }
                }

                // Handle done=true for final token counts.
                if data.get("done").and_then(Value::as_bool) == Some(true) {
                    tokens_input = data
                        .get("prompt_eval_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    tokens_output =
                        data.get("eval_count").and_then(Value::as_u64).unwrap_or(0) as u32;
                    if let Some(m) = data.get("model").and_then(Value::as_str) {
                        final_model = m.to_string();
                    }
                }
            }
        }

        // Process any remaining data in the buffer (last line without trailing newline).
        let remaining = line_buffer.trim().to_string();
        if !remaining.is_empty() {
            if let Ok(data) = serde_json::from_str::<Value>(&remaining) {
                if let Some(delta) = data.pointer("/message/content").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        accumulated.push_str(delta);
                        if let Some(cb) = on_token {
                            cb(delta);
                        }
                    }
                }
                if data.get("done").and_then(Value::as_bool) == Some(true) {
                    tokens_input = data
                        .get("prompt_eval_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(tokens_input as u64) as u32;
                    tokens_output = data
                        .get("eval_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(tokens_output as u64) as u32;
                    if let Some(m) = data.get("model").and_then(Value::as_str) {
                        final_model = m.to_string();
                    }
                }
            }
        }

        let cleaned = Self::clean_response(&accumulated);

        Ok(NormalizedResponse {
            response: cleaned,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: final_model,
            provider: self.provider_name().to_string(),
            tool_calls,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
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
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut models: Vec<ModelInfo> = Vec::with_capacity(models_array.len());

        for m in &models_array {
            let model_name = m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let ctx_len = self.get_model_context_length(&model_name).await;

            models.push(ModelInfo {
                id: model_name.clone(),
                name: model_name,
                context_window: ctx_len,
                supports_streaming: true,
                supports_tools: false,
            });
        }

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_response_strips_think_tags() {
        let input = "Hello <think>internal reasoning</think> world";
        assert_eq!(OllamaAdapter::clean_response(input), "Hello  world");
    }

    #[test]
    fn clean_response_strips_nested_think_tags() {
        let input = "<think>outer <think>inner</think> still outer</think>visible";
        assert_eq!(OllamaAdapter::clean_response(input), "visible");
    }

    #[test]
    fn clean_response_strips_unclosed_think_tag() {
        let input = "Hello <think>unclosed reasoning that never ends";
        assert_eq!(OllamaAdapter::clean_response(input), "Hello");
    }

    #[test]
    fn clean_response_no_tags() {
        let input = "Just a normal response";
        assert_eq!(
            OllamaAdapter::clean_response(input),
            "Just a normal response"
        );
    }

    #[test]
    fn clean_response_empty_think() {
        let input = "Before <think></think> after";
        assert_eq!(OllamaAdapter::clean_response(input), "Before  after");
    }

    #[test]
    fn convert_messages_system_and_user() {
        let messages = vec![
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
        let converted = OllamaAdapter::convert_messages_for_ollama(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[1]["content"], "Hello");
    }

    #[test]
    fn convert_messages_tool_result_drops_tool_call_id() {
        let messages = vec![Message {
            role: "tool".into(),
            content: Some("{\"result\": 42}".into()),
            tool_calls: None,
            tool_call_id: Some("call_0".into()),
            media: None,
        }];
        let converted = OllamaAdapter::convert_messages_for_ollama(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "tool");
        assert!(converted[0].get("tool_call_id").is_none());
    }

    #[test]
    fn clean_response_works_on_streamed_accumulated_text() {
        // Simulate accumulated streaming output that contains think tags.
        let accumulated = "Hello <think>some reasoning about the question</think> world! \
                           <think>more thinking</think> Done.";
        let cleaned = OllamaAdapter::clean_response(accumulated);
        assert_eq!(cleaned, "Hello  world!  Done.");
    }

    #[test]
    fn clean_response_works_on_streamed_unclosed_think() {
        // Simulate streaming where think tag was never closed (stream interrupted).
        let accumulated = "Answer: 42 <think>let me reason about why";
        let cleaned = OllamaAdapter::clean_response(accumulated);
        assert_eq!(cleaned, "Answer: 42");
    }

    #[test]
    fn convert_messages_assistant_tool_calls() {
        use super::super::adapter::{FunctionCall, ToolCallRequest};

        let messages = vec![Message {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCallRequest {
                id: "call_0".into(),
                function: FunctionCall {
                    name: "get_weather".into(),
                    arguments: r#"{"city":"London"}"#.into(),
                },
            }]),
            tool_call_id: None,
            media: None,
        }];
        let converted = OllamaAdapter::convert_messages_for_ollama(&messages);
        assert_eq!(converted.len(), 1);

        let tc = &converted[0]["tool_calls"][0];
        assert_eq!(tc["function"]["name"], "get_weather");
        // Ollama expects arguments as dict, not string.
        assert!(tc["function"]["arguments"].is_object());
        assert_eq!(tc["function"]["arguments"]["city"], "London");
    }
}
