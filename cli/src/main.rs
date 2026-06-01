//! OpenMirai CLI — entry point.
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
        Some("edit") => run_edit(&args[1..]).await,
        Some("doctor") => cmd_doctor(&args[1..]).await,
        Some("models") => cmd_models(&args[1..]).await,
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

    // PRD-014: the developer's persisted choice (~/.openmirai/config.toml).
    let cfg = openmirai_engine::config::UserConfig::load();

    // Provider: flag > env > model auto-detect > config default.
    let provider = explicit_provider
        .or(env_provider)
        .or_else(|| {
            explicit_model
                .as_deref()
                .or(env_model.as_deref())
                .and_then(detect_provider_from_model)
                .map(String::from)
        })
        .unwrap_or_else(|| cfg.default_provider.clone());

    // Model: flag > env > config default (same provider) > provider fallback.
    let model = explicit_model.or(env_model).unwrap_or_else(|| {
        if provider == cfg.default_provider {
            cfg.default_model.clone()
        } else {
            adapter_factory::default_model(&provider).to_string()
        }
    });

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

    // PRD-014: preflight — ensure the provider/model is ready, guide if not.
    if !run_preflight(&provider, &model, &api_key, &base_url).await {
        eprintln!(
            "{}Run aborted: provider not ready. Run `mirai doctor` for the full check.{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    }

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
    let llm_factory = build_llm_factory(
        provider.clone(),
        model.clone(),
        api_key.clone(),
        base_url.clone(),
    );

    // Read API key from env or flag.
    let server_api_key =
        parse_flag(args, "--api-key").or_else(|| std::env::var("MIRAI_API_KEY").ok());

    if let Err(e) = openmirai_engine::server::serve(&host, port, llm_factory, server_api_key).await
    {
        eprintln!("{}Server error: {e}{}", colors::RED, colors::RESET);
        process::exit(1);
    }
}

/// Build a factory that creates a REAL LLM resource per request. A mock is only
/// returned when `--provider mock` is set explicitly. Shared by `serve` and `edit`.
fn build_llm_factory(
    provider: String,
    model: String,
    api_key: String,
    base_url: String,
) -> std::sync::Arc<dyn Fn() -> Box<dyn openmirai_engine::LLMResource> + Send + Sync> {
    std::sync::Arc::new(move || {
        if provider == "mock" {
            Box::new(openmirai_engine::MockLLMResource::new())
        } else {
            let adapter = adapter_factory::create_adapter(&provider, &api_key, &base_url);
            Box::new(AdapterBridgeLLMResource::new(adapter, &model))
        }
    })
}

/// `mirai edit <archivo.yaml>` — abre el mini-IDE visual en el navegador (PRD-013).
async fn run_edit(args: &[String]) {
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => {
            eprintln!(
                "{}Uso: mirai edit <archivo.yaml>{}",
                colors::RED,
                colors::RESET
            );
            process::exit(1);
        }
    };

    let port: u16 = parse_flag(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(4317);

    let (provider, model, api_key, base_url) = resolve_provider(args);

    // Real LLM factory (for the run/test feature). NO mocks unless --provider mock.
    let llm_factory = build_llm_factory(
        provider.clone(),
        model.clone(),
        api_key.clone(),
        base_url.clone(),
    );

    let url = format!("http://127.0.0.1:{port}");
    eprintln!(
        "{}OpenMirai editor{} → {}{url}{}  (archivo: {path})",
        colors::BOLD,
        colors::RESET,
        colors::GREEN,
        colors::RESET
    );
    eprintln!("{}Ctrl-C para salir{}", colors::DIM, colors::RESET);
    open_browser(&url);

    let cfg = openmirai_engine::config::UserConfig::load();
    let run_ctx = openmirai_engine::server::editor::RunCtx {
        ollama_host: ollama_host(&base_url, &cfg),
        has_key: provider_has_key(&provider, &api_key),
        provider,
        model,
    };
    if let Err(e) = openmirai_engine::server::editor::serve_editor(
        "127.0.0.1",
        port,
        llm_factory,
        &path,
        run_ctx,
    )
    .await
    {
        eprintln!("{}Editor error: {e}{}", colors::RED, colors::RESET);
        process::exit(1);
    }
}

/// Best-effort: abre el navegador por defecto en `url`.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
}

