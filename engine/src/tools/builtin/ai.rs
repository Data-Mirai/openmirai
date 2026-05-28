use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{FieldType, ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder (same pattern as logic.rs)
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
        required,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.into())
        },
        default: None,
    }
}

// ---------------------------------------------------------------------------
// Macro: simplify boilerplate for struct + factory + spec
// ---------------------------------------------------------------------------

macro_rules! ai_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: "ai".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

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
        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let temperature = config
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);
        let max_tokens = config
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(1024) as u32;

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
            let media = crate::llm::media::read_media_file(&media_path, &provider_name)
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "ai/llm_call".into(),
                    message: e,
                })?;
            info!(
                media_path = %media_path,
                mime_type = %media.mime_type,
                provider = %provider_name,
                "llm_call: attaching media file to prompt"
            );
            let media_json = serde_json::to_value(vec![&media]).unwrap_or(json!([]));
            context_messages.push(json!({ "__user_media": media_json }));
        }

        // --- Execute with optional schema validation + retries ---
        let mut current_prompt = effective_prompt.clone();
        let mut last_response = String::new();
        let mut last_tokens_input = 0u32;
        let mut last_tokens_output = 0u32;

        for attempt in 0..=max_retries {
            let result = context
                .llm()
                .call(model, &current_prompt, &context_messages, temperature, max_tokens)
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

            if parsed.is_some() && errors.is_empty() {
                let mut out = HashMap::new();
                out.insert("response".to_string(), json!(last_response));
                out.insert("model".to_string(), json!(used_model));
                out.insert("tokens_input".to_string(), json!(last_tokens_input));
                out.insert("tokens_output".to_string(), json!(last_tokens_output));
                out.insert("structured_output".to_string(), parsed.unwrap());
                out.insert("schema_valid".to_string(), json!(true));
                return Ok(out);
            }

            // --- Retry with error feedback ---
            if attempt < max_retries {
                warn!(
                    attempt = attempt + 1,
                    total = max_retries + 1,
                    errors = ?errors,
                    "output_schema validation failed, retrying"
                );
                current_prompt =
                    build_retry_prompt(&effective_prompt, &last_response, &errors);
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
        let text = inputs
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "ai/embeddings".into(),
                message: "input 'text' is required".into(),
            })?;

        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let embedding = context
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
                        let truncated =
                            format!("{}\n[... truncated {} -> {} chars]", &value[..cap], value.len(), cap);
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
    let schema_str =
        serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
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
        return (Some(parsed), vec![format!("Expected JSON object, got {}", type_name)]);
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
                        let actual_type = json_type_name(value);
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

/// Get a human-readable type name for a JSON value.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Build a retry prompt with error feedback.
fn build_retry_prompt(original_prompt: &str, bad_response: &str, errors: &[String]) -> String {
    let error_list: String = errors.iter().map(|e| format!("- {}", e)).collect::<Vec<_>>().join("\n");
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
    description = "Transcribes audio content using an LLM to describe/transcribe the file",
    inputs = [
        field("file_path", FieldType::String, true, "Path to the audio file"),
    ],
    outputs = [
        field("text", FieldType::String, true, "Transcription text"),
        field("duration_seconds", FieldType::Number, true, "Audio duration in seconds"),
    ],
    config_fields = [
        field("model", FieldType::String, false, "Model to use for transcription"),
    ]
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
        let file_path_value = inputs
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

        let model = config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        // PRD-009: Read the audio file and send as multimodal content.
        let provider_name = context.llm().provider_name();
        let media = crate::llm::media::read_media_file(&file_path, &provider_name)
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: e,
            })?;

        info!(
            file_path = %file_path,
            mime_type = %media.mime_type,
            provider = %provider_name,
            "transcribe: read media file, sending as multimodal"
        );

        let prompt = "Transcribe this audio. Output only the raw transcription text, no timestamps, no speaker labels, no formatting.";

        // Pass media via __user_media carrier (bridge attaches it to user prompt).
        let media_json = serde_json::to_value(vec![&media]).unwrap_or(json!([]));
        let context_messages = vec![json!({ "__user_media": media_json })];

        let result = context
            .llm()
            .call(model, prompt, &context_messages, 0.0, 4096)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "ai/transcribe".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("text".to_string(), json!(result.response));
        out.insert("duration_seconds".to_string(), json!(0.0));
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

        let prompt = inputs.get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if prompt.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "ai/claude_code".into(),
                message: "prompt is required and cannot be empty".into(),
            });
        }

        let context = inputs.get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let system_prompt = config.get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let timeout_ms = config.get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(60_000);

        let max_tokens = config.get("max_tokens")
            .and_then(|v| v.as_u64());

        let model = config.get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        let cli_path = config.get("cli_path")
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
                message: format!("claude CLI exited with {}: {}", output.status, stderr.trim()),
            });
        }

        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Estimate tokens (rough: 1 token ≈ 4 chars).
        let tokens_in = (full_prompt.len() / 4) as u64;
        let tokens_out = (response.len() / 4) as u64;

        let mut out = HashMap::new();
        out.insert("response".to_string(), json!(response));
        out.insert("model".to_string(), json!(model.unwrap_or_else(|| "claude-cli-default".into())));
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
    for candidate in &["claude", "/opt/homebrew/bin/claude", "/usr/local/bin/claude"] {
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

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all AI tools into the given registry.
pub fn register_ai_tools(registry: &mut ToolRegistry) {
    registry.register("ai/llm_call", Box::new(LlmCallFactory::new()));
    registry.register("ai/embeddings", Box::new(EmbeddingsFactory::new()));
    registry.register("ai/transcribe", Box::new(TranscribeFactory::new()));
    registry.register("ai/claude_code", Box::new(ClaudeCodeFactory::new()));
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
    fn register_ai_tools_adds_four() {
        let mut reg = ToolRegistry::new();
        register_ai_tools(&mut reg);
        assert!(reg.get("ai/llm_call").is_some());
        assert!(reg.get("ai/embeddings").is_some());
        assert!(reg.get("ai/transcribe").is_some());
        assert!(reg.get("ai/claude_code").is_some());
        assert_eq!(reg.list_tools().len(), 4);
    }
}
