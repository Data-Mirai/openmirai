//! Claude adapter — Anthropic models via HTTP API.
//!
//! Key differences from OpenAI-compatible providers:
//! - System prompt is a top-level `"system"` field, NOT a message in the array.
//! - Tool definitions use `{ name, description, input_schema }` (no `"function"` wrapper).
//! - Tool calls in responses are `tool_use` blocks inside the `content` array.
//! - Tool call arguments are a dict (`input`), not a JSON string.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::adapter::{
    LLMAdapter, LLMError, Message, ModelInfo, NormalizedResponse, TokenUsage, ToolCall,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Hardcoded model list with context windows.
const MODELS: &[(&str, u32)] = &[
    ("claude-sonnet-4-20250514", 200_000),
    ("claude-haiku-4-20250514", 200_000),
    ("claude-opus-4-20250514", 200_000),
];

// ---------------------------------------------------------------------------
// Adapter struct
// ---------------------------------------------------------------------------

/// Adapter for Anthropic's Claude models.
pub struct ClaudeAdapter {
    api_key: String,
    base_url: String,
    #[allow(dead_code)]
    timeout: Duration,
    client: Client,
}

impl ClaudeAdapter {
    /// Create a new adapter with the given API key.
    pub fn new(api_key: &str) -> Self {
        Self::with_options(api_key, DEFAULT_BASE_URL, Duration::from_secs(60))
    }

    /// Create a new adapter with custom base URL and timeout.
    pub fn with_options(api_key: &str, base_url: &str, timeout: Duration) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");

        Self {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            timeout,
            client,
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Build the Anthropic-specific headers.
    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};

