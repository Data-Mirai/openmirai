use std::collections::HashMap;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use regex::Regex;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType};
use crate::tools::registry::{Tool, ToolRegistry};


// ===========================================================================
// LlmCallTool
// ===========================================================================

ai_tool! {
    struct LlmCallTool, factory LlmCallFactory;
    tool_type = "ai/llm_call",
    name = "LLM Call",
    description = "Processes session data through an LLM to produce grounded responses",
    inputs = [
        field("prompt", FieldType::String, false, "Instruction for the LLM on how to process the data"),
    ],
    outputs = [
        field("response", FieldType::String, true, "Raw LLM text response"),
        field("model", FieldType::String, true, "Model used"),
        field("tokens_input", FieldType::Number, true, "Input tokens used"),
        field("tokens_output", FieldType::Number, true, "Output tokens used"),
        field("structured_output", FieldType::Object, false, "Parsed JSON when output_schema is defined"),
        field("schema_valid", FieldType::Boolean, false, "Whether response matched output_schema"),
    ],
    config_fields = [
        field("model", FieldType::String, false, "LLM model identifier"),
        field("temperature", FieldType::Number, false, "Sampling temperature"),
        field("max_tokens", FieldType::Number, false, "Max tokens to generate"),
        field("system_prompt", FieldType::String, false, "Node-level system prompt"),
        field("output_schema", FieldType::String, false, "JSON Schema to enforce structured output"),
        field("output_schema_strict", FieldType::Boolean, false, "Fail hard if schema validation fails after retries (default true)"),
        field("max_retries", FieldType::Number, false, "Retries for schema validation (default 2)"),
        field("max_context_length", FieldType::Number, false, "Max chars for session context (default 12000)"),
        field("media_path", FieldType::String, false, "Path to media file (image/audio/video) to send alongside the prompt"),
    ]
}

#[async_trait]
impl Tool for LlmCallTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // --- Resolve prompt ---
        let prompt = inputs
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_default();

        if prompt.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/llm_call".into(),
                message: "prompt is required (via input or config)".into(),
            });
        }

        // --- Security: prompt injection scan ---
        let scanner_enabled = config
            .get("security_scan")
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // Enabled by default

        if scanner_enabled {
            let sensitivity = match config.get("security_sensitivity").and_then(|v| v.as_str()) {
                Some("low") => crate::security::Sensitivity::Low,
                Some("high") => crate::security::Sensitivity::High,
                _ => crate::security::Sensitivity::Medium,
            };
            let scan_config = crate::security::ScannerConfig {
                enabled: true,
                sensitivity,
                block_on_detection: config
                    .get("security_block")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            };

            // Scan all text inputs recursively (prompt + session data).
            let mut text_parts: Vec<String> = Vec::new();
            fn collect_strings(value: &Value, parts: &mut Vec<String>) {
                match value {
                    Value::String(s) => parts.push(s.clone()),
                    Value::Array(arr) => arr.iter().for_each(|v| collect_strings(v, parts)),
                    Value::Object(map) => map.values().for_each(|v| collect_strings(v, parts)),
                    _ => {}
                }
            }
            for v in inputs.values() {
                collect_strings(v, &mut text_parts);
            }
            let all_input_text = text_parts.join(" ");

            let scan_result = crate::security::scan(&all_input_text, &scan_config);
            if scan_result.blocked {
                warn!(
                    threat = ?scan_result.threat_type,
                    confidence = scan_result.confidence,
                    pattern = ?scan_result.matched_pattern,
                    "Prompt injection detected — blocking execution"
                );
                return Err(ToolError::ExecutionFailed {
                    tool_type: "ai/llm_call".into(),
                    message: format!(
                        "Security scan blocked execution: {:?} detected (confidence: {:.0}%). {}",
                        scan_result.threat_type,
                        scan_result.confidence * 100.0,
                        scan_result.details.unwrap_or_default()
                    ),
                });
            }
        }

        // --- Build session context from all non-prompt inputs ---
        let session_data: HashMap<String, Value> = inputs
            .iter()
            .filter(|(k, _)| k.as_str() != "prompt")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let max_ctx_len = config
            .get("max_context_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(12000) as usize;

        let session_context = format_session_context(&session_data, max_ctx_len);

        // --- Parse output_schema if defined ---
        let output_schema = parse_output_schema(config.get("output_schema"));
        let max_retries = if output_schema.is_some() {
            config
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(2) as usize
        } else {
            0
        };

        // --- If output_schema, enrich prompt with JSON format instructions ---
        let effective_prompt = match &output_schema {
            Some(schema) => enrich_prompt_with_schema(&prompt, schema),
            None => prompt.clone(),
        };

        // --- Build system prompt (agent-level + node-level) ---
        let mut system_parts: Vec<String> = Vec::new();
        if let Some(sp) = context.system_prompt() {
            system_parts.push(sp.to_string());
        }
        if let Some(node_sp) = config.get("system_prompt").and_then(|v| v.as_str()) {
            if !node_sp.is_empty() {
                system_parts.push(node_sp.to_string());
            }
        }

        // Combine system_prompt + session_context
        let combined_context = if system_parts.is_empty() {
            session_context
        } else {
            format!("{}\n\n{}", system_parts.join("\n\n"), session_context)
        };

        // --- Prepare LLM call params ---
        let model = config.get("model").and_then(|v| v.as_str()).unwrap_or("");
        let temperature = config
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);
        let max_tokens = config
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        // Build context as proper message objects for the LLM adapter.
        let mut context_messages = Vec::new();
        if !combined_context.is_empty() {
            context_messages.push(json!({"role": "system", "content": combined_context}));
        }

        // PRD-009/010: resolve media_path (inputs > config). Accepts String or FileRef.
        let media_path_raw = inputs
            .get("media_path")
            .or_else(|| config.get("media_path"));
        let media_path = match media_path_raw {
            Some(v) => crate::llm::media::resolve_file_input(v),
            None => String::new(),
        };
        if !media_path.is_empty() {
            let provider_name = context.llm().provider_name();
            let media =
                crate::llm::media::read_media_file(&media_path, provider_name).map_err(|e| {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/llm_call".into(),
                        message: e,
                    }
                })?;
            info!(
                media_path = %media_path,
                mime_type = %media.mime_type,
                provider = %provider_name,
                "llm_call: attaching media file to prompt"
            );
            context_messages.push(crate::llm::media::user_media_entry(&media));
        }

        // --- Execute with optional schema validation + retries ---
        let mut current_prompt = effective_prompt.clone();
        let mut last_response = String::new();
        let mut last_tokens_input = 0u32;
        let mut last_tokens_output = 0u32;

        for attempt in 0..=max_retries {
            let result = context
                .llm()
                .call(
                    model,
                    &current_prompt,
                    &context_messages,
                    temperature,
                    max_tokens,
                )
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/llm_call".into(),
                    message: e.to_string(),
                })?;

            last_response = result.response;
            last_tokens_input = result.tokens_used.input;
            last_tokens_output = result.tokens_used.output;
            let used_model = result.model;

            if output_schema.is_none() {
                // No schema -- return raw response
                let mut out = HashMap::new();
                out.insert("response".to_string(), json!(last_response));
                out.insert("model".to_string(), json!(used_model));
                out.insert("tokens_input".to_string(), json!(last_tokens_input));
                out.insert("tokens_output".to_string(), json!(last_tokens_output));
                out.insert("structured_output".to_string(), Value::Null);
                out.insert("schema_valid".to_string(), json!(false));
                return Ok(out);
            }

            // --- Validate against schema ---
            let schema = output_schema.as_ref().unwrap();
            let (parsed, errors) = validate_response(&last_response, schema);

            if let Some(parsed_value) = &parsed {
                if errors.is_empty() {
                    let mut out = HashMap::new();
                    out.insert("response".to_string(), json!(last_response));
                    out.insert("model".to_string(), json!(used_model));
                    out.insert("tokens_input".to_string(), json!(last_tokens_input));
                    out.insert("tokens_output".to_string(), json!(last_tokens_output));
                    out.insert("structured_output".to_string(), parsed_value.clone());
                    out.insert("schema_valid".to_string(), json!(true));
                    return Ok(out);
                }
            }

            // --- Retry with error feedback ---
            if attempt < max_retries {
                warn!(
                    attempt = attempt + 1,
                    total = max_retries + 1,
                    errors = ?errors,
                    "output_schema validation failed, retrying"
                );
                current_prompt = build_retry_prompt(&effective_prompt, &last_response, &errors);
            }
        }

        // --- All retries exhausted ---
        let schema = output_schema.as_ref().unwrap();
        let (parsed, final_errors) = validate_response(&last_response, schema);
        let schema_valid = parsed.is_some() && final_errors.is_empty();

        // If strict mode is enabled, fail hard when schema validation fails.
        let strict = config
            .get("output_schema_strict")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if strict && !schema_valid {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/llm_call".into(),
                message: format!(
                    "Output validation failed after {} retries. Errors: {}",
                    max_retries,
                    final_errors.join("; ")
                ),
            });
        }

        // Non-strict: return best effort.
        let mut out = HashMap::new();
        out.insert("response".to_string(), json!(last_response));
        out.insert("model".to_string(), json!(model));
        out.insert("tokens_input".to_string(), json!(last_tokens_input));
        out.insert("tokens_output".to_string(), json!(last_tokens_output));
        out.insert(
            "structured_output".to_string(),
            parsed.unwrap_or(Value::Null),
        );
        out.insert("schema_valid".to_string(), json!(schema_valid));
        Ok(out)
    }
}

