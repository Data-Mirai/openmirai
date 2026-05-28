//! Gemini adapter — Google AI models via HTTP API.
//!
//! Key differences from OpenAI-compatible providers:
//! - API key is sent as a query parameter (`?key=...`), not a header.
//! - Messages use `contents: [{ parts: [{ text }] }]` format.
//! - Role `"assistant"` maps to `"model"`.
//! - System message goes to `systemInstruction`, NOT in the contents array.
//! - Tool definitions use `functionDeclarations` (no `"function"` wrapper).
//! - Function calls appear as `functionCall` in `parts`.

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

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

// ---------------------------------------------------------------------------
// Adapter struct
// ---------------------------------------------------------------------------

/// Adapter for Google Gemini models.
pub struct GeminiAdapter {
    api_key: String,
    base_url: String,
    #[allow(dead_code)]
    timeout: Duration,
    client: Client,
}

impl GeminiAdapter {
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

    /// Convert our [`Message`] vec into Gemini's `contents` + optional `systemInstruction`.
    ///
    /// Gemini differences:
    /// - `system` messages are extracted into a separate `systemInstruction`.
    /// - role `"assistant"` becomes `"model"`.
    /// - `tool` results become `functionResponse` parts.
    /// - `assistant` tool_calls become `functionCall` parts.
    fn convert_messages(messages: &[Message]) -> (Option<Value>, Vec<Value>) {
        let mut system_parts: Vec<Value> = Vec::new();
        let mut contents: Vec<Value> = Vec::with_capacity(messages.len());

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    if let Some(ref content) = msg.content {
                        system_parts.push(json!({ "text": content }));
                    }
                }
                "assistant" => {
                    let mut parts: Vec<Value> = Vec::new();

                    if let Some(ref content) = msg.content {
                        if !content.is_empty() {
                            parts.push(json!({ "text": content }));
                        }
                    }

                    if let Some(ref tool_calls) = msg.tool_calls {
                        for tc in tool_calls {
                            let args: Value = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(json!({}));
                            parts.push(json!({
                                "functionCall": {
                                    "name": tc.function.name,
                                    "args": args,
                                }
                            }));
                        }
                    }

                    if !parts.is_empty() {
                        contents.push(json!({
                            "role": "model",
                            "parts": parts,
                        }));
                    }
                }
                "tool" => {
                    // Gemini expects function responses as model-role parts.
                    let name = msg.tool_call_id.as_deref().unwrap_or("unknown");
                    let response: Value = msg
                        .content
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| {
                            json!({ "result": msg.content.as_deref().unwrap_or("") })
                        });
                    contents.push(json!({
                        "role": "function",
                        "parts": [{
                            "functionResponse": {
                                "name": name,
                                "response": response,
                            }
                        }],
                    }));
                }
                _ => {
                    // user — pass through, with optional media parts.
                    let mut parts: Vec<Value> = Vec::new();
                    if let Some(ref content) = msg.content {
                        if !content.is_empty() {
                            parts.push(json!({ "text": content }));
                        }
                    }
                    // Append media as inline_data parts (PRD-009).
                    if let Some(ref media_list) = msg.media {
                        for mc in media_list {
                            parts.push(json!({
                                "inline_data": {
                                    "mime_type": mc.mime_type,
                                    "data": mc.data,
                                }
                            }));
                        }
                    }
                    if parts.is_empty() {
                        parts.push(json!({ "text": "" }));
                    }
                    contents.push(json!({
                        "role": "user",
                        "parts": parts,
                    }));
                }
            }
        }

        let system_instruction = if system_parts.is_empty() {
            None
        } else {
            Some(json!({ "parts": system_parts }))
        };

        (system_instruction, contents)
    }

    /// Convert OpenAI-style tool definitions to Gemini format.
    ///
    /// OpenAI: `[{ type: "function", function: { name, description, parameters } }]`
    /// Gemini: `[{ functionDeclarations: [{ name, description, parameters }] }]`
    fn convert_tools(tools: &[Value]) -> Vec<Value> {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                if let Some(func) = tool.get("function") {
                    json!({
                        "name": func.get("name").cloned().unwrap_or(json!("")),
                        "description": func.get("description").cloned().unwrap_or(json!("")),
                        "parameters": func.get("parameters").cloned().unwrap_or(json!({})),
                    })
                } else {
                    // Assume already in Gemini format.
                    tool.clone()
                }
            })
            .collect();

        vec![json!({ "functionDeclarations": declarations })]
    }

    /// Parse function calls from Gemini response parts.
    fn parse_function_calls(parts: &[Value]) -> Vec<ToolCall> {
        parts
            .iter()
            .filter_map(|part| part.get("functionCall"))
            .enumerate()
            .map(|(i, fc)| {
                let name = fc
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = fc.get("args").cloned().unwrap_or(json!({}));
                let arguments = serde_json::to_string(&args).unwrap_or_default();

                ToolCall {
                    id: format!("call_{i}"),
                    name,
                    arguments,
                }
            })
            .collect()
    }

}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMAdapter for GeminiAdapter {
    fn provider_name(&self) -> &str {
        "gemini"
    }

    async fn call(
        &self,
        model: &str,
        prompt: &str,
        context: Option<&str>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<NormalizedResponse, LLMError> {
        let mut payload = json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": max_tokens,
            },
        });

        if let Some(ctx) = context {
            payload["systemInstruction"] = json!({
                "parts": [{ "text": ctx }]
            });
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
        );

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

        let text = data
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let usage = data.get("usageMetadata").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: text,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: model.to_string(),
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
        let (system_instruction, contents) = Self::convert_messages(&messages);

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": max_tokens,
            },
        });

        if let Some(sys) = system_instruction {
            payload["systemInstruction"] = sys;
        }

        if let Some(tool_defs) = tools {
            let gemini_tools = Self::convert_tools(&tool_defs);
            payload["tools"] = Value::Array(gemini_tools);
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
        );

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

        let parts = data
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        // Extract text from text parts.
        let text: String = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");

        let tool_calls = Self::parse_function_calls(&parts);

        let usage = data.get("usageMetadata").cloned().unwrap_or(json!({}));
        let tokens_input = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let tokens_output = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        Ok(NormalizedResponse {
            response: text,
            tokens_used: TokenUsage {
                input: tokens_input,
                output: tokens_output,
            },
            model: model.to_string(),
            provider: self.provider_name().to_string(),
            tool_calls,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models?key={}", self.base_url, self.api_key);

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

        let models: Vec<ModelInfo> = models_array
            .iter()
            .map(|m| {
                let id = m
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let display_name = m
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let ctx = m
                    .get("inputTokenLimit")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);

                ModelInfo {
                    id: id.clone(),
                    name: display_name,
                    context_window: ctx,
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
    fn content_format_uses_parts_array() {
        let messages = vec![Message {
            role: "user".into(),
            content: Some("Hello world".into()),
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");

        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "Hello world");
    }

    #[test]
    fn system_instruction_separate_from_contents() {
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
                content: Some("Hi".into()),
                tool_calls: None,
                tool_call_id: None,
                media: None,
            },
        ];

        let (system_instruction, contents) = GeminiAdapter::convert_messages(&messages);

        // System goes to systemInstruction, not contents.
        assert!(system_instruction.is_some());
        let sys = system_instruction.unwrap();
        let sys_parts = sys["parts"].as_array().unwrap();
        assert_eq!(sys_parts[0]["text"], "You are a helpful assistant.");

        // Only the user message in contents.
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn role_mapping_assistant_to_model() {
        let messages = vec![
            Message {
                role: "user".into(),
                content: Some("Hello".into()),
                tool_calls: None,
                tool_call_id: None,
                media: None,
            },
            Message {
                role: "assistant".into(),
                content: Some("Hi there!".into()),
                tool_calls: None,
                tool_call_id: None,
                media: None,
            },
        ];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"], "user");
        // "assistant" is mapped to "model" for Gemini.
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "Hi there!");
    }

    #[test]
    fn function_call_parsing() {
        let parts = vec![
            json!({ "text": "Checking the weather." }),
            json!({
                "functionCall": {
                    "name": "get_weather",
                    "args": { "city": "London" }
                }
            }),
        ];

        let tool_calls = GeminiAdapter::parse_function_calls(&parts);
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "get_weather");
        assert_eq!(tool_calls[0].id, "call_0");

        let args: Value = serde_json::from_str(&tool_calls[0].arguments).unwrap();
        assert_eq!(args["city"], "London");
    }

    #[test]
    fn tool_conversion_to_function_declarations() {
        let openai_tools = vec![json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "Search the web",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } }
                }
            }
        })];

        let gemini_tools = GeminiAdapter::convert_tools(&openai_tools);
        assert_eq!(gemini_tools.len(), 1);

        let declarations = gemini_tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0]["name"], "search");
        assert_eq!(declarations[0]["description"], "Search the web");
        assert!(declarations[0].get("parameters").is_some());
    }

    #[test]
    fn convert_messages_assistant_with_tool_calls() {
        let messages = vec![Message {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCallRequest {
                id: "call_0".into(),
                function: FunctionCall {
                    name: "get_weather".into(),
                    arguments: r#"{"city":"Paris"}"#.into(),
                },
            }]),
            tool_call_id: None,
            media: None,
        }];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "model");

        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert!(parts[0].get("functionCall").is_some());
        assert_eq!(parts[0]["functionCall"]["name"], "get_weather");
        assert_eq!(parts[0]["functionCall"]["args"]["city"], "Paris");
    }

    #[test]
    fn provider_name_is_gemini() {
        let adapter = GeminiAdapter::new("test-key");
        assert_eq!(adapter.provider_name(), "gemini");
    }
}
