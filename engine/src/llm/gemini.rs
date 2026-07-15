//! Gemini adapter — Google AI models via HTTP API.
//!
//! Key differences from OpenAI-compatible providers:
//! - API key is sent via the `x-goog-api-key` header (never in the URL,
//!   so it can't leak through logged/stringified request URLs).
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

/// Client timeout. Generous on purpose (PRD-018): a 2h-meeting transcription
/// generates 25-40k output tokens ≈ 2-4 minutes of generation, plus large
/// uploads. 60s used to kill any long generation mid-flight.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Max wall-clock time waiting for an uploaded file to become ACTIVE.
const FILE_ACTIVE_POLL_MAX_SECS: u64 = 300;
/// Delay between ACTIVE polls.
const FILE_ACTIVE_POLL_INTERVAL_SECS: u64 = 2;

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
        Self::with_options(
            api_key,
            DEFAULT_BASE_URL,
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
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
                            let args: Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
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
                        .unwrap_or_else(
                            || json!({ "result": msg.content.as_deref().unwrap_or("") }),
                        );
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
                    // Append media parts: by reference (Files API, PRD-018)
                    // when uploaded, inline base64 (PRD-009) otherwise.
                    if let Some(ref media_list) = msg.media {
                        for mc in media_list {
                            if let Some(ref uri) = mc.file_uri {
                                parts.push(json!({
                                    "file_data": {
                                        "mime_type": mc.mime_type,
                                        "file_uri": uri,
                                    }
                                }));
                            } else {
                                parts.push(json!({
                                    "inline_data": {
                                        "mime_type": mc.mime_type,
                                        "data": mc.data,
                                    }
                                }));
                            }
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

    // ------------------------------------------------------------------
    // PRD-018: Files API — large media travels by reference
    // ------------------------------------------------------------------

    /// Reject a response whose generation was cut before a clean stop.
    ///
    /// PRD-018 (fail-loud): partial text must never surface as success. A
    /// 2h-meeting transcript that silently covers only the first 25 minutes
    /// is worse than an explicit error — the caller can't tell it's broken.
    ///
    /// Whitelist: only `"STOP"` or an absent finishReason (tool calls,
    /// streaming chunks) count as success. Anything else — MAX_TOKENS,
    /// SAFETY, RECITATION, BLOCKLIST, OTHER, … — cut the output and must
    /// surface as an error, not as a silent partial success.
    fn check_finish_reason(data: &Value) -> Result<(), LLMError> {
        let reason = data
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            .unwrap_or("");
        match reason {
            "" | "STOP" => Ok(()),
            "MAX_TOKENS" => {
                let emitted = data
                    .pointer("/usageMetadata/candidatesTokenCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                Err(LLMError::Truncated(format!(
                    "finishReason=MAX_TOKENS after {emitted} output tokens — raise max_tokens to the model's output ceiling or shrink the requested output"
                )))
            }
            other => Err(LLMError::Truncated(format!(
                "finishReason={other} — the model stopped before completing the output; any returned text is partial"
            ))),
        }
    }

    /// Upload a local file to the Gemini Files API (resumable, single shot)
    /// and wait until it is ACTIVE. Returns `(file_uri, file_name)` — the
    /// `file_name` (`files/abc123`) is needed to delete it afterwards.
    async fn upload_file(&self, path: &str, mime_type: &str) -> Result<(String, String), LLMError> {
        let metadata = tokio::fs::metadata(path).await.map_err(|e| {
            LLMError::ConnectionError(format!("media file unreadable for upload '{path}': {e}"))
        })?;
        let size = metadata.len();
        let display_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("media");

        // 1. Start resumable session. The upload root strips the API-version
        //    suffix from base_url (…/v1beta → …/upload/v1beta).
        let upload_root = self.base_url.trim_end_matches("/v1beta");
        let start_url = format!("{upload_root}/upload/v1beta/files");
        let start_resp = self
            .client
            .post(&start_url)
            .header("x-goog-api-key", &self.api_key)
            .header("X-Goog-Upload-Protocol", "resumable")
            .header("X-Goog-Upload-Command", "start")
            .header("X-Goog-Upload-Header-Content-Length", size.to_string())
            .header("X-Goog-Upload-Header-Content-Type", mime_type)
            .json(&json!({ "file": { "display_name": display_name } }))
            .send()
            .await
            .map_err(super::error::map_reqwest_error)?;

        let status = start_resp.status();
        if !status.is_success() {
            let body = start_resp.text().await.unwrap_or_default();
            return Err(LLMError::RequestFailed {
                status: status.as_u16(),
                body: format!("Files API upload start failed: {body}"),
            });
        }
        let session_url = start_resp
            .headers()
            .get("x-goog-upload-url")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                LLMError::ParseError("Files API: missing x-goog-upload-url header".into())
            })?
            .to_string();

        // 2. Send the bytes and finalize. One shot — resumable chunking is
        //    unnecessary below the 2GB cap on a local connection. The body is
        //    streamed from disk so a large file never sits fully in RAM.
        let file = tokio::fs::File::open(path).await.map_err(|e| {
            LLMError::ConnectionError(format!("failed to read media file '{path}': {e}"))
        })?;
        let upload_resp = self
            .client
            .post(&session_url)
            // The session URL no longer embeds ?key= (it mirrored the start
            // request), so authenticate the finalize explicitly too.
            .header("x-goog-api-key", &self.api_key)
            .header("X-Goog-Upload-Command", "upload, finalize")
            .header("X-Goog-Upload-Offset", "0")
            // Explicit Content-Length is required here: with a streamed body
            // reqwest can't derive it and would fall back to chunked
            // transfer, which the upload endpoint may reject.
            .header("Content-Length", size.to_string())
            .body(reqwest::Body::from(file))
            .send()
            .await
            .map_err(super::error::map_reqwest_error)?;

        let status = upload_resp.status();
        if !status.is_success() {
            let body = upload_resp.text().await.unwrap_or_default();
            return Err(LLMError::RequestFailed {
                status: status.as_u16(),
                body: format!("Files API upload failed: {body}"),
            });
        }
        let file_info: Value = upload_resp
            .json()
            .await
            .map_err(|e| LLMError::ParseError(format!("Files API upload response: {e}")))?;

        let file_uri = file_info
            .pointer("/file/uri")
            .and_then(Value::as_str)
            .ok_or_else(|| LLMError::ParseError("Files API: response missing file.uri".into()))?
            .to_string();
        let file_name = file_info
            .pointer("/file/name")
            .and_then(Value::as_str)
            .ok_or_else(|| LLMError::ParseError("Files API: response missing file.name".into()))?
            .to_string();
        let mut state = file_info
            .pointer("/file/state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 3. Poll until ACTIVE (the API processes uploads asynchronously).
        let poll_url = format!("{}/{}", self.base_url, file_name);
        let deadline = std::time::Instant::now() + Duration::from_secs(FILE_ACTIVE_POLL_MAX_SECS);
        while state != "ACTIVE" {
            if state == "FAILED" {
                return Err(LLMError::RequestFailed {
                    status: 0,
                    body: format!("Files API: uploaded file {file_name} entered FAILED state"),
                });
            }
            if std::time::Instant::now() >= deadline {
                return Err(LLMError::Timeout);
            }
            tokio::time::sleep(Duration::from_secs(FILE_ACTIVE_POLL_INTERVAL_SECS)).await;
            let poll: Value = self
                .client
                .get(&poll_url)
                .header("x-goog-api-key", &self.api_key)
                .send()
                .await
                .map_err(super::error::map_reqwest_error)?
                .json()
                .await
                .map_err(|e| LLMError::ParseError(format!("Files API poll: {e}")))?;
            state = poll
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
        }

        Ok((file_uri, file_name))
    }

    /// Delete an uploaded file (best-effort — the API auto-expires files
    /// after 48h, so a failed delete only widens the retention window).
    async fn delete_remote_file(&self, file_name: &str) {
        let url = format!("{}/{}", self.base_url, file_name);
        let _ = self
            .client
            .delete(&url)
            .header("x-goog-api-key", &self.api_key)
            .send()
            .await;
    }

    /// Upload every `pending_upload` media attachment in `messages`, swapping
    /// it for its `file_uri`. Returns the remote file names for post-request
    /// cleanup.
    async fn upload_pending_media(
        &self,
        messages: &mut [Message],
    ) -> Result<Vec<String>, LLMError> {
        let mut uploaded: Vec<String> = Vec::new();
        for msg in messages.iter_mut() {
            let Some(ref mut media_list) = msg.media else {
                continue;
            };
            for mc in media_list.iter_mut() {
                if !mc.pending_upload || mc.file_uri.is_some() {
                    continue;
                }
                let path = mc.source_path.clone().ok_or_else(|| {
                    LLMError::ParseError(
                        "media marked pending_upload but has no source_path".into(),
                    )
                })?;
                let result = self.upload_file(&path, &mc.mime_type).await;
                match result {
                    Ok((uri, name)) => {
                        mc.file_uri = Some(uri);
                        mc.pending_upload = false;
                        uploaded.push(name);
                    }
                    Err(e) => {
                        // Clean up files already uploaded for this request.
                        for name in &uploaded {
                            self.delete_remote_file(name).await;
                        }
                        return Err(e);
                    }
                }
            }
        }
        Ok(uploaded)
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
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError> {
        let mut payload = json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "temperature": temperature,
            },
        });

        if let Some(max) = max_tokens {
            payload["generationConfig"]["maxOutputTokens"] = json!(max);
        }

        if let Some(ctx) = context {
            payload["systemInstruction"] = json!({
                "parts": [{ "text": ctx }]
            });
        }

        let url = format!("{}/models/{}:generateContent", self.base_url, model);

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
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

        // PRD-018: a token-limit cut is an error, never a partial success.
        Self::check_finish_reason(&data)?;

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
        mut messages: Vec<Message>,
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError> {
        // PRD-018: large media goes up via Files API first; the remote copies
        // are deleted after the request whether it succeeds or fails.
        let uploaded = self.upload_pending_media(&mut messages).await?;
        let result = self
            .generate_with_messages(model, &messages, tools, temperature, max_tokens)
            .await;
        for name in &uploaded {
            self.delete_remote_file(name).await;
        }
        result
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.base_url);

        let resp = self
            .client
            .get(&url)
            .header("x-goog-api-key", &self.api_key)
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