// ---------------------------------------------------------------------------
// PRD-014: provider readiness — preflight, doctor, models
// ---------------------------------------------------------------------------

/// Ollama host: `--base-url` > config (if non-default) > `OLLAMA_BASE_URL` > localhost.
fn ollama_host(base_url: &str, cfg: &openmirai_engine::config::UserConfig) -> String {
    if !base_url.is_empty() {
        return base_url.to_string();
    }
    if cfg.ollama_host != "http://localhost:11434" {
        return cfg.ollama_host.clone();
    }
    std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| cfg.ollama_host.clone())
}

/// Whether a cloud provider has an API key available (flag or env).
fn provider_has_key(provider: &str, api_key: &str) -> bool {
    if !api_key.is_empty() {
        return true;
    }
    let env_name = match provider {
        "claude" | "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GOOGLE_API_KEY",
        "groq" => "GROQ_API_KEY",
        "nvidia" => "NVIDIA_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => return true, // ollama / unknown → not key-gated here
    };
    std::env::var(env_name)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Run preflight; print actionable guidance and, on a TTY, offer to pull a
/// missing Ollama model. Returns true if it's OK to proceed.
async fn run_preflight(provider: &str, model: &str, api_key: &str, base_url: &str) -> bool {
    if provider == "mock" {
        return true;
    }
    let cfg = openmirai_engine::config::UserConfig::load();
    let host = ollama_host(base_url, &cfg);
    let has_key = provider_has_key(provider, api_key);

    let pf = openmirai_engine::preflight::check(provider, model, &host, has_key).await;
    if pf.ok {
        return true;
    }

    for d in &pf.diagnostics {
        eprintln!("{}✗ {}{}", colors::RED, d.message, colors::RESET);
        eprintln!("  {}→ {}{}", colors::DIM, d.action, colors::RESET);
    }

    let missing_model = provider == "ollama"
        && pf
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not downloaded"));
    if missing_model && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprint!(
            "\n  {}Download '{model}' now? [Y/n]: {}",
            colors::YELLOW,
            colors::RESET
        );
        let _ = std::io::Write::flush(&mut std::io::stderr());
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            let ans = line.trim().to_lowercase();
            let yes = ans.is_empty() || ans == "y" || ans == "yes" || ans == "s" || ans == "si";
            if yes && pull_with_progress(&host, model).await {
                let pf2 = openmirai_engine::preflight::check(provider, model, &host, has_key).await;
                return pf2.ok;
            }
        }
    }
    false
}

/// Pull an Ollama model, printing live progress. Returns true on success.
async fn pull_with_progress(host: &str, name: &str) -> bool {
    eprintln!("{}Pulling {name} …{}", colors::DIM, colors::RESET);
    let mut last = String::new();
    let res = openmirai_engine::preflight::pull_model(host, name, |status, completed, total| {
        let pct = match (completed, total) {
            (Some(c), Some(t)) if t > 0 => format!(" {}%", c * 100 / t),
            _ => String::new(),
        };
        let line = format!("{status}{pct}");
        if line != last {
            eprint!(
                "\r  {}{}{}                    ",
                colors::DIM,
                line,
                colors::RESET
            );
            let _ = std::io::Write::flush(&mut std::io::stderr());
            last = line;
        }
    })
    .await;
    eprintln!();
    match res {
        Ok(()) => {
            eprintln!("{}✓ {name} ready{}", colors::GREEN, colors::RESET);
            true
        }
        Err(e) => {
            eprintln!("{}✗ pull failed: {e}{}", colors::RED, colors::RESET);
            false
        }
    }
}

