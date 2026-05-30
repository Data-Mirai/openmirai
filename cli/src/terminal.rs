//! Interactive agent terminal — the agentic loop that powers Mirai Code.
//!
//! User talks to an LLM that has access to the engine's tool catalog.
//! The LLM reasons, calls tools, gets results, and repeats until done.

use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Instant;

use openmirai_engine::llm::{
    FunctionCall, LLMAdapter, Message, NormalizedResponse, ToolCallRequest,
};
use openmirai_engine::tools::builtin::register_all_builtin_tools;
use openmirai_engine::tools::registry::ToolRegistry;
use openmirai_engine::tools::base::ToolSpec;

use serde_json::{json, Value};

use crate::adapter_factory;
use crate::colors::*;
use crate::session_storage::SessionStorage;
use crate::setup_wizard::SessionConfig;

// ---------------------------------------------------------------------------
// Token tracking
// ---------------------------------------------------------------------------

pub struct TokenTracker {
    pub total_input: u32,
    pub total_output: u32,
    pub calls: u32,
}

impl TokenTracker {
    pub fn new() -> Self {
        Self {
            total_input: 0,
            total_output: 0,
            calls: 0,
        }
    }

    pub fn add(&mut self, input: u32, output: u32) {
        self.total_input += input;
        self.total_output += output;
        self.calls += 1;
    }

    pub fn total(&self) -> u32 {
        self.total_input + self.total_output
    }

    pub fn summary(&self) -> String {
        format!(
            "{} tokens ({} in / {} out) across {} calls",
            self.total(),
            self.total_input,
            self.total_output,
            self.calls
        )
    }
}

// ---------------------------------------------------------------------------
// Autonomy config
// ---------------------------------------------------------------------------

struct AutonomyConfig {
    level: String,
    max_tool_rounds: u32,
    confirm_writes: bool,
}

fn get_autonomy(level: &str) -> AutonomyConfig {
    match level {
        "assisted" => AutonomyConfig {
            level: "assisted".to_string(),
            max_tool_rounds: 1,
            confirm_writes: true,
        },
        "autopilot" => AutonomyConfig {
            level: "autopilot".to_string(),
            max_tool_rounds: 50,
            confirm_writes: false,
        },
        "self_driving" => AutonomyConfig {
            level: "self_driving".to_string(),
            max_tool_rounds: 100,
            confirm_writes: false,
        },
        _ => AutonomyConfig {
            level: "copilot".to_string(),
            max_tool_rounds: 25,
            confirm_writes: false,
        },
    }
}

// Write tool types that require confirmation at assisted level.
const WRITE_TOOLS: &[&str] = &[
    "fs/write_file",
    "fs/edit_file",
    "fs/move",
    "fs/copy",
    "fs/delete",
    "fs/mkdir",
    "system/bash",
    "git/commit",
];

fn needs_confirmation(autonomy: &AutonomyConfig, tool_type: &str) -> bool {
    if !autonomy.confirm_writes {
        return false;
    }
    WRITE_TOOLS.iter().any(|t| *t == tool_type)
}

fn ask_confirmation(tool_type: &str, args: &Value) -> bool {
    let preview = format_args_preview(args);
    print!("  {YELLOW}? Confirm {tool_type}({preview})? [Y/n]: {RESET}");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    let trimmed = input.trim().to_lowercase();
    trimmed.is_empty() || trimmed == "y" || trimmed == "yes" || trimmed == "si" || trimmed == "s"
}

// ---------------------------------------------------------------------------
// Tool schema builder (ToolSpec -> OpenAI function calling JSON)
// ---------------------------------------------------------------------------

/// Tool types excluded from the CLI schema (graph-only blocks).
const EXCLUDED_TOOLS: &[&str] = &[
    "trigger/webhook",
    "trigger/manual",
    "trigger/schedule",
    "trigger/heartbeat",
    "output/response",
];

/// Core tools for local models (smaller context, fewer tools).
const CORE_TOOLS: &[&str] = &[
    "fs/read_file",
    "fs/write_file",
    "fs/edit_file",
    "fs/glob",
    "fs/grep",
    "fs/list_dir",
    "fs/tree",
    "fs/mkdir",
    "fs/delete",
    "system/bash",
    "git/status",
    "git/diff",
    "git/log",
    "git/commit",
];

