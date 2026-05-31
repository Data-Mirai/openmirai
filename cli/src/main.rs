//! Mirai CLI — entry point.
//!
//! Subcommands:
//!   mirai              → interactive setup wizard + terminal (default)
//!   mirai run <file>   → execute agent from JSON/YAML file
//!   mirai validate <f> → validate agent spec without running
//!   mirai serve        → start HTTP server
//!   mirai version      → show version
//!   mirai agent load   → import agent from YAML
//!   mirai agent list   → list agents

mod adapter_factory;
mod colors;
mod session_storage;
mod setup_wizard;
mod terminal;

use std::process;
use std::sync::Arc;

use openmirai_engine::tools::builtin::register_all_builtin_tools;
use openmirai_engine::{
    AdapterBridgeLLMResource, AgentSpec, DefaultExecutionContext, ExecutionContext,
    ExecutionStatus, GraphRunner, RegistryExecutor, ToolRegistry,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(|s| s.as_str()) {
        None => run_default().await,
        Some("version" | "--version" | "-V") => {
            println!("mirai {VERSION}");
        }
        Some("run") => {
            let rest = &args[1..];
            run_agent(rest).await;
        }
        Some("validate") => {
            let rest = &args[1..];
            validate_agent(rest);
        }
        Some("serve") => {
            let rest = &args[1..];
            run_serve(rest).await;
        }
        Some("tools") => cmd_tools(&args[1..]),
        Some("templates") => cmd_templates(&args[1..]),
        Some("new") => cmd_new(&args[1..]),
        Some("describe") => cmd_describe(&args[1..]),
        Some("eval") => cmd_eval(&args[1..]).await,
        Some("rag") => cmd_rag(&args[1..]).await,
        Some("agent") => handle_agent_subcommand(&args[1..]),
        Some("help" | "--help" | "-h") => print_help(),
        Some(other) => {
            eprintln!(
                "{}Unknown command: {other}. Run `mirai help` for usage.{}",
                colors::RED,
                colors::RESET
            );
            process::exit(1);
        }
    }
}

/// Default: run setup wizard, then interactive terminal.
async fn run_default() {
    let config = match setup_wizard::run_setup_wizard(false, "", "", "").await {
        Some(c) => c,
        None => {
            println!("\n{}Setup cancelled.{}", colors::DIM, colors::RESET);
            process::exit(0);
        }
    };

    terminal::run_interactive_session(config).await;
}

/// Parse a named flag from args: `--flag value` → Some(value)
fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find_map(|w| {
        if w[0] == flag {
            Some(w[1].clone())
        } else {
            None
        }
    })
}

/// Check if a boolean flag is present: `--benchmark` → true
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Auto-detect provider from model name.
fn detect_provider_from_model(model: &str) -> Option<&'static str> {
    let m = model.to_lowercase();
    if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        Some("openai")
    } else if m.starts_with("claude") {
        Some("claude")
    } else if m.starts_with("gemini") || m.starts_with("gemma") {
        Some("gemini")
    } else if m.contains("llama")
        || m.contains("mistral")
        || m.contains("qwen")
        || m.contains("phi")
        || m.contains("deepseek")
    {
        // Common open models → likely Ollama
        Some("ollama")
    } else {
        None
    }
}

/// Resolve LLM provider: explicit flag > env var > model auto-detect > default ollama.
fn resolve_provider(args: &[String]) -> (String, String, String, String) {
    let explicit_provider = parse_flag(args, "--provider");
    let explicit_model = parse_flag(args, "--model");
    let explicit_key = parse_flag(args, "--api-key");
    let explicit_url = parse_flag(args, "--base-url");

    let env_provider = std::env::var("MIRAI_LLM_PROVIDER").ok();
    let env_model = std::env::var("MIRAI_LLM_MODEL").ok();

    // Provider resolution chain
    let provider = explicit_provider
        .or(env_provider)
        .or_else(|| {
            explicit_model
                .as_deref()
                .or(env_model.as_deref())
                .and_then(detect_provider_from_model)
                .map(String::from)
        })
        .unwrap_or_else(|| "ollama".to_string());

    let model = explicit_model
        .or(env_model)
        .unwrap_or_else(|| adapter_factory::default_model(&provider).to_string());

    let api_key = explicit_key.unwrap_or_default();
    let base_url = explicit_url.unwrap_or_default();

    (provider, model, api_key, base_url)
}