// ===========================================================================
// EmbeddingsTool
// ===========================================================================

ai_tool! {
    struct EmbeddingsTool, factory EmbeddingsFactory;
    tool_type = "ai/embeddings",
    name = "Generate Embeddings",
    description = "Generates vector embedding from text input",
    inputs = [
        field("text", FieldType::String, true, "Text to generate embedding for"),
    ],
    outputs = [
        field("embedding", FieldType::Array, true, "Embedding vector"),
        field("dimensions", FieldType::Number, true, "Vector dimension count"),
    ],
    config_fields = [
        field("model", FieldType::String, false, "Embedding model to use"),
    ]
}

#[async_trait]
impl Tool for EmbeddingsTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let text = inputs.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "ai/embeddings".into(),
                message: "input 'text' is required".into(),
            }
        })?;

        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let embedding =
            context
                .llm()
                .embed(text, model)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/embeddings".into(),
                    message: e.to_string(),
                })?;

        let dimensions = embedding.len();

        let mut out = HashMap::new();
        out.insert("embedding".to_string(), json!(embedding));
        out.insert("dimensions".to_string(), json!(dimensions));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Session context formatting
// ---------------------------------------------------------------------------

/// Format session data into structured context for the LLM.
///
/// If `max_length > 0` and total exceeds it, truncates values proportionally
/// -- largest values lose the most, each keeps at least 200 chars.
fn format_session_context(data: &HashMap<String, Value>, max_length: usize) -> String {
    if data.is_empty() {
        return "=== Session Context ===\n(no data)\n=== End Session Context ===".to_string();
    }

    let mut entries: Vec<(String, String)> = data
        .iter()
        .map(|(key, value)| {
            let formatted = match value {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
            };
            (key.clone(), formatted)
        })
        .collect();

    if max_length > 0 {
        // Overhead: wrapper lines + key labels + separators
        let overhead: usize = 60
            + entries.iter().map(|(k, _)| k.len() + 6).sum::<usize>()
            + entries.len().saturating_sub(1) * 2;

        let content_budget = max_length.saturating_sub(overhead);
        let total_content: usize = entries.iter().map(|(_, v)| v.len()).sum();

        if total_content > content_budget && content_budget > 0 {
            let ratio = content_budget as f64 / total_content as f64;
            entries = entries
                .into_iter()
                .map(|(key, value)| {
                    let cap = (value.len() as f64 * ratio).max(200.0) as usize;
                    if value.len() > cap {
                        info!(
                            key = %key,
                            from = value.len(),
                            to = cap,
                            "truncating session context key"
                        );
                        let truncated = format!(
                            "{}\n[... truncated {} -> {} chars]",
                            &value[..cap],
                            value.len(),
                            cap
                        );
                        (key, truncated)
                    } else {
                        (key, value)
                    }
                })
                .collect();
        }
    }

    let parts: Vec<String> = entries
        .iter()
        .map(|(key, value)| format!("[{}]:\n{}", key, value))
        .collect();

    format!(
        "=== Session Context ===\n{}\n=== End Session Context ===",
        parts.join("\n\n")
    )
}

// ---------------------------------------------------------------------------
// Output schema helpers
// ---------------------------------------------------------------------------

/// Parse output_schema from config value (JSON string or object). Returns None if empty.
fn parse_output_schema(raw: Option<&Value>) -> Option<Value> {
    match raw {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                match serde_json::from_str::<Value>(s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        warn!(error = %e, "output_schema is not valid JSON, ignoring");
                        None
                    }
                }
            }
        }
        Some(v @ Value::Object(_)) => Some(v.clone()),
        _ => None,
    }
}

/// Prepend/append JSON schema instructions to the prompt.
fn enrich_prompt_with_schema(prompt: &str, schema: &Value) -> String {
    let schema_str = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    format!(
        "{}\n\n\
         === OUTPUT FORMAT (MANDATORY) ===\n\
         Respond ONLY with a valid JSON object matching this schema. \
         No markdown, no explanation, no text before or after the JSON.\n\n\
         {}\n\n\
         === END OUTPUT FORMAT ===",
        prompt, schema_str
    )
}

/// Extract JSON from LLM response, handling markdown code blocks.
fn extract_json_from_response(text: &str) -> Option<String> {
    let text = text.trim();

    // Try direct parse first
    if text.starts_with('{') || text.starts_with('[') {
        return Some(text.to_string());
    }

    // Extract from ```json ... ``` blocks
    let code_block_re = Regex::new(r"```(?:json)?\s*\n?([\s\S]*?)\n?\s*```").expect("valid regex");
    if let Some(caps) = code_block_re.captures(text) {
        return Some(caps[1].trim().to_string());
    }

    // Find first balanced { ... } or [ ... ] block
    for (opener, closer) in [('{', '}'), ('[', ']')] {
        if let Some(start) = text.find(opener) {
            let mut depth = 0i32;
            let bytes = text.as_bytes();
            for i in start..bytes.len() {
                if bytes[i] == opener as u8 {
                    depth += 1;
                } else if bytes[i] == closer as u8 {
                    depth -= 1;
                }
                if depth == 0 {
                    return Some(text[start..=i].to_string());
                }
            }
        }
    }

    None
}