fn type_map(t: &openmirai_engine::FieldType) -> &'static str {
    match t {
        openmirai_engine::FieldType::String => "string",
        openmirai_engine::FieldType::Number => "number",
        openmirai_engine::FieldType::Boolean => "boolean",
        openmirai_engine::FieldType::Object => "object",
        openmirai_engine::FieldType::Array => "array",
        openmirai_engine::FieldType::Integer => "integer",
        openmirai_engine::FieldType::File => "file",
    }
}

fn spec_to_openai_schema(spec: &ToolSpec) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for inp in &spec.inputs {
        let mut prop = serde_json::Map::new();
        prop.insert("type".to_string(), json!(type_map(&inp.field_type)));
        if let Some(ref desc) = inp.description {
            prop.insert("description".to_string(), json!(desc));
        }
        properties.insert(inp.name.clone(), Value::Object(prop));
        if inp.required {
            required.push(json!(inp.name));
        }
    }

    for cfg in &spec.config_fields {
        if properties.contains_key(&cfg.name) {
            continue;
        }
        let mut prop = serde_json::Map::new();
        prop.insert("type".to_string(), json!(type_map(&cfg.field_type)));
        if let Some(ref desc) = cfg.description {
            prop.insert("description".to_string(), json!(desc));
        }
        if let Some(ref def) = cfg.default {
            prop.insert("default".to_string(), def.clone());
        }
        properties.insert(cfg.name.clone(), Value::Object(prop));
    }

    json!({
        "type": "function",
        "function": {
            "name": spec.tool_type.replace("/", "_"),
            "description": spec.description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        },
    })
}

fn build_tool_schemas(registry: &ToolRegistry, core_only: bool) -> Vec<Value> {
    registry
        .list_tools()
        .iter()
        .filter(|s| !EXCLUDED_TOOLS.contains(&s.tool_type.as_str()))
        .filter(|s| !core_only || CORE_TOOLS.contains(&s.tool_type.as_str()))
        .map(|s| spec_to_openai_schema(s))
        .collect()
}