impl GeminiAdapter {
    /// The generateContent request itself (post media upload). Split out so
    /// `call_with_messages` can guarantee remote-file cleanup around it.
    async fn generate_with_messages(
        &self,
        model: &str,
        messages: &[Message],
        tools: Option<Vec<Value>>,
        temperature: f32,
        max_tokens: Option<u32>,
    ) -> Result<NormalizedResponse, LLMError> {
        let (system_instruction, contents) = Self::convert_messages(messages);

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": temperature,
            },
        });

        if let Some(max) = max_tokens {
            payload["generationConfig"]["maxOutputTokens"] = json!(max);
        }

        if let Some(sys) = system_instruction {
            payload["systemInstruction"] = sys;
        }

        if let Some(tool_defs) = tools {
            let gemini_tools = Self::convert_tools(&tool_defs);
            payload["tools"] = Value::Array(gemini_tools);
        }

        let url = format!("{}/models/{}:generateContent", self.base_url, model);

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
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

        // PRD-018: a token-limit cut is an error, never a partial success.
        Self::check_finish_reason(&data)?;

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

    #[test]
    fn user_message_with_media_produces_inline_data_parts() {
        use crate::llm::media::MediaContent;
        let messages = vec![Message {
            role: "user".into(),
            content: Some("Transcribe this audio".into()),
            tool_calls: None,
            tool_call_id: None,
            media: Some(vec![MediaContent {
                mime_type: "audio/mp4".into(),
                data: "dGVzdA==".into(), // "test" in base64
                source_path: Some("/tmp/test.m4a".into()),
                file_uri: None,
                pending_upload: false,
            }]),
        }];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");

        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "Transcribe this audio");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "audio/mp4");
        assert_eq!(parts[1]["inline_data"]["data"], "dGVzdA==");
    }

    // --- PRD-018: by-reference media + truncation fail-loud ---

    #[test]
    fn user_message_with_uploaded_media_produces_file_data_part() {
        use crate::llm::media::MediaContent;
        let messages = vec![Message {
            role: "user".into(),
            content: Some("Transcribe this audio".into()),
            tool_calls: None,
            tool_call_id: None,
            media: Some(vec![MediaContent {
                mime_type: "audio/mp4".into(),
                data: String::new(),
                source_path: Some("/tmp/big.m4a".into()),
                file_uri: Some("https://generativelanguage.googleapis.com/v1beta/files/abc".into()),
                pending_upload: false,
            }]),
        }];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["file_data"]["mime_type"], "audio/mp4");
        assert_eq!(
            parts[1]["file_data"]["file_uri"],
            "https://generativelanguage.googleapis.com/v1beta/files/abc"
        );
        assert!(parts[1].get("inline_data").is_none());
    }

    #[test]
    fn finish_reason_max_tokens_is_an_error() {
        let data = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "partial transcri" }] },
                "finishReason": "MAX_TOKENS"
            }],
            "usageMetadata": { "candidatesTokenCount": 4096 }
        });
        let err = GeminiAdapter::check_finish_reason(&data).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("truncated"), "got: {msg}");
        assert!(msg.contains("4096"), "got: {msg}");
    }

    #[test]
    fn finish_reason_stop_is_ok() {
        let data = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "full text" }] },
                "finishReason": "STOP"
            }]
        });
        assert!(GeminiAdapter::check_finish_reason(&data).is_ok());
    }

    #[test]
    fn finish_reason_absent_is_ok() {
        // Some responses (tool calls, streaming chunks) omit finishReason.
        let data = json!({ "candidates": [{ "content": { "parts": [] } }] });
        assert!(GeminiAdapter::check_finish_reason(&data).is_ok());
    }

    #[test]
    fn user_message_without_media_is_unchanged() {
        let messages = vec![Message {
            role: "user".into(),
            content: Some("Hello".into()),
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }];

        let (_, contents) = GeminiAdapter::convert_messages(&messages);
        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "Hello");
        assert!(parts[0].get("inline_data").is_none());
    }
}