/// Validate a response against a JSON schema.
/// Returns (parsed_value, errors). If errors is empty, validation passed.
fn validate_response(response: &str, schema: &Value) -> (Option<Value>, Vec<String>) {
    let mut errors: Vec<String> = Vec::new();

    let json_str = match extract_json_from_response(response) {
        Some(s) => s,
        None => return (None, vec!["Response does not contain valid JSON".into()]),
    };

    let parsed: Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => return (None, vec![format!("JSON parse error: {}", e)]),
    };

    if !parsed.is_object() {
        let type_name = match &parsed {
            Value::Array(_) => "array",
            Value::String(_) => "string",
            Value::Number(_) => "number",
            Value::Bool(_) => "boolean",
            Value::Null => "null",
            _ => "unknown",
        };
        return (
            Some(parsed),
            vec![format!("Expected JSON object, got {}", type_name)],
        );
    }

    let obj = parsed.as_object().unwrap();

    // Validate required fields
    if let Some(Value::Array(required)) = schema.get("required") {
        for req in required {
            if let Some(field_name) = req.as_str() {
                if !obj.contains_key(field_name) {
                    errors.push(format!("Missing required field: '{}'", field_name));
                }
            }
        }
    }

    // Validate types for present fields
    if let Some(Value::Object(properties)) = schema.get("properties") {
        for (field_name, field_schema) in properties {
            if let Some(value) = obj.get(field_name) {
                // Check type
                if let Some(expected_type) = field_schema.get("type").and_then(|t| t.as_str()) {
                    if !check_json_type(value, expected_type) {
                        let actual_type = value_type_label(value);
                        errors.push(format!(
                            "Field '{}' expected type '{}', got '{}'",
                            field_name, expected_type, actual_type
                        ));
                    }
                }

                // Check enum
                if let Some(Value::Array(enum_values)) = field_schema.get("enum") {
                    if !enum_values.contains(value) {
                        errors.push(format!(
                            "Field '{}' must be one of {:?}, got {:?}",
                            field_name, enum_values, value
                        ));
                    }
                }
            }
        }
    }

    (Some(parsed), errors)
}

/// Check if a JSON value matches an expected JSON Schema type.
fn check_json_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true, // Unknown type, skip validation
    }
}

use crate::core::value_type::value_type_label;

/// Build a retry prompt with error feedback.
fn build_retry_prompt(original_prompt: &str, bad_response: &str, errors: &[String]) -> String {
    let error_list: String = errors
        .iter()
        .map(|e| format!("- {}", e))
        .collect::<Vec<_>>()
        .join("\n");
    let truncated_response = if bad_response.len() > 500 {
        &bad_response[..500]
    } else {
        bad_response
    };
    format!(
        "{}\n\n\
         === RETRY -- PREVIOUS RESPONSE WAS INVALID ===\n\
         Your previous response had these errors:\n{}\n\n\
         Previous response (DO NOT repeat this):\n{}\n\n\
         Fix these errors and respond ONLY with valid JSON matching the schema.\n\
         === END RETRY ===",
        original_prompt, error_list, truncated_response
    )
}

// ===========================================================================
// TranscribeTool
// ===========================================================================

ai_tool! {
    struct TranscribeTool, factory TranscribeFactory;
    tool_type = "ai/transcribe",
    name = "Transcribe Audio",
    description = "Transcribes audio to text. Routes by provider: 'elevenlabs' calls the Scribe STT API; any other value uses the CLI's configured LLM provider (e.g. Gemini) as multimodal transcriber.",
    inputs = [
        field("file_path", FieldType::String, true, "Path to the audio file"),
    ],
    outputs = [
        field("text", FieldType::String, true, "Transcription text"),
        field("duration_seconds", FieldType::Number, true, "Audio duration in seconds (0 when unknown)"),
        field("provider", FieldType::String, true, "Provider used (elevenlabs | LLM provider name)"),
    ],
    config_fields = [
        field("provider", FieldType::String, false, "STT provider: 'elevenlabs' (Scribe API) or anything else for the LLM multimodal route (default)"),
        field("model", FieldType::String, false, "Model id (LLM route: LLM model; elevenlabs route: default scribe_v1)"),
        field("api_key", FieldType::String, false, "ElevenLabs API key (falls back to ELEVENLABS_API_KEY env var)"),
        field("language", FieldType::String, false, "ISO language code hint, e.g. es (elevenlabs language_code; omit for auto-detect)"),
        field("base_url", FieldType::String, false, "Override API base URL (elevenlabs route)"),
    ]
}

const SCRIBE_DEFAULT_MODEL: &str = "scribe_v1";

// --- Pure request builders for the ElevenLabs Scribe route (unit-tested without network) ---
fn eleven_stt_url(base_url: &str) -> String {
    format!("{base_url}/v1/speech-to-text")
}
fn eleven_stt_text_fields(model: &str, language: Option<&str>) -> Vec<(String, String)> {
    // tag_audio_events off: conversational STT, no "(laughter)"-style markers in the text.
    let mut fields = vec![
        ("model_id".to_string(), model.to_string()),
        ("tag_audio_events".to_string(), "false".to_string()),
    ];
    if let Some(lang) = language {
        if !lang.is_empty() {
            fields.push(("language_code".to_string(), lang.to_string()));
        }
    }
    fields
}
fn audio_mime_for_path(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}
/// Scribe returns word-level timestamps by default; duration = end of the last word.
fn scribe_duration_seconds(resp: &Value) -> f64 {
    resp.get("words")
        .and_then(|w| w.as_array())
        .and_then(|a| a.last())
        .and_then(|w| w.get("end"))
        .and_then(|e| e.as_f64())
        .unwrap_or(0.0)
}