/// Map function names (underscores) -> tool_type (slashes).
fn build_name_map(registry: &ToolRegistry) -> HashMap<String, String> {
    registry
        .list_tools()
        .iter()
        .filter(|s| !EXCLUDED_TOOLS.contains(&s.tool_type.as_str()))
        .map(|s| (s.tool_type.replace("/", "_"), s.tool_type.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Tool executor
// ---------------------------------------------------------------------------

async fn execute_tool(
    registry: &ToolRegistry,
    tool_type: &str,
    arguments: &Value,
    cwd: &str,
) -> Value {
    let factory = match registry.get(tool_type) {
        Some(f) => f,
        None => return json!({"error": format!("Unknown tool: {tool_type}")}),
    };

    let tool = factory.create();
    let spec = factory.spec();

    let input_names: std::collections::HashSet<&str> =
        spec.inputs.iter().map(|i| i.name.as_str()).collect();
    let config_names: std::collections::HashSet<&str> =
        spec.config_fields.iter().map(|c| c.name.as_str()).collect();

    let mut inputs: HashMap<String, Value> = HashMap::new();
    let mut config: HashMap<String, Value> = HashMap::new();

    if let Some(obj) = arguments.as_object() {
        for (key, value) in obj {
            if input_names.contains(key.as_str()) {
                inputs.insert(key.clone(), value.clone());
            } else if config_names.contains(key.as_str()) {
                config.insert(key.clone(), value.clone());
            } else {
                inputs.insert(key.clone(), value.clone());
            }
        }
    }

    // Resolve relative paths against cwd
    for key in &["path", "source", "destination"] {
        if let Some(val) = inputs.get(*key) {
            if let Some(s) = val.as_str() {
                if !s.starts_with('/') {
                    let abs = std::path::Path::new(cwd).join(s);
                    inputs.insert(
                        key.to_string(),
                        Value::String(abs.to_string_lossy().to_string()),
                    );
                }
            }
        }
    }

    if config_names.contains("cwd") && !config.contains_key("cwd") {
        config.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }

    // We need a minimal ExecutionContext for tool execution.
    let ctx = openmirai_engine::adapters::DefaultExecutionContext::default_dev();

    match tool.execute(inputs, &config, &ctx).await {
        Ok(result) => {
            let map: serde_json::Map<String, Value> = result.into_iter().collect();
            Value::Object(map)
        }
        Err(e) => json!({"error": format!("{e}")}),
    }
}

// ---------------------------------------------------------------------------
// System prompt builder
// ---------------------------------------------------------------------------

fn build_system_prompt(cwd: &str, autonomy_level: &str) -> String {
    let platform = format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH);

    let autonomy_section = match autonomy_level {
        "assisted" => "\
# Autonomy: ASSISTED (L1)
- Execute ONE tool call per turn. Then STOP and report.
- ALWAYS explain what you'll do BEFORE doing it.
- After each action, ask the user what to do next.",
        "autopilot" => "\
# Autonomy: AUTOPILOT (L3)
- Work autonomously toward the objective until done or stuck.
- Make decisions independently. Debug failures yourself.
- Report progress every 5-10 tool calls.",
        "self_driving" => "\
# Autonomy: SELF-DRIVING (L4)
- Pursue business goals independently. Decompose into sub-tasks.
- NEVER ask for input unless critical ambiguity.
- Commit after each sub-task.",
        _ => "\
# Autonomy: COPILOT (L2)
- Execute multiple tool calls to complete the request in one turn.
- Read files before modifying them. Verify changes after.
- Do NOT ask 'should I proceed?' -- the user already told you to do it.",
    };

    format!(
        "You are Mirai Code -- an agentic coding assistant that runs in the terminal.\n\
\n\
# Environment\n\
- Working directory: {cwd}\n\
- Platform: {platform}\n\
\n\
# CRITICAL RULE: You MUST use tool calls to take actions\n\
- NEVER write text describing what you would do. ALWAYS actually call the tool.\n\
- If you want to create a file -> call fs_write_file\n\
- If you want to read a file -> call fs_read_file\n\
- If you want to run a command -> call system_bash\n\
\n\
# Tool usage guidelines\n\
- fs_read_file: Read files before editing them\n\
- fs_edit_file: Surgical search-and-replace edits on existing files\n\
- fs_write_file: Create new files or complete rewrites\n\
- fs_glob / fs_grep: Find files and search code\n\
- system_bash: Run shell commands (tests, builds, installs, etc.)\n\
- git_status / git_diff / git_log: Understand repository state\n\
- git_commit: Commit changes (only when explicitly asked)\n\
\n\
# Safety guardrails\n\
- NEVER run destructive commands (rm -rf /, drop database without WHERE)\n\
- NEVER modify .env files or files containing secrets\n\
- NEVER git push, git reset --hard, or git push --force\n\
- If after 5 attempts something doesn't compile or pass tests, STOP and report\n\
\n\
# Communication\n\
- Respond in the same language the user speaks\n\
- Be direct, technical, and concise\n\
\n\
{autonomy_section}"
    )
}

// ---------------------------------------------------------------------------
// Format helpers
// ---------------------------------------------------------------------------

fn format_args_preview(args: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(obj) = args.as_object() {
        for (k, v) in obj.iter().take(3) {
            let val = v.to_string();
            let truncated = if val.len() > 60 {
                format!("{}...", &val[..57])
            } else {
                val
            };
            parts.push(format!("{k}={truncated}"));
        }
    }
    let text = parts.join(", ");
    if text.len() > 200 {
        format!("{}...", &text[..197])
    } else {
        text
    }
}

fn summarize_result(result: &Value) -> String {
    let mut parts = Vec::new();
    for key in &[
        "path", "count", "lines", "files_changed", "exit_code", "branch", "hash",
    ] {
        if let Some(v) = result.get(key) {
            let val = v.to_string();
            let truncated = if val.len() > 50 {
                format!("{}...", &val[..47])
            } else {
                val
            };
            parts.push(format!("{key}={truncated}"));
        }
    }
    if !parts.is_empty() {
        parts[..parts.len().min(4)].join(", ")
    } else if let Some(obj) = result.as_object() {
        let keys: Vec<&String> = obj.keys().take(5).collect();
        format!("keys: {:?}", keys)
    } else {
        "ok".to_string()
    }
}

fn estimate_tokens(messages: &[Value]) -> u32 {
    let mut total = 0u32;
    for msg in messages {
        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
            total += (content.len() / 4) as u32;
        }
        if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                if let Some(args) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                {
                    total += (args.len() / 4) as u32;
                }
            }
        }
    }
    total
}

