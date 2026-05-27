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

use datamirai_engine::{
    AdapterBridgeLLMResource, AgentSpec, ExecutionStatus, GraphRunner,
    RegistryExecutor, SimpleExecutionContext, ToolRegistry,
};
use datamirai_engine::tools::builtin::register_all_builtin_tools;

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
    } else if m.contains("llama") || m.contains("mistral") || m.contains("qwen") || m.contains("phi") || m.contains("deepseek") {
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
fn build_context(provider: &str, model: &str, api_key: &str, base_url: &str) -> SimpleExecutionContext {
    use datamirai_engine::resources::InMemoryDBResource;
    use datamirai_engine::resources::InMemoryStorageResource;

    // Special case: if provider is "mock", use MockLLMResource for testing
    if provider == "mock" {
        return SimpleExecutionContext::default_dev();
    }

    // For Ollama, we can use either the direct OllamaLLMResource or the bridge.
    // Using bridge for consistency across all providers.
    let adapter = adapter_factory::create_adapter(provider, api_key, base_url);
    let bridge = AdapterBridgeLLMResource::new(adapter, model);

    // If provider supports OpenAI-compatible embeddings, configure embed
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

    SimpleExecutionContext::builder(Box::new(bridge))
        .with_db(Box::new(InMemoryDBResource::new()))
        .with_storage(Box::new(InMemoryStorageResource::new()))
        .build()
}

/// Execute an agent from a JSON/YAML file.
///
/// Usage: mirai run <agent.json|agent.yaml> [--input '{"key":"value"}'] [--provider ollama] [--model gemma3]
///
/// Output: JSON with status, state, trace, transcript.
async fn run_agent(args: &[String]) {
    let path = match args.first() {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!(
                "{}Usage: mirai run <agent.json|agent.yaml> [--input '{{...}}'] [--provider <p>] [--model <m>]{}",
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
        || std::env::var("MIRAI_BENCHMARK").map(|v| v == "1").unwrap_or(false);
    if benchmark_enabled {
        let bench_path = std::env::var("MIRAI_BENCHMARK_FILE")
            .unwrap_or_else(|_| "benchmarks.jsonl".to_string());
        datamirai_engine::benchmark::mark_process_start();
        datamirai_engine::benchmark::enable(bench_path);
    }

    // Show provider info
    eprintln!(
        "{}LLM: {provider}/{model}{}{}",
        colors::DIM,
        if benchmark_enabled { " [benchmark]" } else { "" },
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

    // Create context with REAL LLM provider
    let context = build_context(&provider, &model, &api_key, &base_url);

    // If input provided, inject into entry node state
    if let Some(ref input_str) = input_json {
        if let Ok(input_value) = serde_json::from_str::<serde_json::Value>(input_str) {
            if let Some(entry) = graph.nodes.iter_mut().find(|n| n.tool_type.starts_with("trigger/")) {
                entry.config.insert(
                    "mock_payload".to_string(),
                    input_value,
                );
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

    // Record cold start
    if benchmark_enabled {
        datamirai_engine::benchmark::record_cold_start();
    }
    let exec_start = std::time::Instant::now();

    // Run
    match runner.run(&graph, &context).await {
        Ok(result) => {
            let exec_ms = exec_start.elapsed().as_millis() as u64;

            // Log benchmarks
            if benchmark_enabled {
                datamirai_engine::benchmark::log_execution(
                    exec_ms,
                    &spec.name,
                    graph.nodes.len(),
                    &provider,
                );
                // Log individual node latencies from trace
                for entry in &result.trace {
                    datamirai_engine::benchmark::log_tool_latency(
                        entry.duration_ms,
                        &entry.tool_type,
                        &entry.node_id,
                    );
                }
                datamirai_engine::benchmark::log_memory_usage();
            }

            let output = serde_json::json!({
                "status": result.status,
                "state": result.state.snapshot(),
                "trace": result.trace,
                "transcript": result.transcript,
                "error": result.error,
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
                "{}Usage: mirai validate <agent.json|agent.yaml>{}",
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
            eprintln!(
                "{}✗ Invalid agent spec: {e}{}",
                colors::RED,
                colors::RESET
            );
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

    eprintln!(
        "{}Starting datamirai-engine server on {}:{}{}",
        colors::GREEN,
        host,
        port,
        colors::RESET
    );

    if let Err(e) = datamirai_engine::server::app::serve(&host, port).await {
        eprintln!(
            "{}Server error: {e}{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
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
    mirai validate my-agent.json
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}