#[async_trait]
impl Tool for TranscribeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // PRD-010: resolve file_path — accepts both String paths and FileRef objects.
        let file_path_value =
            inputs
                .get("file_path")
                .ok_or_else(|| ToolError::ExecutionFailed {
                    tool_type: "ai/transcribe".into(),
                    message: "input 'file_path' is required".into(),
                })?;
        let file_path = crate::llm::media::resolve_file_input(file_path_value);
        if file_path.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: "input 'file_path' is required".into(),
            });
        }

        // Route by provider (input > config): "elevenlabs" -> Scribe STT API;
        // anything else -> multimodal transcription via the CLI's LLM provider.
        let get = |k: &str| inputs.get(k).or_else(|| config.get(k));
        let stt_provider = get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("llm")
            .to_lowercase();

        if stt_provider == "elevenlabs" {
            let api_key = get("api_key")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
                .ok_or_else(|| ToolError::ExecutionFailed {
                    tool_type: "ai/transcribe".into(),
                    message: "ElevenLabs API key not found. Set config.api_key or ELEVENLABS_API_KEY env var".into(),
                })?;
            let base = get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.elevenlabs.io")
                .to_string();
            let model = get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(SCRIBE_DEFAULT_MODEL)
                .to_string();
            let language = get("language").and_then(|v| v.as_str()).map(String::from);

            let bytes = tokio::fs::read(&file_path).await.map_err(|e| {
                ToolError::ExecutionFailed {
                    tool_type: "ai/transcribe".into(),
                    message: format!("failed to read audio file '{file_path}': {e}"),
                }
            })?;
            if bytes.is_empty() {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "ai/transcribe".into(),
                    message: format!("audio file '{file_path}' is empty"),
                });
            }
            let file_name = std::path::Path::new(&file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio")
                .to_string();
            let mime = audio_mime_for_path(&file_path);
            let url = eleven_stt_url(&base);
            let text_fields = eleven_stt_text_fields(&model, language.as_deref());

            info!(
                file_path = %file_path,
                model = %model,
                bytes = bytes.len(),
                "ai/transcribe: sending audio to ElevenLabs Scribe"
            );

            let client = reqwest::Client::new();
            let max_retries: u32 = 3;
            let mut last_error = String::new();

            for attempt in 0..=max_retries {
                // multipart::Form is consumed by the request -> rebuild per attempt.
                let part = reqwest::multipart::Part::bytes(bytes.clone())
                    .file_name(file_name.clone())
                    .mime_str(mime)
                    .map_err(|e| ToolError::ExecutionFailed {
                        tool_type: "ai/transcribe".into(),
                        message: format!("invalid mime type '{mime}': {e}"),
                    })?;
                let mut form = reqwest::multipart::Form::new().part("file", part);
                for (k, v) in &text_fields {
                    form = form.text(k.clone(), v.clone());
                }

                let resp = client
                    .post(&url)
                    .header("xi-api-key", &api_key)
                    .timeout(std::time::Duration::from_secs(120))
                    .multipart(form)
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            ToolError::ExecutionFailed {
                                tool_type: "ai/transcribe".into(),
                                message: "elevenlabs STT request timed out after 120s".into(),
                            }
                        } else {
                            ToolError::ExecutionFailed {
                                tool_type: "ai/transcribe".into(),
                                message: format!("HTTP request failed: {e}"),
                            }
                        }
                    })?;

                let status = resp.status().as_u16();

                if (200..300).contains(&status) {
                    let body: Value = resp.json().await.map_err(|e| ToolError::ExecutionFailed {
                        tool_type: "ai/transcribe".into(),
                        message: format!("failed to parse Scribe response JSON: {e}"),
                    })?;
                    let text = body.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    let duration = scribe_duration_seconds(&body);

                    info!(chars = text.chars().count(), duration, "ai/transcribe: Scribe transcription ok");

                    let mut out = HashMap::new();
                    out.insert("text".to_string(), json!(text));
                    out.insert("duration_seconds".to_string(), json!(duration));
                    out.insert("provider".to_string(), json!("elevenlabs"));
                    return Ok(out);
                }

                if matches!(status, 429 | 500 | 502 | 503 | 504) && attempt < max_retries {
                    let body_text = resp.text().await.unwrap_or_default();
                    last_error = format!("HTTP {status}: {body_text}");
                    let delay = 2u64.pow(attempt + 1);
                    warn!(status, attempt = attempt + 1, delay_secs = delay, "ai/transcribe: retriable error, backing off");
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }

                let body_text = resp.text().await.unwrap_or_default();
                let msg = match status {
                    401 | 403 => format!("elevenlabs API key is invalid or unauthorized (HTTP {status})"),
                    _ => format!("elevenlabs STT error (HTTP {status}): {body_text}"),
                };
                return Err(ToolError::ExecutionFailed {
                    tool_type: "ai/transcribe".into(),
                    message: msg,
                });
            }

            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: format!("elevenlabs STT failed after {max_retries} retries: {last_error}"),
            });
        }

        let model = get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        // PRD-009: Read the audio file and send as multimodal content.
        let provider_name = context.llm().provider_name();
        let media = crate::llm::media::read_media_file(&file_path, provider_name).map_err(|e| {
            ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: e,
            }
        })?;

        info!(
            file_path = %file_path,
            mime_type = %media.mime_type,
            provider = %provider_name,
            "transcribe: read media file, sending as multimodal"
        );

        let prompt = "Transcribe this audio faithfully. Include every word exactly as spoken, including filler words (um, uh, like, eh, este, etc.), false starts, and repetitions. Add proper punctuation (periods, commas, question marks, exclamation marks, parentheses, dashes) to reflect the speaker's natural pauses and intonation. Use paragraph breaks for topic changes or long pauses. Do not remove, rephrase, or add any words. Output only the transcription.";

        // Pass media via __user_media carrier (bridge attaches it to user prompt).
        let media_json = serde_json::to_value(vec![&media]).unwrap_or(json!([]));
        let context_messages = vec![json!({ "__user_media": media_json })];

        let result = context
            .llm()
            .call(model, prompt, &context_messages, 0.0, None)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("text".to_string(), json!(result.response));
        out.insert("duration_seconds".to_string(), json!(0.0));
        out.insert("provider".to_string(), json!(provider_name));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

// ===========================================================================
// ClaudeCodeTool — native Claude Code CLI integration
// ===========================================================================

ai_tool! {
    struct ClaudeCodeTool, factory ClaudeCodeFactory;
    tool_type = "ai/claude_code",
    name = "Claude Code CLI",
    description = "Execute a prompt via the locally installed Claude Code CLI (claude -p). Uses the user's existing Claude subscription (Max/Pro). No API key required.",
    inputs = [
        field("prompt", FieldType::String, true, "The prompt to send to Claude"),
        field("context", FieldType::String, false, "Additional context prepended to the prompt"),
    ],
    outputs = [
        field("response", FieldType::String, true, "Claude's response text"),
        field("model", FieldType::String, false, "Model used"),
        field("duration_ms", FieldType::Number, true, "Execution time in milliseconds"),
        field("tokens_input", FieldType::Number, false, "Estimated input tokens (prompt length / 4)"),
        field("tokens_output", FieldType::Number, false, "Estimated output tokens (response length / 4)"),
    ],
    config_fields = [
        field("timeout_ms", FieldType::Number, false, "Timeout in ms (default: 60000)"),
        field("max_tokens", FieldType::Number, false, "Max tokens flag passed to claude CLI"),
        field("system_prompt", FieldType::String, false, "System prompt prepended to the user prompt"),
        field("model", FieldType::String, false, "Model override (e.g. claude-sonnet-4-20250514)"),
        field("cli_path", FieldType::String, false, "Override path to claude binary (default: auto-detect from $PATH)"),
    ]
}

#[async_trait]
impl Tool for ClaudeCodeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        use std::time::Instant;
        use tokio::process::Command;

        let prompt = inputs
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if prompt.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/claude_code".into(),
                message: "prompt is required and cannot be empty".into(),
            });
        }

        let context = inputs
            .get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let system_prompt = config
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let timeout_ms = config
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(60_000);

        let max_tokens = config.get("max_tokens").and_then(|v| v.as_u64());

        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        let cli_path = config
            .get("cli_path")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Detect Claude CLI binary.
        let claude_bin = if let Some(path) = cli_path {
            path
        } else {
            detect_claude_cli().map_err(|e| ToolError::ExecutionFailed {
                tool_type: "ai/claude_code".into(),
                message: e,
            })?
        };

        // Build args.
        let mut args = vec!["-p".to_string()];
        if let Some(ref m) = model {
            args.extend(["--model".to_string(), m.clone()]);
        }
        if let Some(max) = max_tokens {
            args.extend(["--max-tokens".to_string(), max.to_string()]);
        }

        // Build full prompt: system_prompt + context + prompt.
        let mut full_prompt = String::new();
        if !system_prompt.is_empty() {
            full_prompt.push_str(&system_prompt);
            full_prompt.push_str("\n\n");
        }
        if !context.is_empty() {
            full_prompt.push_str(&context);
            full_prompt.push_str("\n\n");
        }
        full_prompt.push_str(&prompt);

        // Spawn process.
        let start = Instant::now();

        let mut child = Command::new(&claude_bin)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "ai/claude_code".into(),
                message: format!("failed to spawn '{}': {}", claude_bin, e),
            })?;

        // Write prompt to stdin, then close it.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(full_prompt.as_bytes()).await;
            let _ = stdin.flush().await;
            drop(stdin);
        }

        // Wait for output with timeout.
        let timeout_dur = std::time::Duration::from_millis(timeout_ms);
        let output = match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "ai/claude_code".into(),
                    message: format!("process error: {}", e),
                });
            }
            Err(_) => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "ai/claude_code".into(),
                    message: format!("claude CLI timed out after {}ms", timeout_ms),
                });
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/claude_code".into(),
                message: format!(
                    "claude CLI exited with {}: {}",
                    output.status,
                    stderr.trim()
                ),
            });
        }

        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Estimate tokens (rough: 1 token ≈ 4 chars).
        let tokens_in = (full_prompt.len() / 4) as u64;
        let tokens_out = (response.len() / 4) as u64;

        let mut out = HashMap::new();
        out.insert("response".to_string(), json!(response));
        out.insert(
            "model".to_string(),
            json!(model.unwrap_or_else(|| "claude-cli-default".into())),
        );
        out.insert("duration_ms".to_string(), json!(elapsed_ms));
        out.insert("tokens_input".to_string(), json!(tokens_in));
        out.insert("tokens_output".to_string(), json!(tokens_out));

        info!(
            tool = "ai/claude_code",
            duration_ms = elapsed_ms,
            tokens_in = tokens_in,
            tokens_out = tokens_out,
            "claude code CLI execution complete"
        );

        Ok(out)
    }
}