        let mut map = HeaderMap::new();
        map.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).expect("invalid api_key for header"),
        );
        map.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        map
    }

    /// Convert our [`Message`] vec into the JSON array Claude expects.
    ///
    /// Claude differences from OpenAI:
    /// - `system` messages are extracted and returned separately (top-level field).
    /// - `assistant` tool_calls become `tool_use` content blocks.
    /// - `tool` results become `tool_result` content blocks in a `user` message.
    fn convert_messages(messages: &[Message]) -> (Option<String>, Vec<Value>) {
        let mut system_text: Option<String> = None;
        let mut converted: Vec<Value> = Vec::with_capacity(messages.len());

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Accumulate system messages into one string.
                    if let Some(ref content) = msg.content {
                        match system_text {
                            Some(ref mut existing) => {
                                existing.push('\n');
                                existing.push_str(content);
                            }
                            None => system_text = Some(content.clone()),
                        }
                    }
                }
                "assistant" => {
                    if let Some(ref tool_calls) = msg.tool_calls {
                        // Build content blocks: optional text + tool_use blocks.
                        let mut content_blocks: Vec<Value> = Vec::new();
                        if let Some(ref text) = msg.content {
                            if !text.is_empty() {
                                content_blocks.push(json!({
                                    "type": "text",
                                    "text": text,
                                }));
                            }
                        }
                        for tc in tool_calls {
                            let input: Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                            content_blocks.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.function.name,
                                "input": input,
                            }));
                        }
                        converted.push(json!({
                            "role": "assistant",
                            "content": content_blocks,
                        }));
                    } else {
                        converted.push(json!({
                            "role": "assistant",
                            "content": msg.content.as_deref().unwrap_or(""),
                        }));
                    }
                }
                "tool" => {
                    // Claude expects tool results as user messages with tool_result blocks.
                    let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
                    let content_str = msg.content.as_deref().unwrap_or("");
                    converted.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content_str,
                        }],
                    }));
                }
                _ => {
                    // user — build content blocks if media present, plain string otherwise.
                    let has_media = msg.media.as_ref().is_some_and(|m| !m.is_empty());
                    if has_media {
                        let mut blocks: Vec<Value> = Vec::new();
                        for mc in msg.media.as_ref().unwrap() {
                            blocks.push(json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": mc.mime_type,
                                    "data": mc.data,
                                }
                            }));
                        }
                        if let Some(ref text) = msg.content {
                            if !text.is_empty() {
                                blocks.push(json!({"type": "text", "text": text}));
                            }
                        }
                        converted.push(json!({"role": msg.role, "content": blocks}));
                    } else {
                        converted.push(json!({
                            "role": msg.role,
                            "content": msg.content.as_deref().unwrap_or(""),
                        }));
                    }
                }
            }
        }

        (system_text, converted)
    }

    /// Convert OpenAI-style tool definitions to Claude format.
    ///
    /// OpenAI: `{ type: "function", function: { name, description, parameters } }`
    /// Claude: `{ name, description, input_schema }`
    fn convert_tools(tools: &[Value]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                // Accept both OpenAI-wrapped and already-Claude-format tools.
                if let Some(func) = tool.get("function") {
                    json!({
                        "name": func.get("name").cloned().unwrap_or(json!("")),
                        "description": func.get("description").cloned().unwrap_or(json!("")),
                        "input_schema": func.get("parameters").cloned().unwrap_or(json!({})),
                    })
                } else {
                    // Assume already in Claude format.
                    tool.clone()
                }
            })
            .collect()
    }

    /// Parse tool_use blocks from a Claude response content array.
    fn parse_tool_calls(content: &[Value]) -> Vec<ToolCall> {
        content
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            .map(|block| {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(json!({}));
                let arguments = serde_json::to_string(&input).unwrap_or_default();

                ToolCall {
                    id,
                    name,
                    arguments,
                }
            })
            .collect()
    }

    /// Extract text from a Claude response content array.
    fn extract_text(content: &[Value]) -> String {
        content
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("")
    }
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMAdapter for ClaudeAdapter {
    fn provider_name(&self) -> &str {
        "claude"
    }

    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: Option<&str>,
        temperature: f32,
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError> {
        let mut payload = json!({
            "model": model,
            "max_tokens": max_tokens.unwrap_or(8192),
            "temperature": temperature,
            "messages": [{ "role": "user", "content": prompt }],
        });

        if let Some(ctx) = context {
            payload["system"] = Value::String(ctx.to_string());
        }

        let url = format!("{}/messages", self.base_url);
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

        let text = data
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let usage = data.get("usage").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("output_tokens")
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
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError> {
        let (system, claude_messages) = Self::convert_messages(&messages);

        let mut payload = json!({
            "model": model,
            "max_tokens": max_tokens.unwrap_or(8192),
            "temperature": temperature,
            "messages": claude_messages,
        });

        if let Some(sys) = &system {
            payload["system"] = Value::String(sys.clone());
        }

        if let Some(tool_defs) = tools {
            let claude_tools = Self::convert_tools(&tool_defs);
            payload["tools"] = Value::Array(claude_tools);
        }

        let url = format!("{}/messages", self.base_url);
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

        let content = data
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let text = Self::extract_text(&content);
        let tool_calls = Self::parse_tool_calls(&content);

        let usage = data.get("usage").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("output_tokens")
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
            provider: self.provider_name().to_string(),
            tool_calls,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
        Ok(MODELS
            .iter()
            .map(|(id, ctx)| ModelInfo {
                id: id.to_string(),
                name: id.to_string(),
                context_window: Some(*ctx),
                supports_streaming: true,
                supports_tools: true,
            })
            .collect())
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
    fn system_message_is_separated_not_in_messages() {
        let messages = vec![
            Message {
                role: "system".into(),
                content: Some("You are a helpful assistant.".into()),
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

        let (system, converted) = ClaudeAdapter::convert_messages(&messages);

        // System is extracted as a top-level string.
        assert_eq!(system, Some("You are a helpful assistant.".to_string()));

        // Only the user message remains in the array.
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "Hello");
    }

    #[test]
    fn tool_use_block_parsing() {
        let content = vec![
            json!({
                "type": "text",
                "text": "Let me check the weather."
            }),
            json!({
                "type": "tool_use",
                "id": "toolu_01ABC",
                "name": "get_weather",
                "input": { "city": "London", "units": "celsius" }
            }),
        ];

        let tool_calls = ClaudeAdapter::parse_tool_calls(&content);
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_01ABC");
        assert_eq!(tool_calls[0].name, "get_weather");

        // Arguments are serialized to JSON string (normalized format).
        let args: Value = serde_json::from_str(&tool_calls[0].arguments).unwrap();
        assert_eq!(args["city"], "London");
        assert_eq!(args["units"], "celsius");

        // Text extraction also works alongside tool_use blocks.
        let text = ClaudeAdapter::extract_text(&content);
        assert_eq!(text, "Let me check the weather.");
    }

    #[test]
    fn hardcoded_model_list() {
        // Verify the hardcoded list contains the expected models.
        let model_ids: Vec<&str> = MODELS.iter().map(|(id, _)| *id).collect();
        assert!(model_ids.contains(&"claude-sonnet-4-20250514"));
        assert!(model_ids.contains(&"claude-haiku-4-20250514"));
        assert!(model_ids.contains(&"claude-opus-4-20250514"));
        assert_eq!(model_ids.len(), 3);
    }

    #[test]
    fn tool_conversion_from_openai_format() {
        let openai_tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather for a city",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                }
            }
        })];

        let claude_tools = ClaudeAdapter::convert_tools(&openai_tools);
        assert_eq!(claude_tools.len(), 1);
        assert_eq!(claude_tools[0]["name"], "get_weather");
        assert_eq!(claude_tools[0]["description"], "Get the weather for a city");
        assert!(claude_tools[0].get("input_schema").is_some());
        // No "function" wrapper in Claude format.
        assert!(claude_tools[0].get("function").is_none());
    }

    #[test]
    fn convert_messages_assistant_with_tool_calls() {
        let messages = vec![Message {
            role: "assistant".into(),
            content: Some("I'll look that up.".into()),
            tool_calls: Some(vec![ToolCallRequest {
                id: "toolu_01".into(),
                function: FunctionCall {
                    name: "search".into(),
                    arguments: r#"{"query":"rust"}"#.into(),
                },
            }]),
            tool_call_id: None,
            media: None,
        }];

        let (_, converted) = ClaudeAdapter::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "assistant");

        let content = converted[0]["content"].as_array().unwrap();
        // First block: text.
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "I'll look that up.");
        // Second block: tool_use.
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "toolu_01");
        assert_eq!(content[1]["name"], "search");
        // Arguments are a dict, not a JSON string.
        assert!(content[1]["input"].is_object());
    }

    #[test]
    fn convert_messages_tool_result() {
        let messages = vec![Message {
            role: "tool".into(),
            content: Some(r#"{"temp": 20}"#.into()),
            tool_calls: None,
            tool_call_id: Some("toolu_01".into()),
            media: None,
        }];

        let (_, converted) = ClaudeAdapter::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        // Claude expects tool results as a user message.
        assert_eq!(converted[0]["role"], "user");
        let content = converted[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_01");
    }

    #[test]
    fn provider_name_is_claude() {
        let adapter = ClaudeAdapter::new("test-key");
        assert_eq!(adapter.provider_name(), "claude");
    }

    #[test]
    fn user_message_with_image_media_produces_content_blocks() {
        use crate::llm::media::MediaContent;
        let messages = vec![Message {
            role: "user".into(),
            content: Some("Describe this image".into()),
            tool_calls: None,
            tool_call_id: None,
            media: Some(vec![MediaContent {
                mime_type: "image/png".into(),
                data: "dGVzdA==".into(),
                source_path: None,
                file_uri: None,
                pending_upload: false,
            }]),
        }];

        let (_, converted) = ClaudeAdapter::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");

        let content = converted[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        // Claude: image block first, then text.
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[0]["source"]["data"], "dGVzdA==");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "Describe this image");
    }
}