/// Build the ExecutionContext with a real or mock LLM depending on flags.
fn build_context(
    provider: &str,
    model: &str,
    api_key: &str,
    base_url: &str,
    system_prompt: Option<&str>,
) -> DefaultExecutionContext {
    use openmirai_engine::adapters::InMemoryDBResource;
    use openmirai_engine::adapters::InMemoryStorageResource;
    use openmirai_engine::adapters::MockLLMResource;

    // Special case: if provider is "mock", use MockLLMResource for testing
    let llm: Box<dyn openmirai_engine::LLMResource> = if provider == "mock" {
        Box::new(MockLLMResource::new())
    } else {
        let adapter = adapter_factory::create_adapter(provider, api_key, base_url);
        let bridge = AdapterBridgeLLMResource::new(adapter, model);
        let bridge = match provider {
            "openai" => {
                let url = if base_url.is_empty() {
                    "https://api.openai.com/v1".to_string()
                } else {
                    base_url.to_string()
                };
                bridge.with_embed(url, api_key)
            }
            _ => bridge,
        };
        Box::new(bridge)
    };

    let mut builder = DefaultExecutionContext::builder(llm)
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()));

    if let Some(prompt) = system_prompt {
        builder = builder.with_system_prompt(prompt);
    }

    builder.build()
}