/// Detect the Claude Code CLI binary in $PATH.
fn detect_claude_cli() -> Result<String, String> {
    // Check common locations.
    for candidate in &[
        "claude",
        "/opt/homebrew/bin/claude",
        "/usr/local/bin/claude",
    ] {
        if let Ok(output) = std::process::Command::new(candidate)
            .arg("--version")
            .output()
        {
            if output.status.success() {
                return Ok(candidate.to_string());
            }
        }
    }
    Err(
        "Claude Code CLI not found. Install it with: npm install -g @anthropic-ai/claude-code"
            .into(),
    )
}

// ===========================================================================
// ImageEditTool — PRD-017
// ===========================================================================

ai_tool! {
    struct ImageEditTool, factory ImageEditFactory;
    tool_type = "ai/image_edit",
    name = "Image Edit",
    description = "Edit an image using AI inpainting via OpenAI GPT-Image-1. Send an image, an optional mask marking the area to edit (PNG with alpha channel), and a text prompt describing the desired change.",
    inputs = [
        field("image_path", FieldType::String, false, "Path or FileRef of the base image (PNG)"),
        field("mask_path", FieldType::String, false, "Path or FileRef of the mask (PNG with alpha: transparent = area to edit)"),
        field("prompt", FieldType::String, false, "Description of the desired edit"),
    ],
    outputs = [
        field("result_path", FieldType::Object, true, "FileRef of the edited image (PNG in scratch dir)"),
        field("revised_prompt", FieldType::String, false, "Prompt as revised by OpenAI, if different"),
        field("model", FieldType::String, true, "Model used for generation"),
        field("size", FieldType::String, true, "Size of the generated image"),
        field("created", FieldType::Number, true, "Unix timestamp of creation"),
    ],
    config_fields = [
        field("image_path", FieldType::String, false, "Image path (fallback if not in inputs)"),
        field("mask_path", FieldType::String, false, "Mask path (fallback if not in inputs)"),
        field("prompt", FieldType::String, false, "Prompt (fallback if not in inputs)"),
        field("model", FieldType::String, false, "OpenAI model: gpt-image-1 (default) or dall-e-2"),
        field("size", FieldType::String, false, "Output size: 1024x1024, 1536x1024, 1024x1536, auto (default)"),
        field("quality", FieldType::String, false, "Quality (gpt-image-1 only): low, medium, high (default)"),
        field("n", FieldType::Number, false, "Number of images to generate (default 1)"),
        field("api_key", FieldType::String, false, "OpenAI API key (falls back to OPENAI_API_KEY env var)"),
        field("base_url", FieldType::String, false, "API base URL (default https://api.openai.com)"),
    ]
}

#[async_trait]
impl Tool for ImageEditTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // --- 1. Resolve inputs (input > config) ---
        let image_raw = inputs
            .get("image_path")
            .or_else(|| config.get("image_path"))
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: "input 'image_path' is required (via input or config)".into(),
            })?;
        let image_path = crate::llm::media::resolve_file_input(image_raw);
        if image_path.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: "input 'image_path' is required (via input or config)".into(),
            });
        }

        let mask_raw = inputs
            .get("mask_path")
            .or_else(|| config.get("mask_path"));
        let mask_path = mask_raw.map(|v| crate::llm::media::resolve_file_input(v));

        let prompt = inputs
            .get("prompt")
            .or_else(|| config.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if prompt.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: "input 'prompt' is required (via input or config)".into(),
            });
        }

        // --- 2. Resolve config ---
        let api_key = config
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: "OpenAI API key not found. Set config.api_key or OPENAI_API_KEY env var"
                    .into(),
            })?;

        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gpt-image-1");
        let size = config
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let quality = config
            .get("quality")
            .and_then(|v| v.as_str())
            .unwrap_or("high");
        let n = config
            .get("n")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.openai.com");

        // --- 3. Read files ---
        let image_bytes = tokio::fs::read(&image_path).await.map_err(|e| {
            ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: format!("failed to read image file: {image_path}: {e}"),
            }
        })?;

        let mask_bytes = match &mask_path {
            Some(p) if !p.is_empty() => {
                Some(tokio::fs::read(p).await.map_err(|e| {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/image_edit".into(),
                        message: format!("failed to read mask file: {p}: {e}"),
                    }
                })?)
            }
            _ => None,
        };

        // --- 4. Build multipart form ---
        let url = format!("{base_url}/v1/images/edits");
        let client = reqwest::Client::new();
        let max_retries: u32 = 3;
        let mut last_error = String::new();

        for attempt in 0..=max_retries {
            let image_part = reqwest::multipart::Part::bytes(image_bytes.clone())
                .file_name("image.png")
                .mime_str("image/png")
                .unwrap();

            let mut form = reqwest::multipart::Form::new()
                .part("image", image_part)
                .text("prompt", prompt.clone())
                .text("model", model.to_string())
                .text("n", n.to_string());

            if let Some(ref mb) = mask_bytes {
                let mask_part = reqwest::multipart::Part::bytes(mb.clone())
                    .file_name("mask.png")
                    .mime_str("image/png")
                    .unwrap();
                form = form.part("mask", mask_part);
            }

            if model == "gpt-image-1" {
                form = form.text("size", size.to_string());
                form = form.text("quality", quality.to_string());
            }

            // --- 5. POST to OpenAI ---
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .multipart(form)
                .timeout(std::time::Duration::from_secs(120))
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        ToolError::ExecutionFailed {
                            tool_type: "ai/image_edit".into(),
                            message: "OpenAI request timed out after 120s".into(),
                        }
                    } else {
                        ToolError::ExecutionFailed {
                            tool_type: "ai/image_edit".into(),
                            message: format!("HTTP request failed: {e}"),
                        }
                    }
                })?;

            let status = resp.status().as_u16();

            // --- 6. Handle response ---
            if status == 200 {
                let body: Value = resp.json().await.map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/image_edit".into(),
                    message: format!("failed to parse OpenAI response: {e}"),
                })?;

                let b64 = body
                    .pointer("/data/0/b64_json")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed {
                        tool_type: "ai/image_edit".into(),
                        message: "unexpected OpenAI response format: missing data[0].b64_json"
                            .into(),
                    })?;

                let revised = body
                    .pointer("/data/0/revised_prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let created_ts = body
                    .get("created")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let decoded = STANDARD.decode(b64).map_err(|e| {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/image_edit".into(),
                        message: format!("failed to decode base64 image: {e}"),
                    }
                })?;

                // --- 7. Save to scratch dir (fallback to /tmp) ---
                let scratch = context
                    .scratch_dir()
                    .unwrap_or("/tmp");

                let file_id = uuid::Uuid::new_v4();
                let out_path = format!("{scratch}/image_edit_{file_id}.png");
                tokio::fs::write(&out_path, &decoded).await.map_err(|e| {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/image_edit".into(),
                        message: format!("failed to save generated image: {e}"),
                    }
                })?;

                info!(
                    path = %out_path,
                    model = %model,
                    size = %size,
                    revised_prompt_len = revised.len(),
                    "ai/image_edit: saved result"
                );

                // --- 8. Build output ---
                let file_ref = crate::llm::media::create_file_ref(&out_path, None)
                    .unwrap_or_else(|| json!(out_path));

                let mut out = HashMap::new();
                out.insert("result_path".to_string(), file_ref);
                out.insert("revised_prompt".to_string(), json!(revised));
                out.insert("model".to_string(), json!(model));
                out.insert("size".to_string(), json!(size));
                out.insert("created".to_string(), json!(created_ts));
                return Ok(out);
            }

            // --- Retriable errors ---
            if matches!(status, 429 | 500 | 502 | 503 | 504) && attempt < max_retries {
                let body_text = resp.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {body_text}");
                let delay = 2u64.pow(attempt + 1);
                warn!(
                    status,
                    attempt = attempt + 1,
                    delay_secs = delay,
                    "ai/image_edit: retriable error, backing off"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue;
            }

            // --- Non-retriable errors ---
            let body_text = resp.text().await.unwrap_or_default();
            let error_msg = serde_json::from_str::<Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str())
                        .map(String::from)
                })
                .unwrap_or(body_text.clone());

            let msg = match status {
                400 if error_msg.contains("safety") || error_msg.contains("policy") => {
                    format!("OpenAI content policy violation: {error_msg}")
                }
                400 => format!("OpenAI rejected the request: {error_msg}"),
                401 => "OpenAI API key is invalid or expired".into(),
                _ => format!("OpenAI error (HTTP {status}): {error_msg}"),
            };
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/image_edit".into(),
                message: msg,
            });
        }

        // All retries exhausted
        Err(ToolError::ExecutionFailed {
            tool_type: "ai/image_edit".into(),
            message: format!(
                "OpenAI error after {max_retries} retries: {last_error}"
            ),
        })
    }
}