/// `mirai doctor` — check the environment and report exactly what's needed.
async fn cmd_doctor(_args: &[String]) {
    let cfg = openmirai_engine::config::UserConfig::load();
    println!("{}OpenMirai — doctor{}", colors::BOLD, colors::RESET);
    println!("  binary:  mirai {VERSION}");
    let cfg_loc = if openmirai_engine::config::UserConfig::exists() {
        openmirai_engine::config::UserConfig::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    } else {
        "(using defaults — no ~/.openmirai/config.toml yet)".to_string()
    };
    println!("  config:  {cfg_loc}");
    println!(
        "  default: {} / {}",
        cfg.default_provider, cfg.default_model
    );
    println!();

    let host = ollama_host("", &cfg);
    match openmirai_engine::preflight::installed_models(&host).await {
        Ok(models) => {
            println!(
                "  {}✓ Ollama{} reachable at {host} — {} model(s) installed",
                colors::GREEN,
                colors::RESET,
                models.len()
            );
            for m in &models {
                println!("      ● {m}");
            }
        }
        Err(_) => println!(
            "  {}✗ Ollama{} not reachable at {host} → start `ollama serve` (install: https://ollama.com)",
            colors::RED,
            colors::RESET
        ),
    }

    println!();
    for p in ["claude", "openai", "gemini", "groq", "nvidia", "openrouter"] {
        if provider_has_key(p, "") {
            println!("  {}✓{} {p} API key set", colors::GREEN, colors::RESET);
        } else {
            println!("  {}·{} {p} API key not set", colors::DIM, colors::RESET);
        }
    }

    println!();
    let pf = openmirai_engine::preflight::check(
        &cfg.default_provider,
        &cfg.default_model,
        &host,
        provider_has_key(&cfg.default_provider, ""),
    )
    .await;
    if pf.ok {
        println!(
            "{}Ready to run {} / {}{}",
            colors::GREEN,
            cfg.default_provider,
            cfg.default_model,
            colors::RESET
        );
    } else {
        println!("{}Default not ready:{}", colors::YELLOW, colors::RESET);
        for d in &pf.diagnostics {
            println!("  ✗ {} → {}", d.message, d.action);
        }
    }
}

/// `mirai models [list | pull <name> | use <name>]`
async fn cmd_models(args: &[String]) {
    let cfg = openmirai_engine::config::UserConfig::load();
    let host = ollama_host("", &cfg);

    match args.first().map(|s| s.as_str()) {
        Some("pull") => {
            let Some(name) = args.get(1) else {
                eprintln!("Usage: mirai models pull <name>");
                process::exit(1);
            };
            if !pull_with_progress(&host, name).await {
                process::exit(1);
            }
        }
        Some("use") => {
            let Some(name) = args.get(1) else {
                eprintln!("Usage: mirai models use <name>");
                process::exit(1);
            };
            let mut c = cfg.clone();
            c.default_provider = "ollama".to_string();
            c.default_model = name.to_string();
            match c.save() {
                Ok(p) => println!(
                    "{}✓ default → ollama / {name}{}  ({})",
                    colors::GREEN,
                    colors::RESET,
                    p.display()
                ),
                Err(e) => {
                    eprintln!("{}save failed: {e}{}", colors::RED, colors::RESET);
                    process::exit(1);
                }
            }
        }
        _ => {
            let installed = openmirai_engine::preflight::installed_models(&host)
                .await
                .unwrap_or_default();
            let is_installed = |name: &str| {
                installed.iter().any(|i| {
                    let i = i.as_str();
                    i == name || i == format!("{name}:latest").as_str()
                })
            };
            println!(
                "{}Curated Ollama models{}  (★ recommended · ● installed)",
                colors::BOLD,
                colors::RESET
            );
            for m in openmirai_engine::catalog::CATALOG {
                let star = if m.recommended { "★" } else { " " };
                let dot = if is_installed(m.name) {
                    format!("{}●{}", colors::GREEN, colors::RESET)
                } else {
                    " ".to_string()
                };
                println!(
                    "  {star} {dot} {:24} {:>5.1} GB  {}",
                    m.name, m.size_gb, m.description
                );
            }
            let extra: Vec<&String> = installed
                .iter()
                .filter(|i| {
                    let name = i.as_str();
                    !openmirai_engine::catalog::CATALOG
                        .iter()
                        .any(|m| name == m.name || name == format!("{}:latest", m.name).as_str())
                })
                .collect();
            if !extra.is_empty() {
                println!(
                    "\n{}Also installed (not in catalog):{}",
                    colors::DIM,
                    colors::RESET
                );
                for i in extra {
                    println!("  ● {i}");
                }
            }
            println!(
                "\n  Download: {}mirai models pull <name>{}   ·   Set default: {}mirai models use <name>{}",
                colors::BOLD, colors::RESET, colors::BOLD, colors::RESET
            );
        }
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