fn compact_messages(messages: &mut Vec<Value>, max_tokens: u32) {
    let est = estimate_tokens(messages);
    if est <= max_tokens {
        return;
    }

    let keep_tail = 12usize;
    let end = messages.len().saturating_sub(keep_tail).max(1);

    // Truncate long tool results in older messages
    for msg in messages[1..end].iter_mut() {
        if msg.get("role").and_then(|v| v.as_str()) == Some("tool") {
            if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                if content.len() > 500 {
                    let truncated = format!("{}... (compacted)", &content[..200]);
                    msg.as_object_mut().map(|o| {
                        o.insert("content".to_string(), Value::String(truncated))
                    });
                }
            }
        }
    }

    let est = estimate_tokens(messages);
    if est <= max_tokens {
        return;
    }

    // Drop old messages, keep system + summary + tail
    if messages.len() > keep_tail + 1 {
        let system = messages[0].clone();
        let summary = json!({
            "role": "user",
            "content": "(Earlier conversation was compacted to save context. Continue from here.)"
        });
        let tail: Vec<Value> = messages[messages.len() - keep_tail..].to_vec();
        messages.clear();
        messages.push(system);
        messages.push(summary);
        messages.extend(tail);
    }
}

// ---------------------------------------------------------------------------
// Spinner frames
// ---------------------------------------------------------------------------

const SPINNER: &[&str] = &[
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}",
    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}",
];

// ---------------------------------------------------------------------------
// The agentic loop
// ---------------------------------------------------------------------------