// ===========================================================================
// TtsTool — text-to-speech via ElevenLabs or Cartesia (returns MP3 path)
// ===========================================================================

ai_tool! {
    struct TtsTool, factory TtsFactory;
    tool_type = "ai/tts",
    name = "Text to Speech",
    description = "Synthesize natural speech (MP3) from text via ElevenLabs or Cartesia Sonic. Returns the path to the generated audio file. Provider is selected by config.provider.",
    inputs = [
        field("text", FieldType::String, true, "Text to synthesize into speech"),
    ],
    outputs = [
        field("audio_path", FieldType::Object, true, "FileRef of the generated MP3 in the scratch dir"),
        field("provider", FieldType::String, true, "TTS provider used (elevenlabs | cartesia)"),
        field("format", FieldType::String, true, "Audio format (mp3)"),
        field("characters", FieldType::Number, true, "Number of characters synthesized"),
    ],
    config_fields = [
        field("provider", FieldType::String, false, "TTS provider: elevenlabs (default) or cartesia"),
        field("voice_id", FieldType::String, false, "Voice id. ElevenLabs: goes in URL path. Cartesia: goes in body. Falls back to a provider default."),
        field("model", FieldType::String, false, "Model id (default: eleven_flash_v2_5 / sonic-3.5)"),
        field("api_key", FieldType::String, false, "API key (falls back to ELEVENLABS_API_KEY / CARTESIA_API_KEY env var)"),
        field("language", FieldType::String, false, "ISO language code, e.g. es (Cartesia; optional)"),
        field("stability", FieldType::Number, false, "ElevenLabs voice stability 0-1 (default 0.5)"),
        field("similarity_boost", FieldType::Number, false, "ElevenLabs similarity 0-1 (default 0.75)"),
        field("style", FieldType::Number, false, "ElevenLabs style exaggeration 0-1 (default 0.0)"),
        field("speed", FieldType::Number, false, "Speaking speed (Cartesia generation_config.speed, 0.6-1.5)"),
        field("base_url", FieldType::String, false, "Override API base URL"),
    ]
}

const ELEVEN_DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM"; // Rachel (ElevenLabs default library voice)
const CARTESIA_DEFAULT_VOICE: &str = "a0e99841-438c-4a64-b679-ae501e7d6091";
const CARTESIA_VERSION: &str = "2026-03-01";

// --- Pure request builders (unit-tested without network) ---
fn eleven_tts_url(base_url: &str, voice_id: &str) -> String {
    format!("{base_url}/v1/text-to-speech/{voice_id}?output_format=mp3_44100_128")
}
fn eleven_tts_body(text: &str, model: &str, stability: f64, similarity: f64, style: f64, language: Option<&str>) -> Value {
    let mut body = json!({
        "text": text,
        "model_id": model,
        "voice_settings": {
            "stability": stability,
            "similarity_boost": similarity,
            "style": style,
            "use_speaker_boost": true
        }
    });
    // language_code fuerza el idioma para pronunciacion correcta (soportado en flash/turbo v2.5).
    if let Some(lang) = language {
        if !lang.is_empty() { body["language_code"] = json!(lang); }
    }
    body
}
fn cartesia_tts_body(
    text: &str,
    model: &str,
    voice_id: &str,
    language: Option<&str>,
    speed: Option<f64>,
) -> Value {
    let mut body = json!({
        "model_id": model,
        "transcript": text,
        "voice": { "mode": "id", "id": voice_id },
        "output_format": { "container": "mp3", "sample_rate": 44100, "bit_rate": 128000 }
    });
    if let Some(lang) = language {
        body["language"] = json!(lang);
    }
    if let Some(sp) = speed {
        body["generation_config"] = json!({ "speed": sp });
    }
    body
}

