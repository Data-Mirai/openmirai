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
    AgentSpec, ExecutionStatus, GraphRunner, RegistryExecutor, SimpleExecutionContext, ToolRegistry,
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
            println!("{}Server mode not yet implemented.{}", colors::YELLOW, colors::RESET);
            process::exit(1);
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

/// Execute an agent from a JSON/YAML file.
///
/// Usage: mirai run <agent.json|agent.yaml> [--input '{"key":"value"}']
///
/// Output: JSON with status, state, trace, transcript.
async fn run_agent(args: &[String]) {
    let path = match args.first() {
        Some(p) => p.as_str(),
        None => {
            eprintln!(
                "{}Usage: mirai run <agent.json|agent.yaml> [--input '{{...}}']{}",
                colors::YELLOW,
                colors::RESET
            );
            process::exit(1);
        }
    };

    // Parse optional --input JSON
    let input_json: Option<String> = args.windows(2).find_map(|w| {
        if w[0] == "--input" || w[0] == "-i" {
            Some(w[1].clone())
        } else {
            None
        }
    });

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

    // Create context
    let context = SimpleExecutionContext::default_dev();

    // If input provided, inject into entry node state
    // (For now, the trigger tools handle mock_payload from config)
    if let Some(ref input_str) = input_json {
        // Parse input JSON and inject into the first trigger node's config
        if let Ok(input_value) = serde_json::from_str::<serde_json::Value>(input_str) {
            if let Some(entry) = graph.nodes.iter_mut().find(|n| n.tool_type.starts_with("trigger/")) {
                entry.config.insert(
                    "mock_payload".to_string(),
                    input_value,
                );
            }
        }
    }

    // Run
    match runner.run(&graph, &context).await {
        Ok(result) => {
            // Build output JSON
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
    mirai                       Interactive setup wizard + terminal
    mirai run <file> [-i JSON]  Execute agent from JSON/YAML file
    mirai validate <file>       Validate agent spec
    mirai serve                 Start HTTP server
    mirai version               Show version
    mirai agent load <file>     Import agent from YAML
    mirai agent list            List agents
    mirai help                  This message

{bold}EXAMPLES:{reset}
    mirai run agent.json
    mirai run agent.yaml --input '{{\"query\": \"hello\"}}'
    mirai validate my-agent.json
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}