/// Execute an agent from a JSON/YAML file.
///
/// Usage: mirai run <agent.yaml> [--input '{"key":"value"}'] [--provider ollama] [--model gemma3]
///
/// Output: JSON with status, state, trace, transcript.
async fn run_agent(args: &[String]) {
    let path = match args.first() {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!(
                "{}Usage: mirai run <agent.yaml> [--input '{{...}}'] [--provider <p>] [--model <m>]{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    };

    // Parse flags
    let input_json = parse_flag(args, "--input").or_else(|| parse_flag(args, "-i"));
    let (provider, model, api_key, base_url) = resolve_provider(args);

    // Benchmark setup
    let benchmark_enabled = has_flag(args, "--benchmark")
        || std::env::var("MIRAI_BENCHMARK")
            .map(|v| v == "1")
            .unwrap_or(false);
    if benchmark_enabled {
        let bench_path = std::env::var("MIRAI_BENCHMARK_FILE")
            .unwrap_or_else(|_| "benchmarks.jsonl".to_string());
        openmirai_engine::benchmark::mark_process_start();
        openmirai_engine::benchmark::enable(bench_path);
    }

    // Show provider info
    eprintln!(
        "{}LLM: {provider}/{model}{}{}",
        colors::DIM,
        if benchmark_enabled {
            " [benchmark]"
        } else {
            ""
        },
        colors::RESET
    );

    // Load agent spec
    let spec = match AgentSpec::from_file(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}Error loading agent spec: {e}{}",
                colors::RED,
                colors::RESET
            );
            process::exit(1);
        }
    };

    // PRD-008: Reject live agents — they must be started with `mirai play`
    if spec.agent_type == openmirai_engine::AgentType::Live {
        eprintln!(
            "{}Error: live agents must be started with `mirai play`, not `mirai run`{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    }

    // Convert to graph
    let mut graph = spec.to_graph(Some(&spec.name));
    graph.auto_generate_edge_ids();
    if let Err(e) = graph.validate() {
        eprintln!(
            "{}Graph validation failed: {e}{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    }

    // Create registry with all builtins
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);

    // Create runner
    let executor = RegistryExecutor::new(Arc::new(registry));
    let runner = GraphRunner::new(Box::new(executor));

    // Resolve Soul if specified → system prompt
    let system_prompt = if let Some(ref soul_path) = spec.soul {
        match openmirai_engine::soul::load_from_file(std::path::Path::new(soul_path)) {
            Ok(soul) => {
                eprintln!("{}Soul: {}{}", colors::DIM, soul.name, colors::RESET);
                Some(soul.to_system_prompt())
            }
            Err(e) => {
                eprintln!(
                    "{}Warning: failed to load soul '{}': {}{}",
                    colors::YELLOW,
                    soul_path,
                    e,
                    colors::RESET
                );
                spec.system_prompt.clone()
            }
        }
    } else {
        spec.system_prompt.clone()
    };

    // Create context with LLM provider + system prompt
    let context = build_context(
        &provider,
        &model,
        &api_key,
        &base_url,
        system_prompt.as_deref(),
    );

    // If input provided, validate against spec.inputs and inject into trigger node
    if let Some(ref input_str) = input_json {
        if let Ok(input_value) = serde_json::from_str::<serde_json::Value>(input_str) {
            // Convert Value to HashMap for validation
            let payload: std::collections::HashMap<String, serde_json::Value> = match input_value {
                serde_json::Value::Object(map) => map.into_iter().collect(),
                _ => {
                    let mut m = std::collections::HashMap::new();
                    m.insert("_raw".to_string(), input_value);
                    m
                }
            };

            // Validate against spec.inputs if defined (PRD-004 Capa 1)
            let validated_payload = if let Some(ref inputs_schema) = spec.inputs {
                match openmirai_engine::core::agent_spec::validate_agent_inputs(
                    &payload,
                    inputs_schema,
                ) {
                    Ok(enriched) => enriched,
                    Err(errors) => {
                        eprintln!(
                            "{}Error: input validation failed for agent '{}':{}",
                            colors::RED,
                            spec.name,
                            colors::RESET
                        );
                        for err in &errors {
                            eprintln!("  - {}", err);
                        }
                        std::process::exit(1);
                    }
                }
            } else {
                payload
            };

            // Inject validated payload into trigger node
            if let Some(entry) = graph
                .nodes
                .iter_mut()
                .find(|n| n.tool_type.starts_with("trigger/"))
            {
                let payload_value =
                    serde_json::to_value(&validated_payload).unwrap_or(serde_json::json!({}));
                entry.config.insert("payload".to_string(), payload_value);
            }
        }
    }

    // Inject model override into all LLM nodes if not already set
    for node in &mut graph.nodes {
        if node.tool_type == "ai/llm_call" {
            node.config
                .entry("model".to_string())
                .or_insert_with(|| serde_json::Value::String(model.clone()));
        }
    }

    // Inject MCP server configs into all mcp/call nodes
    if !spec.config.mcp_servers.is_empty() {
        let mcp_servers_val = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config
                    .insert("__mcp_servers".to_string(), mcp_servers_val.clone());
            }
        }
    }

    // Record cold start
    if benchmark_enabled {
        openmirai_engine::benchmark::record_cold_start();
    }
    let exec_start = std::time::Instant::now();

    // Run
    match runner.run(&graph, &context).await {
        Ok(result) => {
            let exec_ms = exec_start.elapsed().as_millis() as u64;
            let trace_enabled = has_flag(args, "--trace");

            // Print trace tree if --trace
            if trace_enabled {
                let tree = openmirai_engine::observability::build_trace_tree(&result, &spec.name);
                let rendered = openmirai_engine::observability::render_trace_tree(&tree);
                eprintln!("\n{}Trace:{}\n{}\n", colors::BOLD, colors::RESET, rendered);

                let metrics = openmirai_engine::observability::compute_metrics(&result.trace);
                eprintln!(
                    "{}Metrics:{} {} nodes, {}ms total, {:.1}ms avg, {} retries\n",
                    colors::BOLD,
                    colors::RESET,
                    metrics.total_nodes,
                    metrics.total_duration_ms,
                    metrics.avg_node_duration_ms,
                    metrics.total_retries,
                );
            }

            // Log benchmarks
            if benchmark_enabled {
                openmirai_engine::benchmark::log_execution(
                    exec_ms,
                    &spec.name,
                    graph.nodes.len(),
                    &provider,
                );
                // Log individual node latencies from trace
                for entry in &result.trace {
                    openmirai_engine::benchmark::log_tool_latency(
                        entry.duration_ms,
                        &entry.tool_type,
                        &entry.node_id,
                    );
                }
                openmirai_engine::benchmark::log_memory_usage();
            }

            let output = serde_json::json!({
                "status": result.status,
                "state": result.state.snapshot(),
                "trace": result.trace,
                "transcript": result.transcript,
                "error": result.error,
                "interrupt_info": result.interrupt_info,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());

            if result.status != ExecutionStatus::Completed {
                process::exit(1);
            }
        }
        Err(e) => {
            let output = serde_json::json!({
                "status": "failed",
                "error": e.to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
            process::exit(1);
        }
    }
}

/// Validate an agent spec without running it.
fn validate_agent(args: &[String]) {
    let path = match args.first() {
        Some(p) => p.as_str(),
        None => {
            eprintln!(
                "{}Usage: mirai validate <agent.yaml>{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    };

    match AgentSpec::from_file(path) {
        Ok(spec) => {
            let nodes = spec.graph.nodes.len();
            let edges = spec.graph.edges.len();
            println!(
                "{}✓ Valid agent spec: '{}' ({nodes} nodes, {edges} edges){}",
                colors::GREEN,
                spec.name,
                colors::RESET
            );
        }
        Err(e) => {
            eprintln!("{}✗ Invalid agent spec: {e}{}", colors::RED, colors::RESET);
            process::exit(1);
        }
    }
}

/// Start the HTTP server.
async fn run_serve(args: &[String]) {
    let port: u16 = parse_flag(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let host = parse_flag(args, "--host").unwrap_or_else(|| "0.0.0.0".to_string());

    // Resolve provider using the same chain as `mirai run`.
    let (provider, model, api_key, base_url) = resolve_provider(args);

    eprintln!(
        "{}Starting openmirai-engine server on {}:{}{}",
        colors::GREEN,
        host,
        port,
        colors::RESET
    );
    eprintln!(
        "{}LLM: {provider}/{model} (REAL — zero mocks){}",
        colors::DIM,
        colors::RESET
    );

    // Build a factory that creates REAL LLM resources for each request.
    let llm_factory: std::sync::Arc<
        dyn Fn() -> Box<dyn openmirai_engine::LLMResource> + Send + Sync,
    > = {
        let provider = provider.clone();
        let model = model.clone();
        let api_key = api_key.clone();
        let base_url = base_url.clone();
        std::sync::Arc::new(move || {
            if provider == "mock" {
                // Only allowed in explicit --provider mock for testing
                Box::new(openmirai_engine::MockLLMResource::new())
            } else {
                let adapter = adapter_factory::create_adapter(&provider, &api_key, &base_url);
                Box::new(AdapterBridgeLLMResource::new(adapter, &model))
            }
        })
    };

    // Read API key from env or flag.
    let server_api_key =
        parse_flag(args, "--api-key").or_else(|| std::env::var("MIRAI_API_KEY").ok());

    if let Err(e) = openmirai_engine::server::serve(&host, port, llm_factory, server_api_key).await
    {
        eprintln!("{}Server error: {e}{}", colors::RED, colors::RESET);
        process::exit(1);
    }
}

/// List all tools or show details for a specific tool.
///
/// `mirai tools`                → list all tools grouped by category
/// `mirai tools ai/claude_code` → show inputs, outputs, config for that tool
fn cmd_tools(args: &[String]) {
    let mut registry = openmirai_engine::ToolRegistry::new();
    openmirai_engine::tools::builtin::register_all_builtin_tools(&mut registry);

    let specs = registry.list_tools();

    match args.first().map(|s| s.as_str()) {
        // Detail view: mirai tools <tool_type>
        Some(tool_type) => {
            let spec = match specs.iter().find(|s| s.tool_type == tool_type) {
                Some(s) => s,
                None => {
                    eprintln!(
                        "{}Tool not found: {tool_type}{}",
                        colors::RED,
                        colors::RESET
                    );
                    eprintln!("Run `mirai tools` to see all available tools.");
                    process::exit(1);
                }
            };

            println!(
                "\n{}{}  —  {}{}",
                colors::BOLD,
                spec.tool_type,
                spec.name,
                colors::RESET
            );
            println!("{}", spec.description);

            if !spec.inputs.is_empty() {
                println!(
                    "\n{}INPUTS (what this node receives via data_map):{}",
                    colors::GREEN,
                    colors::RESET
                );
                for f in &spec.inputs {
                    let req = if f.required { "required" } else { "optional" };
                    let desc = f.description.as_deref().unwrap_or("");
                    println!("  {:<20} {:<10} {:<10} {}", f.name, f.field_type, req, desc);
                }
            }

            if !spec.outputs.is_empty() {
                println!(
                    "\n{}OUTPUTS (what this node produces — use in data_map of next edge):{}",
                    colors::CYAN,
                    colors::RESET
                );
                for f in &spec.outputs {
                    let desc = f.description.as_deref().unwrap_or("");
                    println!("  {:<20} {:<10} {}", f.name, f.field_type, desc);
                }
            }

            if !spec.config_fields.is_empty() {
                println!(
                    "\n{}CONFIG (set in the node's config section):{}",
                    colors::DIM,
                    colors::RESET
                );
                for f in &spec.config_fields {
                    let req = if f.required { "required" } else { "optional" };
                    let desc = f.description.as_deref().unwrap_or("");
                    let default = f
                        .default
                        .as_ref()
                        .map(|d| format!(" (default: {})", d))
                        .unwrap_or_default();
                    println!(
                        "  {:<20} {:<10} {:<10} {}{}",
                        f.name, f.field_type, req, desc, default
                    );
                }
            }

            println!();
        }
        // List view: mirai tools
        None => {
            // Group by category.
            let mut by_category: std::collections::BTreeMap<
                String,
                Vec<&openmirai_engine::ToolSpec>,
            > = std::collections::BTreeMap::new();
            for spec in &specs {
                by_category
                    .entry(spec.category.clone())
                    .or_default()
                    .push(spec);
            }

            println!(
                "\n{}Available tools ({}):{}\n",
                colors::BOLD,
                specs.len(),
                colors::RESET
            );

            for (category, tools) in &by_category {
                println!("{}{}:{}", colors::GREEN, category, colors::RESET);
                for tool in tools {
                    println!(
                        "  {:<30} {}",
                        tool.tool_type,
                        tool.description.chars().take(60).collect::<String>()
                    );
                }
                println!();
            }

            println!(
                "{}Tip:{} run `mirai tools <tool_type>` to see inputs, outputs, and config.",
                colors::DIM,
                colors::RESET
            );
            println!("     Example: mirai tools ai/claude_code\n");
        }
    }
}

/// Describe agent contract — inputs/outputs (PRD-004).
fn cmd_describe(args: &[String]) {
    let path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!(
                "{}Usage: mirai describe <agent.yaml>{}",
                colors::RED,
                colors::RESET
            );
            process::exit(1);
        }
    };

    let spec = match AgentSpec::from_file(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}Error: {}{}", colors::RED, e, colors::RESET);
            process::exit(1);
        }
    };

    println!(
        "\n{}Agent:{} {} ({})",
        colors::BOLD,
        colors::RESET,
        spec.name,
        spec.version
    );
    if !spec.description.is_empty() {
        println!("{}", spec.description);
    }

    if let Some(ref inputs) = spec.inputs {
        println!("\n{}Inputs:{}", colors::BOLD, colors::RESET);
        for (name, field) in inputs {
            let req = if field.required {
                "required"
            } else {
                "optional"
            };
            let desc = if field.description.is_empty() {
                String::new()
            } else {
                format!("  {}", field.description)
            };
            println!("  {}  {}  {}{}", name, field.field_type, req, desc);
        }
    } else {
        println!(
            "\n{}Inputs:{} (none declared — accepts any payload)",
            colors::BOLD,
            colors::RESET
        );
    }

    if let Some(ref outputs) = spec.outputs {
        println!("\n{}Outputs:{}", colors::BOLD, colors::RESET);
        for (name, field) in outputs {
            let desc = if field.description.is_empty() {
                String::new()
            } else {
                format!("  {}", field.description)
            };
            println!("  {}  {}{}", name, field.field_type, desc);
        }
    } else {
        println!(
            "\n{}Outputs:{} (none declared)",
            colors::BOLD,
            colors::RESET
        );
    }
    println!();
}

/// List available agent templates.
fn cmd_templates(_args: &[String]) {
    let templates = openmirai_engine::templates::builtin_templates();
    println!(
        "{bold}Available templates ({count}):{reset}\n",
        bold = colors::BOLD,
        count = templates.len(),
        reset = colors::RESET,
    );
    for t in &templates {
        println!(
            "  {green}{:<25}{reset} [{:?}] {}",
            t.id,
            t.category,
            t.description,
            green = colors::GREEN,
            reset = colors::RESET,
        );
    }
    println!(
        "\n{}Use: mirai new --template <id> --name <agent-name>{}",
        colors::DIM,
        colors::RESET,
    );
}

/// Create a new agent from a template.
fn cmd_new(args: &[String]) {
    let template_id = match parse_flag(args, "--template") {
        Some(t) => t,
        None => {
            eprintln!(
                "{}Usage: mirai new --template <template-id> --name <agent-name> [--provider <p>]{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    };

    let name = parse_flag(args, "--name").unwrap_or_else(|| template_id.clone());

    let template = match openmirai_engine::templates::get_template(&template_id) {
        Some(t) => t,
        None => {
            eprintln!(
                "{}Template '{}' not found. Run 'mirai templates' to see available templates.{}",
                colors::RED,
                template_id,
                colors::RESET
            );
            process::exit(1);
        }
    };

    // Clone spec and override name + provider.
    let mut spec = template.spec.clone();
    spec["name"] = serde_json::Value::String(name.clone());

    if let Some(provider) = parse_flag(args, "--provider") {
        // Inject provider/model into LLM nodes.
        if let Some(nodes) = spec["graph"]["nodes"].as_array_mut() {
            for node in nodes {
                if node["tool_type"].as_str() == Some("ai/llm_call") {
                    node["config"]["provider"] = serde_json::Value::String(provider.clone());
                }
            }
        }
    }

    // Write to file.
    let filename = format!("{name}.yaml");
    let yaml = serde_yaml::to_string(&spec)
        .unwrap_or_else(|_| serde_json::to_string_pretty(&spec).unwrap());

    match std::fs::write(&filename, &yaml) {
        Ok(_) => {
            println!(
                "{}✓ Created '{}' from template '{}'{}",
                colors::GREEN,
                filename,
                template_id,
                colors::RESET
            );
            println!(
                "{}Run it: mirai run {}{}",
                colors::DIM,
                filename,
                colors::RESET
            );
        }
        Err(e) => {
            eprintln!(
                "{}Error writing {}: {e}{}",
                colors::RED,
                filename,
                colors::RESET
            );
            process::exit(1);
        }
    }
}

/// Evaluate a session or input/output pair.
async fn cmd_eval(args: &[String]) {
    let input = parse_flag(args, "--input").unwrap_or_default();
    let output = parse_flag(args, "--output").unwrap_or_default();
    let types_str =
        parse_flag(args, "--types").unwrap_or_else(|| "format_compliance,latency".to_string());

    if output.is_empty() {
        eprintln!(
            "{}Usage: mirai eval --input <text> --output <text> --types relevance,format_compliance{}",
            colors::YELLOW, colors::RESET
        );
        process::exit(1);
    }

    let eval_types: Vec<openmirai_engine::eval::EvalType> = types_str
        .split(',')
        .filter_map(|s| match s.trim() {
            "relevance" => Some(openmirai_engine::eval::EvalType::Relevance),
            "faithfulness" => Some(openmirai_engine::eval::EvalType::Faithfulness),
            "completeness" => Some(openmirai_engine::eval::EvalType::Completeness),
            "format_compliance" => Some(openmirai_engine::eval::EvalType::FormatCompliance),
            "latency" => Some(openmirai_engine::eval::EvalType::Latency),
            _ => None,
        })
        .collect();

    let (provider, model, api_key, base_url) = resolve_provider(args);
    let context = build_context(&provider, &model, &api_key, &base_url, None);

    let results = openmirai_engine::eval::execute_eval(
        &eval_types,
        openmirai_engine::eval::EvalInput {
            input: &input,
            output: &output,
            context: None,
        },
        0,
        None,
        context.llm(),
        "",
    )
    .await;

    println!("{}Eval Results:{}", colors::BOLD, colors::RESET);
    for r in &results {
        let bar = "=".repeat((r.score * 20.0) as usize);
        println!(
            "  {:20} [{:<20}] {:.2}  {}",
            format!("{:?}", r.eval_type),
            bar,
            r.score,
            r.details.as_deref().unwrap_or(""),
        );
    }
}

/// RAG search from CLI.
async fn cmd_rag(args: &[String]) {
    let sub = args.first().map(|s| s.as_str());
    match sub {
        Some("search") => {
            let query = parse_flag(args, "--query").unwrap_or_default();
            let docs_str = parse_flag(args, "--documents").unwrap_or_default();
            let top_k: usize = parse_flag(args, "--top-k")
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);

            if query.is_empty() || docs_str.is_empty() {
                eprintln!(
                    "{}Usage: mirai rag search --query <text> --documents <file1,file2,...> --top-k <n>{}",
                    colors::YELLOW, colors::RESET
                );
                process::exit(1);
            }

            // Read documents from file paths.
            let mut documents = Vec::new();
            for path in docs_str.split(',') {
                let path = path.trim();
                match std::fs::read_to_string(path) {
                    Ok(content) => documents.push(content),
                    Err(e) => eprintln!(
                        "{}Warning: could not read {}: {}{}",
                        colors::YELLOW,
                        path,
                        e,
                        colors::RESET
                    ),
                }
            }

            if documents.is_empty() {
                eprintln!("{}No documents loaded{}", colors::RED, colors::RESET);
                process::exit(1);
            }

            let (provider, model, api_key, base_url) = resolve_provider(args);
            let context = build_context(&provider, &model, &api_key, &base_url, None);

            // Chunk + embed + search.
            let rag_config = openmirai_engine::rag::RAGPipelineConfig {
                name: "cli".into(),
                source_type: openmirai_engine::rag::SourceType::Text,
                chunking_strategy: openmirai_engine::rag::ChunkingStrategy::Paragraph,
                chunk_size: 512,
                chunk_overlap: 50,
                embedding_model: String::new(),
            };

            let mut all_chunks: Vec<String> = Vec::new();
            for doc in &documents {
                all_chunks.extend(openmirai_engine::rag::chunk_text(doc, &rag_config));
            }

            eprintln!(
                "{}Chunks: {} | Embedding...{}",
                colors::DIM,
                all_chunks.len(),
                colors::RESET
            );

            let query_emb = match context.llm().embed(&query, "").await {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{}Embedding error: {e}{}", colors::RED, colors::RESET);
                    process::exit(1);
                }
            };

            let mut scored = Vec::new();
            for (i, chunk) in all_chunks.iter().enumerate() {
                if let Ok(emb) = context.llm().embed(chunk, "").await {
                    let dot: f64 = query_emb.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
                    let mag_a: f64 = query_emb.iter().map(|x| x * x).sum::<f64>().sqrt();
                    let mag_b: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();
                    let sim = if mag_a > 0.0 && mag_b > 0.0 {
                        dot / (mag_a * mag_b)
                    } else {
                        0.0
                    };
                    scored.push((i, sim));
                }
            }
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            println!("{}Results (top {}):{}", colors::BOLD, top_k, colors::RESET);
            for (i, score) in scored.iter().take(top_k) {
                println!(
                    "  [{:.4}] {}...",
                    score,
                    &all_chunks[*i][..80.min(all_chunks[*i].len())]
                );
            }
        }
        _ => {
            eprintln!(
                "{}Usage: mirai rag search --query <text> --documents <paths>{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    }
}

fn handle_agent_subcommand(args: &[String]) {
    match args.first().map(|s| s.as_str()) {
        Some("load") => {
            let path = args.get(1).map(|s| s.as_str()).unwrap_or("");
            if path.is_empty() {
                eprintln!(
                    "{}Usage: mirai agent load <path.yaml>{}",
                    colors::YELLOW,
                    colors::RESET
                );
                process::exit(1);
            }
            println!(
                "{}Agent loading not yet implemented. Path: {path}{}",
                colors::YELLOW,
                colors::RESET
            );
        }
        Some("list") => {
            println!(
                "{}Agent listing not yet implemented.{}",
                colors::YELLOW,
                colors::RESET
            );
        }
        _ => {
            eprintln!(
                "{}Usage: mirai agent <load|list>{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    }
}

fn print_help() {
    println!(
        "\
{bold}mirai{reset} — Agentic graph engine

{bold}USAGE:{reset}
    mirai                                    Interactive setup wizard + terminal
    mirai run <file> [options]               Execute agent from JSON/YAML file
    mirai validate <file>                    Validate agent spec
    mirai serve [--port N]                   Start HTTP server
    mirai version                            Show version
    mirai agent load <file>                  Import agent from YAML
    mirai agent list                         List agents
    mirai help                               This message

{bold}RUN OPTIONS:{reset}
    -i, --input <JSON>       Input data for the agent
    --provider <name>        LLM provider: ollama, openai, claude, gemini, groq, openrouter, nvidia
    --model <name>           Model name (auto-detects provider if omitted)
    --api-key <key>          API key (or use env: OPENAI_API_KEY, ANTHROPIC_API_KEY, etc.)
    --base-url <url>         Custom API base URL

{bold}ENVIRONMENT VARIABLES:{reset}
    MIRAI_LLM_PROVIDER       Default LLM provider
    MIRAI_LLM_MODEL          Default model
    OPENAI_API_KEY            OpenAI API key
    ANTHROPIC_API_KEY         Anthropic (Claude) API key
    GROQ_API_KEY              Groq API key
    NVIDIA_API_KEY            NVIDIA API key
    OPENROUTER_API_KEY        OpenRouter API key
    OLLAMA_BASE_URL           Ollama server URL (default: http://localhost:11434)

{bold}PROVIDER RESOLUTION:{reset}
    1. --provider flag
    2. MIRAI_LLM_PROVIDER env var
    3. Auto-detect from model name (gpt-* → openai, claude-* → claude, etc.)
    4. Default: ollama (localhost)

{bold}EXAMPLES:{reset}
    mirai run agent.yaml
    mirai run agent.yaml --provider ollama --model gemma3
    mirai run agent.yaml --provider openai --model gpt-4o
    mirai run agent.yaml --input '{{\"query\": \"hello\"}}'
    mirai validate my-agent.yaml
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}