#[async_trait]
impl Tool for TtsTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // --- Resolve text (input > config) ---
        let text = inputs
            .get("text")
            .or_else(|| config.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/tts".into(),
                message: "input 'text' is required (via input or config)".into(),
            });
        }

        // Resolve runtime params from inputs first, then node config (input > config),
        // so the client can parameterize the call via --input.
        let get = |k: &str| inputs.get(k).or_else(|| config.get(k));

        // Provider from input (or node config) — the client picks elevenlabs | cartesia per call.
        let provider = get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("elevenlabs")
            .to_lowercase();

        // --- Build the provider-specific request (url, headers, json body) ---
        let (url, headers, body): (String, Vec<(String, String)>, Value) = match provider.as_str() {
            "cartesia" => {
                let api_key = get("api_key")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| std::env::var("CARTESIA_API_KEY").ok())
                    .ok_or_else(|| ToolError::ExecutionFailed {
                        tool_type: "ai/tts".into(),
                        message: "Cartesia API key not found. Set config.api_key or CARTESIA_API_KEY env var".into(),
                    })?;
                let base = get("base_url").and_then(|v| v.as_str()).unwrap_or("https://api.cartesia.ai");
                let voice = get("voice_id").and_then(|v| v.as_str()).unwrap_or(CARTESIA_DEFAULT_VOICE);
                let model = get("model").and_then(|v| v.as_str()).unwrap_or("sonic-3.5");
                let language = get("language").and_then(|v| v.as_str());
                let speed = get("speed").and_then(|v| v.as_f64());
                let body = cartesia_tts_body(&text, model, voice, language, speed);
                let headers = vec![
                    ("X-API-Key".to_string(), api_key),
                    ("Cartesia-Version".to_string(), CARTESIA_VERSION.to_string()),
                ];
                (format!("{base}/tts/bytes"), headers, body)
            }
            // elevenlabs (default for anything not explicitly "cartesia")
            _ => {
                let api_key = get("api_key")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
                    .ok_or_else(|| ToolError::ExecutionFailed {
                        tool_type: "ai/tts".into(),
                        message: "ElevenLabs API key not found. Set config.api_key or ELEVENLABS_API_KEY env var".into(),
                    })?;
                let base = get("base_url").and_then(|v| v.as_str()).unwrap_or("https://api.elevenlabs.io");
                let voice = get("voice_id").and_then(|v| v.as_str()).unwrap_or(ELEVEN_DEFAULT_VOICE);
                let model = get("model").and_then(|v| v.as_str()).unwrap_or("eleven_flash_v2_5");
                let stability = get("stability").and_then(|v| v.as_f64()).unwrap_or(0.5);
                let similarity = get("similarity_boost").and_then(|v| v.as_f64()).unwrap_or(0.75);
                let style = get("style").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let language = get("language").and_then(|v| v.as_str());
                let body = eleven_tts_body(&text, model, stability, similarity, style, language);
                let headers = vec![
                    ("xi-api-key".to_string(), api_key),
                    ("Accept".to_string(), "audio/mpeg".to_string()),
                ];
                (eleven_tts_url(base, voice), headers, body)
            }
        };

        let scratch = context.scratch_dir().unwrap_or("/tmp");
        let out_path = format!("{scratch}/tts_{}.mp3", uuid::Uuid::new_v4());

        let client = reqwest::Client::new();
        let max_retries: u32 = 3;
        let mut last_error = String::new();

        for attempt in 0..=max_retries {
            let mut req = client
                .post(&url)
                .timeout(std::time::Duration::from_secs(120))
                .json(&body);
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }

            let resp = req.send().await.map_err(|e| {
                if e.is_timeout() {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/tts".into(),
                        message: format!("{provider} TTS request timed out after 120s"),
                    }
                } else {
                    ToolError::ExecutionFailed {
                        tool_type: "ai/tts".into(),
                        message: format!("HTTP request failed: {e}"),
                    }
                }
            })?;

            let status = resp.status().as_u16();

            if (200..300).contains(&status) {
                let bytes = resp.bytes().await.map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/tts".into(),
                    message: format!("failed to read audio bytes: {e}"),
                })?;
                if bytes.is_empty() {
                    return Err(ToolError::ExecutionFailed {
                        tool_type: "ai/tts".into(),
                        message: format!("{provider} returned empty audio"),
                    });
                }
                tokio::fs::write(&out_path, &bytes).await.map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/tts".into(),
                    message: format!("failed to save audio: {e}"),
                })?;

                info!(path = %out_path, provider = %provider, bytes = bytes.len(), "ai/tts: saved audio");

                let file_ref = crate::llm::media::create_file_ref(&out_path, None)
                    .unwrap_or_else(|| json!(out_path));

                let mut out = HashMap::new();
                out.insert("audio_path".to_string(), file_ref);
                out.insert("provider".to_string(), json!(provider));
                out.insert("format".to_string(), json!("mp3"));
                out.insert("characters".to_string(), json!(text.chars().count()));
                return Ok(out);
            }

            // Retriable errors
            if matches!(status, 429 | 500 | 502 | 503 | 504) && attempt < max_retries {
                let body_text = resp.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {body_text}");
                let delay = 2u64.pow(attempt + 1);
                warn!(status, attempt = attempt + 1, delay_secs = delay, "ai/tts: retriable error, backing off");
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue;
            }

            // Non-retriable
            let body_text = resp.text().await.unwrap_or_default();
            let msg = match status {
                401 | 403 => format!("{provider} API key is invalid or unauthorized (HTTP {status})"),
                _ => format!("{provider} TTS error (HTTP {status}): {body_text}"),
            };
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/tts".into(),
                message: msg,
            });
        }

        Err(ToolError::ExecutionFailed {
            tool_type: "ai/tts".into(),
            message: format!("{provider} TTS failed after {max_retries} retries: {last_error}"),
        })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all AI tools into the given registry.