async fn agentic_loop(
    adapter: &dyn LLMAdapter,
    model: &str,
    messages: &mut Vec<Value>,
    tools: &[Value],
    name_map: &HashMap<String, String>,
    registry: &ToolRegistry,
    cwd: &str,
    tracker: &mut TokenTracker,
    autonomy: &AutonomyConfig,
    temperature: f32,
    max_tokens: u32,
    _context_window: Option<u32>,
    storage: &SessionStorage,
    session_id: &str,
) -> Result<(String, bool), String> {
    let max_rounds = autonomy.max_tool_rounds;
    let mut was_streamed = false;

    for round_num in 0..max_rounds {
        // Convert Value messages to engine Message structs
        let engine_messages: Vec<Message> = messages
            .iter()
            .map(|v| value_to_message(v))
            .collect();

        let tool_defs = if tools.is_empty() {
            None
        } else {
            Some(tools.to_vec())
        };

        if round_num > 0 {
            print!("  {DIM}(round {}){RESET}\n", round_num + 1);
        }

        // Show spinner while waiting
        print!(
            "\r  {MAGENTA}{}{RESET} {DIM}Generating...{RESET}",
            SPINNER[0]
        );
        let _ = io::stdout().flush();

        // Try streaming first, fall back to non-streaming
        let response: NormalizedResponse = {
            let on_token_fn = |token: &str| {
                // For streaming: print tokens as they arrive
                // We handle <think> tags by simply not printing them in the final output
                print!("{token}");
                let _ = io::stdout().flush();
            };

            // Check if the adapter supports streaming by trying it
            let stream_result = adapter
                .stream_with_messages(
                    model,
                    engine_messages.clone(),
                    tool_defs.clone(),
                    temperature,
                    max_tokens,
                    Some(&on_token_fn),
                )
                .await;

            match stream_result {
                Ok(resp) => {
                    // Clear spinner
                    print!("\r{}\r", " ".repeat(60));
                    let _ = io::stdout().flush();
                    was_streamed = true;
                    resp
                }
                Err(_e) => {
                    // Try non-streaming fallback
                    print!("\r{}\r", " ".repeat(60));
                    let _ = io::stdout().flush();

                    match adapter
                        .call_with_messages(
                            model,
                            engine_messages,
                            tool_defs,
                            temperature,
                            max_tokens,
                        )
                        .await
                    {
                        Ok(resp) => resp,
                        Err(e2) => {
                            return Err(format!("LLM error: {e2}"));
                        }
                    }
                }
            }
        };

        tracker.add(response.tokens_used.input, response.tokens_used.output);

        // No tool calls -> we have the final response
        if response.tool_calls.is_empty() {
            return Ok((response.response, was_streamed));
        }

        // Build assistant message with tool_calls
        let tool_calls_json: Vec<Value> = response
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments,
                    },
                })
            })
            .collect();

        let mut assistant_msg = json!({
            "role": "assistant",
        });
        if !response.response.is_empty() {
            assistant_msg["content"] = json!(response.response);
        }
        assistant_msg["tool_calls"] = json!(tool_calls_json);
        messages.push(assistant_msg);

        // Execute each tool call
        for tc in &response.tool_calls {
            let tool_type = name_map
                .get(&tc.name)
                .cloned()
                .unwrap_or_else(|| tc.name.clone());

            let args: Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

            print!(
                "  {CYAN}\u{25B6} {tool_type}{RESET} {DIM}{}{RESET}\n",
                format_args_preview(&args)
            );

            // Check autonomy confirmation
            if needs_confirmation(autonomy, &tool_type) {
                if !ask_confirmation(&tool_type, &args) {
                    println!("    {YELLOW}\u{23ED} Skipped by user{RESET}");
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": r#"{"error": "User declined this action"}"#,
                    }));
                    continue;
                }
            }

            // Log tool call
            storage.append_tool_call(session_id, &tool_type, &args, round_num as u32);

            let start = Instant::now();
            let result = execute_tool(registry, &tool_type, &args, cwd).await;
            let elapsed = start.elapsed().as_secs_f64();

            // Log tool result
            storage.append_tool_result(session_id, &tool_type, &result, round_num as u32);

            if result.get("error").is_some() {
                let err_msg = result["error"].as_str().unwrap_or("unknown error");
                let truncated = if err_msg.len() > 120 {
                    &err_msg[..120]
                } else {
                    err_msg
                };
                println!("    {RED}\u{2717} {truncated}{RESET} {DIM}({elapsed:.1}s){RESET}");
            } else {
                let summary = summarize_result(&result);
                println!(
                    "    {GREEN}\u{2713}{RESET} {DIM}{summary} ({elapsed:.1}s){RESET}"
                );
            }

            // Truncate large results
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            let content = if result_str.len() > 30_000 {
                format!("{}\n... (truncated)", &result_str[..30_000])
            } else {
                result_str
            };

            messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content,
            }));
        }
    }

    Ok((
        "(Max tool-call rounds reached. Please continue or rephrase.)".to_string(),
        false,
    ))
}

// ---------------------------------------------------------------------------
// Convert serde_json::Value <-> engine Message
// ---------------------------------------------------------------------------

fn value_to_message(v: &Value) -> Message {
    let role = v
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("user")
        .to_string();
    let content = v.get("content").and_then(|c| c.as_str()).map(String::from);
    let tool_call_id = v
        .get("tool_call_id")
        .and_then(|t| t.as_str())
        .map(String::from);

    let tool_calls = v.get("tool_calls").and_then(|tcs| {
        tcs.as_array().map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    let id = tc.get("id")?.as_str()?.to_string();
                    let func = tc.get("function")?;
                    let name = func.get("name")?.as_str()?.to_string();
                    let arguments = func
                        .get("arguments")
                        .map(|a| {
                            if a.is_string() {
                                a.as_str().unwrap_or("{}").to_string()
                            } else {
                                serde_json::to_string(a).unwrap_or_default()
                            }
                        })
                        .unwrap_or_else(|| "{}".to_string());
                    Some(ToolCallRequest {
                        id,
                        function: FunctionCall { name, arguments },
                    })
                })
                .collect()
        })
    });

    Message {
        role,
        content,
        tool_calls,
        tool_call_id,
        media: None,
    }
}

// ---------------------------------------------------------------------------
// Slash command handler
// ---------------------------------------------------------------------------

