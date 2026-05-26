//! Mirai CLI — entry point.
//!
//! Subcommands:
//!   mirai              → interactive setup wizard + terminal (default)
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

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(|s| s.as_str()) {
        None => run_default().await,
        Some("version" | "--version" | "-V") => {
            println!("mirai {VERSION}");
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
{bold}mirai{reset} — Agentic coding in your terminal

{bold}USAGE:{reset}
    mirai              Interactive setup wizard + terminal
    mirai serve        Start HTTP server
    mirai version      Show version
    mirai agent load   Import agent from YAML
    mirai agent list   List agents
    mirai help         This message
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}
