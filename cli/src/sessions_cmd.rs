//! `mirai sessions …` — CLI client for the session orchestrator (PRD-013).
//!
//! Talks to a running `mirai serve` over HTTP so the server stays the single
//! source of truth (registry, tmux state, status polling). Subcommands:
//!
//! ```text
//! mirai sessions list
//! mirai sessions spawn --project <dir> --objective "…" [--name N] [--model M] [--effort E] [--ultracode]
//! mirai sessions send <id> <text…>
//! mirai sessions output <id> [--lines N]
//! mirai sessions stop <id>
//! ```
//!
//! Server location: `--host` / `--port` flags, or `MIRAI_HOST` / `MIRAI_PORT`
//! env vars (default `127.0.0.1:3000`). Auth: `MIRAI_API_KEY` → `X-API-Key`.

use std::process;

use serde_json::{json, Value};

use crate::colors;

/// Entry point for the `sessions` subcommand.
pub async fn cmd_sessions(args: &[String]) {
    match args.first().map(|s| s.as_str()) {
        None | Some("list") => list(args).await,
        Some("spawn") => spawn(&args[1..]).await,
        Some("send") => send(&args[1..]).await,
        Some("output") => output(&args[1..]).await,
        Some("stop") => stop(&args[1..]).await,
        Some("help" | "--help" | "-h") => print_sessions_help(),
        Some(other) => {
            eprintln!(
                "{}Unknown sessions subcommand: {other}. Run `mirai sessions help`.{}",
                colors::RED,
                colors::RESET
            );
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client against the orchestrator API
// ---------------------------------------------------------------------------

struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl Client {
    fn from_args(args: &[String]) -> Self {
        let host = super::parse_flag(args, "--host")
            .or_else(|| std::env::var("MIRAI_HOST").ok())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = super::parse_flag(args, "--port")
            .or_else(|| std::env::var("MIRAI_PORT").ok())
            .unwrap_or_else(|| "3000".to_string());
        Self {
            base_url: format!("http://{host}:{port}/api/v1/orchestrator"),
            api_key: std::env::var("MIRAI_API_KEY").ok(),
            http: reqwest::Client::new(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.request(method, format!("{}{path}", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.header("X-API-Key", key);
        }
        req
    }

    /// Send the request; exit(1) with a readable message on any failure.
    async fn run(&self, req: reqwest::RequestBuilder) -> Value {
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "{}Cannot reach mirai serve at {} — is it running? ({e}){}",
                    colors::RED,
                    self.base_url,
                    colors::RESET
                );
                process::exit(1);
            }
        };
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            eprintln!("{}Server error ({status}): {msg}{}", colors::RED, colors::RESET);
            process::exit(1);
        }
        body
    }
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

async fn list(args: &[String]) {
    let client = Client::from_args(args);
    let body = client.run(client.request(reqwest::Method::GET, "/sessions")).await;
    let sessions = body.as_array().cloned().unwrap_or_default();

    if sessions.is_empty() {
        println!(
            "{}No sessions. Spawn one: mirai sessions spawn --project <dir> --objective \"…\"{}",
            colors::DIM,
            colors::RESET
        );
        return;
    }
    println!("{}", render_sessions_table(&sessions));
    println!(
        "{}attach manually: tmux attach -t <tmux_session>{}",
        colors::DIM,
        colors::RESET
    );
}

async fn spawn(args: &[String]) {
    let project = super::parse_flag(args, "--project").or_else(|| super::parse_flag(args, "-p"));
    let objective =
        super::parse_flag(args, "--objective").or_else(|| super::parse_flag(args, "-o"));
    let (Some(project), Some(objective)) = (project, objective) else {
        eprintln!(
            "{}Usage: mirai sessions spawn --project <dir> --objective \"…\" [--name N] [--model M] [--effort E] [--ultracode]{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    };

    // Resolve relative project paths against cwd (tmux needs an absolute dir).
    let project = std::fs::canonicalize(&project)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(project);

    let mut payload = json!({
        "project_dir": project,
        "objective": objective,
        "ultracode": super::has_flag(args, "--ultracode"),
    });
    if let Some(name) = super::parse_flag(args, "--name") {
        payload["name"] = json!(name);
    }
    if let Some(model) = super::parse_flag(args, "--model") {
        payload["model"] = json!(model);
    }
    if let Some(effort) = super::parse_flag(args, "--effort") {
        payload["effort"] = json!(effort);
    }

    let client = Client::from_args(args);
    let body = client
        .run(client.request(reqwest::Method::POST, "/sessions").json(&payload))
        .await;

    let id = body["id"].as_str().unwrap_or("?");
    let tmux = body["tmux_session"].as_str().unwrap_or("?");
    println!(
        "{}Session spawned:{} {id}  {}(tmux attach -t {tmux}){}",
        colors::GREEN,
        colors::RESET,
        colors::DIM,
        colors::RESET
    );
}

async fn send(args: &[String]) {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!(
            "{}Usage: mirai sessions send <id> <text…>{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    };
    // Text: everything after the id that is not a flag pair we understand.
    let text = super::parse_flag(args, "--text").unwrap_or_else(|| {
        args[1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    });
    if text.trim().is_empty() {
        eprintln!(
            "{}Nothing to send. Usage: mirai sessions send <id> <text…>{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    }

    let client = Client::from_args(args);
    client
        .run(
            client
                .request(reqwest::Method::POST, &format!("/sessions/{id}/send"))
                .json(&json!({ "text": text })),
        )
        .await;
    println!("{}Sent to {id}.{}", colors::GREEN, colors::RESET);
}

async fn output(args: &[String]) {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!(
            "{}Usage: mirai sessions output <id> [--lines N]{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    };
    let lines = super::parse_flag(args, "--lines").unwrap_or_else(|| "100".into());

    let client = Client::from_args(args);
    let body = client
        .run(client.request(
            reqwest::Method::GET,
            &format!("/sessions/{id}/output?lines={lines}"),
        ))
        .await;

    for line in body["lines"].as_array().cloned().unwrap_or_default() {
        println!("{}", line.as_str().unwrap_or(""));
    }
}

async fn stop(args: &[String]) {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!(
            "{}Usage: mirai sessions stop <id>{}",
            colors::RED,
            colors::RESET
        );
        process::exit(1);
    };

    let client = Client::from_args(args);
    client
        .run(client.request(reqwest::Method::POST, &format!("/sessions/{id}/stop")))
        .await;
    println!("{}Session {id} stopped.{}", colors::GREEN, colors::RESET);
}

fn print_sessions_help() {
    println!(
        "\
{bold}mirai sessions{reset} — orchestrated Claude Code sessions over tmux

{bold}USAGE:{reset}
    mirai sessions list                          List sessions (live directory)
    mirai sessions spawn --project <dir> --objective \"…\"
                         [--name N] [--model M] [--effort E] [--ultracode]
    mirai sessions send <id> <text…>             Send a prompt to a session
    mirai sessions output <id> [--lines N]       Show the session's pane output
    mirai sessions stop <id>                     Kill the session

{bold}SERVER:{reset}
    --host <h> / --port <p>   Where mirai serve runs (default 127.0.0.1:3000)
    MIRAI_HOST / MIRAI_PORT   Env var equivalents
    MIRAI_API_KEY             Sent as X-API-Key when set

{bold}NOTES:{reset}
    Every session is a tmux session (mirai-<id>); take over manually with
    `tmux attach -t mirai-<id>`. With --ultracode the keyword is prepended
    to the first prompt sent to the session.
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}

// ---------------------------------------------------------------------------
// Table rendering
// ---------------------------------------------------------------------------

/// Render the sessions list as a readable fixed-width table.
fn render_sessions_table(sessions: &[Value]) -> String {
    let headers = ["ID", "NAME", "STATUS", "MODEL", "PROJECT", "LAST ACTIVITY"];

    let rows: Vec<[String; 6]> = sessions
        .iter()
        .map(|s| {
            [
                cell(s, "id"),
                cell(s, "name"),
                cell(s, "status"),
                s.get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                cell(s, "project_dir"),
                cell(s, "last_activity"),
            ]
        })
        .collect();

    // Column widths = max(header, cells).
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, value) in row.iter().enumerate() {
            widths[i] = widths[i].max(value.len());
        }
    }

    let mut out = String::new();
    for (i, h) in headers.iter().enumerate() {
        out.push_str(&format!("{:<width$}  ", h, width = widths[i]));
    }
    out.push('\n');
    for row in &rows {
        for (i, value) in row.iter().enumerate() {
            out.push_str(&format!("{:<width$}  ", value, width = widths[i]));
        }
        out.push('\n');
    }
    out
}

fn cell(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("-").to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_renders_headers_and_rows_aligned() {
        let sessions = vec![
            serde_json::json!({
                "id": "abc12345",
                "name": "worker-1",
                "status": "working",
                "model": "claude-opus-4-8",
                "project_dir": "/tmp/proj",
                "last_activity": "2026-07-09T10:00:00Z",
            }),
            serde_json::json!({
                "id": "x1",
                "name": "n",
                "status": "waiting",
                "model": null,
                "project_dir": "/p",
                "last_activity": "2026-07-09T11:00:00Z",
            }),
        ];
        let table = render_sessions_table(&sessions);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("ID"));
        assert!(lines[0].contains("STATUS"));
        assert!(lines[1].contains("abc12345"));
        assert!(lines[1].contains("working"));
        // Missing model renders as "default".
        assert!(lines[2].contains("default"));
        // Columns align: STATUS starts at the same offset in every row.
        let col = lines[0].find("STATUS").unwrap();
        assert_eq!(&lines[1][col..col + 7], "working");
        assert_eq!(&lines[2][col..col + 7], "waiting");
    }
}