fn handle_slash(
    cmd: &str,
    messages: &mut Vec<Value>,
    tracker: &TokenTracker,
    tool_schemas: &[Value],
    storage: &SessionStorage,
    session_id: &str,
) -> Option<&'static str> {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let command = parts[0].to_lowercase();

    match command.as_str() {
        "/quit" | "/exit" | "/q" => {
            println!("{DIM}Bye! {}{RESET}", tracker.summary());
            return Some("quit");
        }
        "/clear" => {
            let system_msg = messages.first().cloned();
            messages.clear();
            if let Some(sys) = system_msg {
                messages.push(sys);
            }
            println!("{GREEN}Context cleared.{RESET}");
        }
        "/compact" => {
            let before = estimate_tokens(messages);
            compact_messages(messages, 40_000);
            let after = estimate_tokens(messages);
            println!("{GREEN}Compacted: ~{before} -> ~{after} tokens{RESET}");
        }
        "/tokens" => {
            let est = estimate_tokens(messages);
            println!("{DIM}Session: {}{RESET}", tracker.summary());
            println!("{DIM}Context: ~{est} tokens in {} messages{RESET}", messages.len());
        }
        "/tools" => {
            let mut cats: HashMap<String, Vec<String>> = HashMap::new();
            for s in tool_schemas {
                if let Some(name) = s
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                {
                    let cat = name.split('_').next().unwrap_or("other").to_string();
                    cats.entry(cat).or_default().push(name.to_string());
                }
            }
            for (cat, tools) in &cats {
                println!("  {BOLD}{cat}/{RESET} ({})", tools.len());
                for t in tools {
                    println!("    {DIM}{t}{RESET}");
                }
            }
        }
        "/session" => {
            println!("  {DIM}ID: {session_id}{RESET}");
            if let Some(m) = storage.read_manifest(session_id) {
                println!("  {DIM}Messages: {} | Checkpoints: {}{RESET}", m.message_count, m.checkpoint_count);
            }
        }
        "/sessions" => {
            let sessions = storage.list_sessions(15);
            if sessions.is_empty() {
                println!("  {DIM}No saved sessions{RESET}");
            } else {
                for s in &sessions {
                    let icon = if s.status == "active" { "\u{25CF}" } else { "\u{25CB}" };
                    let current = if s.id == session_id { " <- current" } else { "" };
                    println!(
                        "  {icon} {BOLD}{}{RESET} {DIM}{}/{} | {} msgs{current}{RESET}",
                        s.id, s.provider, s.model, s.message_count
                    );
                }
            }
        }
        "/checkpoint" => {
            let label = if parts.len() > 1 { parts[1].trim() } else { "manual" };
            let cp = storage.create_checkpoint(session_id, messages.len(), label);
            println!("  {GREEN}Checkpoint created: {} ({label}) at message {}{RESET}", cp.id, cp.message_index);
        }
        "/help" => {
            println!(
                "\n{BOLD}Commands:{RESET}\n\
  {BOLD}Session:{RESET}\n\
    /session       -- Current session info\n\
    /sessions      -- List all saved sessions\n\
    /checkpoint    -- Create a checkpoint\n\
\n\
  {BOLD}Context:{RESET}\n\
    /clear         -- Reset conversation (keep system prompt)\n\
    /compact       -- Force context compression\n\
    /tokens        -- Show token usage stats\n\
    /tools         -- List all available tools\n\
\n\
  {BOLD}Other:{RESET}\n\
    /quit          -- Save session and exit\n\
    /help          -- This message\n\
\n\
  {BOLD}Tip:{RESET} Press Ctrl+C while the LLM is thinking to interrupt.\n"
            );
        }
        _ => {
            println!("{YELLOW}Unknown command: {command}. Type /help{RESET}");
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Banner
// ---------------------------------------------------------------------------

const BANNER: &str = concat!(
    "\x1b[1m\x1b[35m\n",
    "  \u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2557}\n",
    "  \u{2551}         Mirai Code v0.1.0           \u{2551}\n",
    "  \u{2551}   Agentic coding in your terminal   \u{2551}\n",
    "  \u{255A}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255D}",
    "\x1b[0m\n"
);

// ---------------------------------------------------------------------------
// Main interactive session entry point
// ---------------------------------------------------------------------------

pub async fn run_interactive_session(config: SessionConfig) {
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    print!("{BANNER}");
    println!("  {DIM}Provider:{RESET} {BOLD}{}{RESET}", config.provider);
    println!("  {DIM}Model:{RESET}    {BOLD}{}{RESET}", config.model);
    println!("  {DIM}CWD:{RESET}      {BOLD}{cwd}{RESET}");
    println!();

    // Create adapter
    let adapter = adapter_factory::create_adapter(&config.provider, "", "");

    // Create registry and register all builtin tools
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);

    // Build tool schemas -- core for local, all for cloud
    let core_only = config.provider == "ollama";
    let tool_schemas = build_tool_schemas(&registry, core_only);
    let name_map = build_name_map(&registry);

    let mut tracker = TokenTracker::new();
    let storage = SessionStorage::new();
    let autonomy = get_autonomy(&config.autonomy_level);

    println!(
        "  {DIM}Tools loaded: {} {RESET}",
        tool_schemas.len()
    );
    println!(
        "  {DIM}Autonomy: {BOLD}{}{RESET} {DIM}(max {} rounds/turn){RESET}",
        autonomy.level, autonomy.max_tool_rounds
    );
    if let Some(ctx) = config.context_window {
        let label = if ctx >= 1_000_000 {
            format!("{}M", ctx / 1_000_000)
        } else {
            format!("{}K", ctx / 1000)
        };
        println!("  {DIM}Context window: {BOLD}{label}{RESET}");
    }

    let system_prompt = build_system_prompt(&cwd, &config.autonomy_level);

    // Create session
    let manifest = storage.create_session(&config.provider, &config.model, &cwd);
    let session_id = manifest.id.clone();
    println!("  {DIM}Session: {session_id}{RESET}");
    println!("  {DIM}Type /help for commands, Ctrl+C while thinking to interrupt{RESET}");
    println!();

    let mut messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];

    // Main input loop
    loop {
        print!("\n{BOLD}{BLUE}>{RESET} ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) | Err(_) => {
                // EOF or error
                storage.close_session(&session_id);
                println!("\n{DIM}Session saved: {session_id}{RESET}");
                println!("{DIM}Bye! {}{RESET}", tracker.summary());
                break;
            }
            Ok(_) => {}
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle Ctrl+C
        if trimmed == "\x03" {
            storage.close_session(&session_id);
            println!("\n{DIM}Session saved: {session_id}{RESET}");
            println!("{DIM}Bye! {}{RESET}", tracker.summary());
            break;
        }

        // Slash commands
        if trimmed.starts_with('/') {
            if let Some("quit") = handle_slash(
                trimmed,
                &mut messages,
                &tracker,
                &tool_schemas,
                &storage,
                &session_id,
            ) {
                storage.close_session(&session_id);
                println!("{DIM}Session saved: {session_id}{RESET}");
                break;
            }
            continue;
        }

        // Add user message
        messages.push(json!({
            "role": "user",
            "content": trimmed,
        }));
        storage.append_user_message(&session_id, trimmed);

        // Compact if needed
        compact_messages(&mut messages, 80_000);

        // Run agentic loop
        match agentic_loop(
            adapter.as_ref(),
            &config.model,
            &mut messages,
            &tool_schemas,
            &name_map,
            &registry,
            &cwd,
            &mut tracker,
            &autonomy,
            config.temperature,
            config.max_tokens,
            config.context_window,
            &storage,
            &session_id,
        )
        .await
        {
            Ok((response_text, was_streamed)) => {
                if !response_text.is_empty() {
                    messages.push(json!({
                        "role": "assistant",
                        "content": response_text,
                    }));
                    storage.append_assistant_message(&session_id, &response_text);
                    if !was_streamed {
                        println!("\n{response_text}");
                    } else {
                        println!(); // newline after streamed output
                    }
                }
            }
            Err(e) => {
                if e.contains("timeout") {
                    println!(
                        "\n{RED}Error: Model timed out. Try a smaller model or reduce context window.{RESET}"
                    );
                } else if e.contains("connect") || e.contains("refused") {
                    println!(
                        "\n{RED}Error: Cannot connect to provider. Is it running?{RESET}"
                    );
                } else {
                    println!("\n{RED}Error: {e}{RESET}");
                }
                let err_msg = format!("(error: {e})");
                messages.push(json!({"role": "assistant", "content": err_msg}));
                storage.append_assistant_message(&session_id, &err_msg);
            }
        }
    }
}