pub fn register_ai_tools(registry: &mut ToolRegistry) {
    registry.register("ai/llm_call", Box::new(LlmCallFactory::new()));
    registry.register("ai/embeddings", Box::new(EmbeddingsFactory::new()));
    registry.register("ai/transcribe", Box::new(TranscribeFactory::new()));
    registry.register("ai/claude_code", Box::new(ClaudeCodeFactory::new()));
    registry.register("ai/image_edit", Box::new(ImageEditFactory::new()));
    registry.register("ai/tts", Box::new(TtsFactory::new()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Session context formatting -------------------------------------------

    #[test]
    fn format_session_context_basic() {
        let mut data = HashMap::new();
        data.insert("name".to_string(), json!("Alice"));
        data.insert("age".to_string(), json!(30));

        let ctx = format_session_context(&data, 0);
        assert!(ctx.contains("=== Session Context ==="));
        assert!(ctx.contains("=== End Session Context ==="));
        assert!(ctx.contains("[name]"));
        assert!(ctx.contains("[age]"));
    }

    #[test]
    fn format_session_context_empty() {
        let data = HashMap::new();
        let ctx = format_session_context(&data, 0);
        assert!(ctx.contains("(no data)"));
    }

    #[test]
    fn format_session_context_truncation() {
        let mut data = HashMap::new();
        // Create a value that's much larger than the budget
        let long_text = "x".repeat(5000);
        data.insert("big".to_string(), json!(long_text));

        let ctx = format_session_context(&data, 500);
        assert!(ctx.contains("[... truncated"));
        assert!(ctx.len() < 5200); // Significantly smaller than original
    }

    // -- Output schema parsing ------------------------------------------------

    #[test]
    fn parse_output_schema_none() {
        assert!(parse_output_schema(None).is_none());
        assert!(parse_output_schema(Some(&Value::Null)).is_none());
        assert!(parse_output_schema(Some(&json!(""))).is_none());
        assert!(parse_output_schema(Some(&json!("  "))).is_none());
    }

    #[test]
    fn parse_output_schema_json_string() {
        let raw = json!(r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#);
        let parsed = parse_output_schema(Some(&raw));
        assert!(parsed.is_some());
        let schema = parsed.unwrap();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn parse_output_schema_object() {
        let raw = json!({"type": "object", "properties": {"x": {"type": "number"}}});
        let parsed = parse_output_schema(Some(&raw));
        assert!(parsed.is_some());
    }

    #[test]
    fn parse_output_schema_invalid_json() {
        let raw = json!("not valid json {{{");
        assert!(parse_output_schema(Some(&raw)).is_none());
    }

    // -- JSON extraction ------------------------------------------------------

    #[test]
    fn extract_json_direct_object() {
        let result = extract_json_from_response(r#"{"name": "test"}"#);
        assert_eq!(result, Some(r#"{"name": "test"}"#.to_string()));
    }

    #[test]
    fn extract_json_direct_array() {
        let result = extract_json_from_response("[1, 2, 3]");
        assert_eq!(result, Some("[1, 2, 3]".to_string()));
    }

    #[test]
    fn extract_json_from_code_block() {
        let text = "Here is the result:\n```json\n{\"key\": \"value\"}\n```\nDone.";
        let result = extract_json_from_response(text);
        assert!(result.is_some());
        let parsed: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn extract_json_embedded_object() {
        let text = "The answer is: {\"score\": 95} and that's it.";
        let result = extract_json_from_response(text);
        assert!(result.is_some());
        let parsed: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(parsed["score"], 95);
    }

    #[test]
    fn extract_json_no_json() {
        let result = extract_json_from_response("Just plain text with no JSON.");
        assert!(result.is_none());
    }

    // -- Schema validation ----------------------------------------------------

    #[test]
    fn validate_response_valid_object() {
        let schema = json!({
            "type": "object",
            "required": ["name", "score"],
            "properties": {
                "name": {"type": "string"},
                "score": {"type": "number"}
            }
        });
        let response = r#"{"name": "test", "score": 42}"#;
        let (parsed, errors) = validate_response(response, &schema);
        assert!(parsed.is_some());
        assert!(errors.is_empty());
    }

    #[test]
    fn validate_response_missing_required() {
        let schema = json!({
            "type": "object",
            "required": ["name", "score"],
            "properties": {
                "name": {"type": "string"},
                "score": {"type": "number"}
            }
        });
        let response = r#"{"name": "test"}"#;
        let (parsed, errors) = validate_response(response, &schema);
        assert!(parsed.is_some());
        assert!(!errors.is_empty());
        assert!(errors[0].contains("Missing required field: 'score'"));
    }

    #[test]
    fn validate_response_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "score": {"type": "number"}
            }
        });
        let response = r#"{"score": "not a number"}"#;
        let (parsed, errors) = validate_response(response, &schema);
        assert!(parsed.is_some());
        assert!(!errors.is_empty());
        assert!(errors[0].contains("expected type 'number'"));
    }

    #[test]
    fn validate_response_enum_check() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["active", "inactive"]}
            }
        });
        let response = r#"{"status": "unknown"}"#;
        let (_, errors) = validate_response(response, &schema);
        assert!(!errors.is_empty());
        assert!(errors[0].contains("must be one of"));
    }

    #[test]
    fn validate_response_no_json() {
        let schema = json!({"type": "object"});
        let (parsed, errors) = validate_response("no json here", &schema);
        assert!(parsed.is_none());
        assert!(!errors.is_empty());
    }

    // -- Enriched prompt ------------------------------------------------------

    #[test]
    fn enrich_prompt_contains_schema() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "number"}}});
        let enriched = enrich_prompt_with_schema("Summarize this", &schema);
        assert!(enriched.contains("Summarize this"));
        assert!(enriched.contains("OUTPUT FORMAT (MANDATORY)"));
        assert!(enriched.contains("END OUTPUT FORMAT"));
    }

    // -- Retry prompt ---------------------------------------------------------

    #[test]
    fn build_retry_prompt_contains_errors() {
        let retry = build_retry_prompt(
            "Original prompt",
            "Bad response",
            &["Missing field: 'name'".into()],
        );
        assert!(retry.contains("Original prompt"));
        assert!(retry.contains("RETRY"));
        assert!(retry.contains("Missing field: 'name'"));
        assert!(retry.contains("Bad response"));
    }

    // -- Type checking --------------------------------------------------------

    #[test]
    fn check_json_type_variants() {
        assert!(check_json_type(&json!("hello"), "string"));
        assert!(!check_json_type(&json!("hello"), "number"));
        assert!(check_json_type(&json!(42), "number"));
        assert!(check_json_type(&json!(42), "integer"));
        assert!(check_json_type(&json!(true), "boolean"));
        assert!(check_json_type(&json!([1, 2]), "array"));
        assert!(check_json_type(&json!({"a": 1}), "object"));
        assert!(check_json_type(&json!(null), "null"));
        // Unknown type should pass
        assert!(check_json_type(&json!("anything"), "custom_type"));
    }

    // -- Registration ---------------------------------------------------------

    #[test]
    fn register_ai_tools_adds_six() {
        let mut reg = ToolRegistry::new();
        register_ai_tools(&mut reg);
        assert!(reg.get("ai/llm_call").is_some());
        assert!(reg.get("ai/embeddings").is_some());
        assert!(reg.get("ai/transcribe").is_some());
        assert!(reg.get("ai/claude_code").is_some());
        assert!(reg.get("ai/image_edit").is_some());
        assert!(reg.get("ai/tts").is_some());
        assert_eq!(reg.list_tools().len(), 6);
    }

    // -- ai/tts request builders ----------------------------------------------

    #[test]
    fn eleven_url_has_voice_and_mp3() {
        let u = eleven_tts_url("https://api.elevenlabs.io", "abc123");
        assert!(u.contains("/v1/text-to-speech/abc123"));
        assert!(u.contains("output_format=mp3_44100_128"));
    }

    #[test]
    fn eleven_body_shape() {
        let b = eleven_tts_body("hola", "eleven_flash_v2_5", 0.5, 0.75, 0.1, None);
        assert_eq!(b["text"], "hola");
        assert_eq!(b["model_id"], "eleven_flash_v2_5");
        assert_eq!(b["voice_settings"]["stability"], 0.5);
        assert_eq!(b["voice_settings"]["similarity_boost"], 0.75);
        assert_eq!(b["voice_settings"]["use_speaker_boost"], true);
        assert!(b.get("language_code").is_none());
    }

    #[test]
    fn eleven_body_includes_language() {
        let b = eleven_tts_body("hello", "eleven_flash_v2_5", 0.5, 0.75, 0.0, Some("en"));
        assert_eq!(b["language_code"], "en");
        let b2 = eleven_tts_body("hi", "eleven_flash_v2_5", 0.5, 0.75, 0.0, Some(""));
        assert!(b2.get("language_code").is_none());
    }

    #[test]
    fn cartesia_body_shape() {
        let b = cartesia_tts_body("hola", "sonic-3.5", "voice-xyz", Some("es"), Some(1.0));
        assert_eq!(b["model_id"], "sonic-3.5");
        assert_eq!(b["transcript"], "hola");
        assert_eq!(b["voice"]["mode"], "id");
        assert_eq!(b["voice"]["id"], "voice-xyz");
        assert_eq!(b["output_format"]["container"], "mp3");
        assert_eq!(b["output_format"]["sample_rate"], 44100);
        assert_eq!(b["language"], "es");
        assert_eq!(b["generation_config"]["speed"], 1.0);
    }

    #[test]
    fn cartesia_body_omits_optional() {
        let b = cartesia_tts_body("hi", "sonic-3.5", "v", None, None);
        assert!(b.get("language").is_none());
        assert!(b.get("generation_config").is_none());
    }

    // -- ai/transcribe (ElevenLabs Scribe route) --------------------------------

    #[test]
    fn scribe_url_format() {
        assert_eq!(
            eleven_stt_url("https://api.elevenlabs.io"),
            "https://api.elevenlabs.io/v1/speech-to-text"
        );
    }

    #[test]
    fn scribe_fields_default_and_language() {
        let f = eleven_stt_text_fields(SCRIBE_DEFAULT_MODEL, None);
        assert!(f.contains(&("model_id".to_string(), "scribe_v1".to_string())));
        assert!(f.contains(&("tag_audio_events".to_string(), "false".to_string())));
        assert!(!f.iter().any(|(k, _)| k == "language_code"));

        let f = eleven_stt_text_fields("scribe_v2", Some("es"));
        assert!(f.contains(&("model_id".to_string(), "scribe_v2".to_string())));
        assert!(f.contains(&("language_code".to_string(), "es".to_string())));

        let f = eleven_stt_text_fields("scribe_v1", Some(""));
        assert!(!f.iter().any(|(k, _)| k == "language_code"));
    }

    #[test]
    fn scribe_audio_mime_mapping() {
        assert_eq!(audio_mime_for_path("/tmp/a.m4a"), "audio/mp4");
        assert_eq!(audio_mime_for_path("/tmp/a.WAV"), "audio/wav");
        assert_eq!(audio_mime_for_path("/tmp/a.mp3"), "audio/mpeg");
        assert_eq!(audio_mime_for_path("/tmp/a.webm"), "audio/webm");
        assert_eq!(audio_mime_for_path("/tmp/noext"), "application/octet-stream");
    }

    #[test]
    fn scribe_duration_from_last_word() {
        let body = json!({
            "text": "hola mundo",
            "words": [
                { "text": "hola", "start": 0.1, "end": 0.5, "type": "word" },
                { "text": " ", "start": 0.5, "end": 0.6, "type": "spacing" },
                { "text": "mundo", "start": 0.6, "end": 1.2, "type": "word" }
            ]
        });
        assert!((scribe_duration_seconds(&body) - 1.2).abs() < 1e-9);
        assert_eq!(scribe_duration_seconds(&json!({ "text": "x" })), 0.0);
        assert_eq!(scribe_duration_seconds(&json!({ "text": "x", "words": [] })), 0.0);
    }
}
