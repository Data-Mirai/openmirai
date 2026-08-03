//! `mirai sessions watch` — live full-screen terminal dashboard (PRD-013).
//!
//! Terminal parity with the web UI: a live table of orchestrated sessions
//! plus an activity feed, updating in real time.
//!
//! - **Data source**: the server's SSE stream (`/api/v1/orchestrator/events`),
//!   with automatic fallback to `GET /sessions` polling every ~2s whenever the
//!   stream is down (it keeps retrying SSE in the background).
//! - **Render**: raw ANSI on the alternate screen buffer, cursor hidden,
//!   frame redrawn in place (home + clear-to-eol per line) — no flicker.
//!   Adapts to the real terminal size on every frame, so resizes just work.
//! - **Exit**: `q`, `Esc` or `Ctrl-C`. The terminal is ALWAYS restored — a
//!   guard runs on drop and a panic hook restores it even on panic.
//!
//! Same server flags as the rest of `mirai sessions`: `--host` / `--port`
//! (or `MIRAI_HOST` / `MIRAI_PORT`), `MIRAI_API_KEY` → `X-API-Key`.

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::colors;

/// Refresh cadence of the render loop (also drives the spinner).
const FRAME_MS: u64 = 250;
/// Fallback polling cadence while the SSE stream is down.
const POLL_FALLBACK_SECS: u64 = 2;
/// Max entries kept in the activity feed.
const ACTIVITY_MAX: usize = 10;

const GRAY: &str = "\x1b[90m";
const AMBER_BOLD: &str = "\x1b[1;33m";
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

// ---------------------------------------------------------------------------
// Shared state between the data task and the render loop
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ActivityEntry {
    time: String, // HH:MM:SS local
    kind: String, // session_created | session_status_changed | …
    detail: String,
}

#[derive(Default)]
struct WatchState {
    /// id → session record (contract shape).
    sessions: HashMap<String, Value>,
    activity: VecDeque<ActivityEntry>,
    /// True while the SSE stream is connected.
    live: bool,
    /// Last data-source error (shown in the header while polling).
    last_error: Option<String>,
}

type Shared = Arc<Mutex<WatchState>>;

fn push_activity(state: &mut WatchState, kind: &str, detail: String) {
    state.activity.push_front(ActivityEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
        kind: kind.to_string(),
        detail,
    });
    state.activity.truncate(ACTIVITY_MAX);
}

/// Apply one orchestrator event to the state (upserts / status updates).
fn apply_event(state: &mut WatchState, event: &str, data: &Value) {
    let id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();

    match event {
        "session_created" => {
            let name = data.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
            push_activity(state, event, name.to_string());
            state.sessions.insert(id, data.clone());
        }
        "session_status_changed" => {
            let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            push_activity(state, event, format!("{id} → {status}"));
            if let Some(rec) = state.sessions.get_mut(&id) {
                rec["status"] = data["status"].clone();
                if data.get("last_activity").is_some() {
                    rec["last_activity"] = data["last_activity"].clone();
                }
            }
        }
        "session_stopped" => {
            push_activity(state, event, id.clone());
            if let Some(rec) = state.sessions.get_mut(&id) {
                rec["status"] = Value::String("stopped".into());
            }
        }
        "session_output" => {
            let n = data
                .get("lines")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            push_activity(state, event, format!("{id} (+{n} lines)"));
        }
        // M6: resource activity from the Claude Code hooks — terminal parity:
        // you SEE who reads/writes/executes what.
        "session_activity" => {
            let action = data.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let target = data
                .get("path")
                .and_then(|v| v.as_str())
                .map(abbrev_path)
                .unwrap_or_else(|| {
                    data.get("tool")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string()
                });
            push_activity(state, event, format!("{id} {action}\u{2192}{target}"));
        }
        _ => {}
    }
}

/// Abbreviate a filesystem path for the activity feed: keep the last two
/// components ("…/src/main.rs").
fn abbrev_path(path: &str) -> String {
    let parts: Vec<&str> = path
        .trim_end_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    match parts.len() {
        0 => path.to_string(),
        1 => parts[0].to_string(),
        n => format!("\u{2026}/{}/{}", parts[n - 2], parts[n - 1]),
    }
}

/// Replace the whole session directory (from a GET /sessions refresh).
fn replace_sessions(state: &mut WatchState, list: &[Value]) {
    state.sessions = list
        .iter()
        .filter_map(|s| {
            s.get("id")
                .and_then(|v| v.as_str())
                .map(|id| (id.to_string(), s.clone()))
        })
        .collect();
}

// ---------------------------------------------------------------------------
// Data task: SSE with polling fallback
// ---------------------------------------------------------------------------

struct Server {
    base_url: String, // http://host:port/api/v1/orchestrator
    host_label: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl Server {
    fn from_args(args: &[String]) -> Self {
        let host = super::parse_flag(args, "--host")
            .or_else(|| std::env::var("MIRAI_HOST").ok())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = super::parse_flag(args, "--port")
            .or_else(|| std::env::var("MIRAI_PORT").ok())
            .unwrap_or_else(|| "3000".to_string());
        Self {
            base_url: format!("http://{host}:{port}/api/v1/orchestrator"),
            host_label: format!("{host}:{port}"),
            api_key: std::env::var("MIRAI_API_KEY").ok(),
            http: reqwest::Client::new(),
        }
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.get(format!("{}{path}", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.header("X-API-Key", key);
        }
        req
    }

    async fn fetch_sessions(&self) -> Result<Vec<Value>, String> {
        let resp = self
            .get("/sessions")
            .send()
            .await
            .map_err(|e| format!("unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(body.as_array().cloned().unwrap_or_default())
    }
}

/// Background task: keep the state fresh. Prefers the SSE stream; while it is
/// down, polls GET /sessions every ~2s and retries the stream each cycle.
async fn data_loop(server: Arc<Server>, shared: Shared) {
    loop {
        // Full refresh first (also the polling fallback path).
        match server.fetch_sessions().await {
            Ok(list) => {
                let mut st = shared.lock().unwrap();
                replace_sessions(&mut st, &list);
                st.last_error = None;
            }
            Err(e) => {
                let mut st = shared.lock().unwrap();
                st.live = false;
                st.last_error = Some(e);
            }
        }

        // Try to attach to the live stream.
        match server.get("/events").send().await {
            Ok(resp) if resp.status().is_success() => {
                {
                    let mut st = shared.lock().unwrap();
                    st.live = true;
                    st.last_error = None;
                }
                let mut resp = resp;
                let mut buffer = String::new();
                // Read chunks until the stream drops.
                while let Ok(Some(bytes)) = resp.chunk().await {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    for (event, data) in parse_sse_frames(&mut buffer) {
                        if let Ok(json) = serde_json::from_str::<Value>(&data) {
                            let mut st = shared.lock().unwrap();
                            apply_event(&mut st, &event, &json);
                        }
                    }
                }
                shared.lock().unwrap().live = false;
            }
            Ok(resp) => {
                let mut st = shared.lock().unwrap();
                st.live = false;
                st.last_error = Some(format!("SSE HTTP {}", resp.status()));
            }
            Err(_) => {
                shared.lock().unwrap().live = false;
            }
        }

        tokio::time::sleep(Duration::from_secs(POLL_FALLBACK_SECS)).await;
    }
}

/// Extract complete SSE frames (`event:`/`data:` pairs) from the buffer,
/// leaving any incomplete trailing frame in place.
fn parse_sse_frames(buffer: &mut String) -> Vec<(String, String)> {
    let mut frames = Vec::new();
    while let Some(pos) = buffer.find("\n\n") {
        let frame: String = buffer.drain(..pos + 2).collect();
        let mut event = String::new();
        let mut data = String::new();
        for line in frame.lines() {
            if let Some(v) = line.strip_prefix("event: ") {
                event = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data: ") {
                data = v.trim().to_string();
            }
        }
        if !event.is_empty() && !data.is_empty() {
            frames.push((event, data));
        }
    }
    frames
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Display priority: sessions needing a human float to the top.
fn status_rank(status: &str) -> u8 {
    match status {
        "permission" => 0,
        "working" => 1,
        "waiting" => 2,
        "starting" => 3,
        "error" => 4,
        _ => 5, // stopped / unknown
    }
}

fn status_cell(status: &str, tick: usize) -> (String, usize) {
    // Returns (colored text, visible width). Width is fixed at 11 chars.
    let (color, label) = match status {
        "working" => (
            colors::CYAN,
            format!("{} working", SPINNER[tick % SPINNER.len()]),
        ),
        "waiting" => (colors::GREEN, "waiting".to_string()),
        "permission" => (AMBER_BOLD, "PERMISSION".to_string()),
        "stopped" => (GRAY, "stopped".to_string()),
        "error" => (colors::RED, "error".to_string()),
        other => (colors::DIM, other.to_string()),
    };
    let visible = label.chars().count();
    (format!("{color}{label}{}", colors::RESET), visible)
}

/// Truncate to `max` chars, appending `…` when cut.
fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        text.to_string()
    } else {
        let mut out: String = chars[..max.saturating_sub(1)].iter().collect();
        out.push('…');
        out
    }
}

/// Human relative time from an RFC-3339 timestamp ("12s", "3m", "2h", "1d").
fn relative_time(now_epoch: i64, rfc3339: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return "-".to_string();
    };
    let delta = (now_epoch - ts.timestamp()).max(0);
    match delta {
        0..=59 => format!("{delta}s"),
        60..=3599 => format!("{}m", delta / 60),
        3600..=86_399 => format!("{}h", delta / 3600),
        _ => format!("{}d", delta / 86_400),
    }
}

/// Compose the MODEL column: model [effort] [ultra].
fn model_cell(rec: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(
        rec.get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string(),
    );
    if let Some(effort) = rec.get("effort").and_then(|v| v.as_str()) {
        parts.push(effort.to_string());
    }
    if rec.get("ultracode").and_then(|v| v.as_bool()) == Some(true) {
        parts.push("ultra".to_string());
    }
    parts.join(" ")
}

/// Sessions sorted for display (permission first, then by created_at desc).
fn sorted_sessions(state: &WatchState) -> Vec<&Value> {
    let mut list: Vec<&Value> = state.sessions.values().collect();
    list.sort_by(|a, b| {
        let ra = status_rank(a.get("status").and_then(|v| v.as_str()).unwrap_or(""));
        let rb = status_rank(b.get("status").and_then(|v| v.as_str()).unwrap_or(""));
        ra.cmp(&rb).then_with(|| {
            let ca = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            let cb = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            cb.cmp(ca)
        })
    });
    list
}

/// Count sessions per status → colored summary for the header.
fn status_counts_line(state: &WatchState) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for rec in state.sessions.values() {
        let s = rec.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        *counts.entry(s).or_insert(0) += 1;
    }
    let piece = |label: &str, color: &str| -> Option<String> {
        counts
            .get(label)
            .map(|n| format!("{color}{n} {label}{}", colors::RESET))
    };
    let parts: Vec<String> = [
        piece("permission", AMBER_BOLD),
        piece("working", colors::CYAN),
        piece("waiting", colors::GREEN),
        piece("starting", colors::DIM),
        piece("error", colors::RED),
        piece("stopped", GRAY),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        format!("{}no sessions{}", colors::DIM, colors::RESET)
    } else {
        parts.join(&format!("{} · {}", colors::DIM, colors::RESET))
    }
}

/// Build the complete frame for the given terminal size.
///
/// Every line ends with clear-to-eol; the frame starts at home and ends with
/// clear-below — redrawing in place, no flicker.
fn build_frame(
    state: &WatchState,
    host_label: &str,
    width: usize,
    height: usize,
    tick: usize,
    now_epoch: i64,
) -> String {
    const EOL: &str = "\x1b[K\r\n";
    let width = width.max(40);
    let mut out = String::from("\x1b[H"); // cursor home

    // -- header ------------------------------------------------------------
    let link = if state.live {
        format!("{}● live{}", colors::GREEN, colors::RESET)
    } else {
        let reason = state
            .last_error
            .as_deref()
            .map(|e| format!(" ({})", truncate(e, 30)))
            .unwrap_or_default();
        format!("{}○ polling{reason}{}", colors::YELLOW, colors::RESET)
    };
    out.push_str(&format!(
        "{}MIRAI SESSIONS{}  {}{host_label}{}  {link}{EOL}",
        colors::BOLD,
        colors::RESET,
        colors::DIM,
        colors::RESET,
    ));
    out.push_str(&format!("{}{EOL}", status_counts_line(state)));
    out.push_str(&format!(
        "{}{}{}{EOL}",
        colors::DIM,
        "─".repeat(width),
        colors::RESET
    ));

    // -- sessions table ------------------------------------------------------
    // Fixed columns; OBJECTIVE gets whatever width remains.
    let id_w = 8;
    let name_w = 16;
    let status_w = 11;
    let model_w = 22;
    let last_w = 5;
    let fixed = id_w + 2 + name_w + 2 + status_w + 2 + model_w + 2 + 2 + last_w;
    let obj_w = width.saturating_sub(fixed).max(8);

    out.push_str(&format!(
        "{}{:<id_w$}  {:<name_w$}  {:<status_w$}  {:<model_w$}  {:<obj_w$}  {:>last_w$}{}{EOL}",
        colors::BOLD,
        "ID",
        "NAME",
        "STATUS",
        "MODEL",
        "OBJECTIVE",
        "LAST",
        colors::RESET,
    ));

    // Rows that fit: total height minus header(4) + activity panel + footer.
    let activity_lines = state.activity.len().min(ACTIVITY_MAX) + 2; // title + sep
    let footer_lines = 1usize;
    let max_rows = height
        .saturating_sub(4 + activity_lines + footer_lines)
        .max(3);

    let sessions = sorted_sessions(state);
    if sessions.is_empty() {
        out.push_str(&format!(
            "{}  (no sessions — spawn one: mirai sessions spawn --project <dir> --objective \"…\"){}{EOL}",
            colors::DIM,
            colors::RESET
        ));
    }
    for rec in sessions.iter().take(max_rows) {
        let str_of = |key: &str| rec.get(key).and_then(|v| v.as_str()).unwrap_or("-");
        let status = str_of("status");
        let (status_txt, visible) = status_cell(status, tick);
        let pad = " ".repeat(status_w.saturating_sub(visible));
        let dim_row = status == "stopped";
        let (row_color, row_reset) = if dim_row {
            (GRAY, colors::RESET)
        } else {
            ("", "")
        };
        out.push_str(&format!(
            "{row_color}{:<id_w$}  {:<name_w$}  {row_reset}{status_txt}{pad}{row_color}  {:<model_w$}  {:<obj_w$}  {:>last_w$}{row_reset}{EOL}",
            truncate(str_of("id"), id_w),
            truncate(str_of("name"), name_w),
            truncate(&model_cell(rec), model_w),
            truncate(str_of("objective"), obj_w),
            relative_time(now_epoch, str_of("last_activity")),
        ));
    }
    let hidden = sessions.len().saturating_sub(max_rows);
    if hidden > 0 {
        out.push_str(&format!(
            "{}  … {hidden} more (enlarge the terminal){}{EOL}",
            colors::DIM,
            colors::RESET
        ));
    }

    // -- activity panel ------------------------------------------------------
    out.push_str(&format!(
        "{}{}{}{EOL}",
        colors::DIM,
        "─".repeat(width),
        colors::RESET
    ));
    out.push_str(&format!("{}ACTIVITY{}{EOL}", colors::BOLD, colors::RESET));
    if state.activity.is_empty() {
        out.push_str(&format!(
            "{}  (waiting for events…){}{EOL}",
            colors::DIM,
            colors::RESET
        ));
    }
    for entry in state.activity.iter().take(ACTIVITY_MAX) {
        let kind_color = match entry.kind.as_str() {
            "session_created" => colors::GREEN,
            "session_status_changed" => colors::CYAN,
            "session_activity" => colors::MAGENTA,
            "session_stopped" => GRAY,
            _ => colors::DIM,
        };
        let line = format!(
            "{}{}{}  {kind_color}{:<22}{}  {}",
            colors::DIM,
            entry.time,
            colors::RESET,
            truncate(&entry.kind, 22),
            colors::RESET,
            truncate(&entry.detail, width.saturating_sub(35)),
        );
        out.push_str(&format!("{line}{EOL}"));
    }

    // -- footer ---------------------------------------------------------------
    out.push_str(&format!(
        "{}q quit · attach: tmux attach -t mirai-<id>{}\x1b[K",
        colors::DIM,
        colors::RESET
    ));

    out.push_str("\x1b[J"); // clear anything below the frame
    out
}

// ---------------------------------------------------------------------------
// Terminal guard — ALWAYS restore, also on panic
// ---------------------------------------------------------------------------

fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(
        stdout,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

/// RAII guard: enters raw mode + alternate screen, restores on drop.
struct TermGuard;

impl TermGuard {
    fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the live dashboard until the user quits.
pub async fn run(args: &[String]) {
    let server = Arc::new(Server::from_args(args));
    let shared: Shared = Arc::new(Mutex::new(WatchState::default()));

    // Restore the terminal even if something panics mid-frame.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let guard = match TermGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                "{}Cannot initialize the terminal: {e}{}",
                colors::RED,
                colors::RESET
            );
            std::process::exit(1);
        }
    };

    tokio::spawn(data_loop(server.clone(), shared.clone()));

    let mut tick: usize = 0;
    loop {
        // -- input (non-blocking, drain everything pending) -----------------
        let mut quit = false;
        while crossterm::event::poll(Duration::from_millis(0)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    use crossterm::event::{KeyCode, KeyModifiers};
                    let ctrl_c = key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) || ctrl_c {
                        quit = true;
                    }
                }
                // Resize is handled implicitly: size is re-read every frame.
                Ok(_) => {}
                Err(_) => quit = true,
            }
        }
        if quit {
            break;
        }

        // -- render ----------------------------------------------------------
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let frame = {
            let st = shared.lock().unwrap();
            build_frame(
                &st,
                &server.host_label,
                width as usize,
                height as usize,
                tick,
                chrono::Utc::now().timestamp(),
            )
        };
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(frame.as_bytes());
        let _ = stdout.flush();

        tick = tick.wrapping_add(1);
        tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
    }

    drop(guard);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str, status: &str, created: &str) -> Value {
        json!({
            "id": id,
            "name": format!("mirai-{id}"),
            "project_dir": "/tmp/p",
            "objective": "test objective",
            "status": status,
            "model": "claude-opus-4-8",
            "effort": "high",
            "ultracode": true,
            "tmux_session": format!("mirai-{id}"),
            "created_at": created,
            "last_activity": created,
        })
    }

    #[test]
    fn parse_sse_frames_extracts_complete_frames_only() {
        let mut buf = String::from(
            ": connected\n\nevent: session_stopped\ndata: {\"id\":\"a\"}\n\nevent: partial\nda",
        );
        let frames = parse_sse_frames(&mut buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "session_stopped");
        assert_eq!(frames[0].1, "{\"id\":\"a\"}");
        // The incomplete frame stays buffered.
        assert_eq!(buf, "event: partial\nda");

        // Completing the frame yields it.
        buf.push_str("ta: {\"id\":\"b\"}\n\n");
        let frames = parse_sse_frames(&mut buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "partial");
        assert!(buf.is_empty());
    }

    #[test]
    fn apply_event_upserts_updates_and_stops() {
        let mut st = WatchState::default();

        apply_event(
            &mut st,
            "session_created",
            &session("abc", "starting", "2026-07-09T10:00:00Z"),
        );
        assert_eq!(st.sessions.len(), 1);
        assert_eq!(st.activity.len(), 1);

        apply_event(
            &mut st,
            "session_status_changed",
            &json!({"id": "abc", "status": "working", "last_activity": "2026-07-09T10:01:00Z"}),
        );
        assert_eq!(st.sessions["abc"]["status"], "working");
        assert_eq!(st.sessions["abc"]["last_activity"], "2026-07-09T10:01:00Z");

        apply_event(
            &mut st,
            "session_output",
            &json!({"id": "abc", "lines": ["x", "y"]}),
        );
        assert!(st.activity[0].detail.contains("+2 lines"));

        apply_event(&mut st, "session_stopped", &json!({"id": "abc"}));
        assert_eq!(st.sessions["abc"]["status"], "stopped");
        assert_eq!(st.activity.len(), 4);
    }

    #[test]
    fn session_activity_enters_the_feed_abbreviated() {
        let mut st = WatchState::default();
        apply_event(
            &mut st,
            "session_activity",
            &json!({"id": "abc123", "tool": "Write", "action": "write",
                    "path": "/home/user/proj/src/main.rs", "ts": "2026-07-10T10:00:00Z"}),
        );
        assert_eq!(st.activity.len(), 1);
        assert_eq!(st.activity[0].kind, "session_activity");
        assert_eq!(
            st.activity[0].detail,
            "abc123 write\u{2192}\u{2026}/src/main.rs"
        );

        // Bash without path falls back to the tool name.
        apply_event(
            &mut st,
            "session_activity",
            &json!({"id": "abc123", "tool": "Bash", "action": "exec", "path": null}),
        );
        assert_eq!(st.activity[0].detail, "abc123 exec\u{2192}Bash");
    }

    #[test]
    fn abbrev_path_keeps_last_two_components() {
        assert_eq!(abbrev_path("/a/b/c/d.rs"), "\u{2026}/c/d.rs");
        assert_eq!(abbrev_path("file.md"), "file.md");
        assert_eq!(abbrev_path("/x"), "x");
    }

    #[test]
    fn activity_feed_is_capped() {
        let mut st = WatchState::default();
        for i in 0..25 {
            apply_event(&mut st, "session_stopped", &json!({"id": format!("s{i}")}));
        }
        assert_eq!(st.activity.len(), ACTIVITY_MAX);
        // Newest first.
        assert_eq!(st.activity[0].detail, "s24");
    }

    #[test]
    fn sorted_sessions_puts_permission_first_and_stopped_last() {
        let mut st = WatchState::default();
        replace_sessions(
            &mut st,
            &[
                session("s1", "stopped", "2026-07-09T10:00:03Z"),
                session("s2", "working", "2026-07-09T10:00:02Z"),
                session("s3", "permission", "2026-07-09T10:00:01Z"),
                session("s4", "waiting", "2026-07-09T10:00:00Z"),
            ],
        );
        let order: Vec<&str> = sorted_sessions(&st)
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(order, vec!["s3", "s2", "s4", "s1"]);
    }

    #[test]
    fn relative_time_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-09T12:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(relative_time(now, "2026-07-09T11:59:50Z"), "10s");
        assert_eq!(relative_time(now, "2026-07-09T11:57:00Z"), "3m");
        assert_eq!(relative_time(now, "2026-07-09T09:00:00Z"), "3h");
        assert_eq!(relative_time(now, "2026-07-07T12:00:00Z"), "2d");
        assert_eq!(relative_time(now, "not-a-date"), "-");
    }

    #[test]
    fn truncate_respects_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hola", 0), "");
    }

    #[test]
    fn model_cell_composes_model_effort_ultracode() {
        let rec = session("a", "waiting", "2026-07-09T10:00:00Z");
        assert_eq!(model_cell(&rec), "claude-opus-4-8 high ultra");
        let bare = json!({"model": null, "effort": null, "ultracode": false});
        assert_eq!(model_cell(&bare), "default");
    }

    #[test]
    fn build_frame_renders_header_rows_and_activity() {
        let mut st = WatchState {
            live: true,
            ..Default::default()
        };
        replace_sessions(
            &mut st,
            &[
                session("abc12345", "permission", "2026-07-09T10:00:00Z"),
                session("def67890", "stopped", "2026-07-09T09:00:00Z"),
            ],
        );
        push_activity(&mut st, "session_created", "mirai-abc12345".into());

        let now = chrono::DateTime::parse_from_rfc3339("2026-07-09T10:00:30Z")
            .unwrap()
            .timestamp();
        let frame = build_frame(&st, "127.0.0.1:3000", 100, 30, 0, now);

        assert!(frame.starts_with("\x1b[H"), "redraws from home");
        assert!(frame.contains("MIRAI SESSIONS"));
        assert!(frame.contains("127.0.0.1:3000"));
        assert!(frame.contains("● live"));
        assert!(frame.contains("PERMISSION"));
        assert!(frame.contains("abc12345"));
        assert!(frame.contains("ACTIVITY"));
        assert!(frame.contains("session_created"));
        assert!(frame.contains("30s")); // relative last_activity
        assert!(frame.contains("q quit"));
        assert!(frame.ends_with("\x1b[J"), "clears below the frame");
        // Permission row appears before the stopped row.
        assert!(frame.find("abc12345").unwrap() < frame.find("def67890").unwrap());
    }

    #[test]
    fn build_frame_offline_shows_polling_state() {
        let st = WatchState {
            live: false,
            last_error: Some("unreachable: connection refused".into()),
            ..Default::default()
        };
        let frame = build_frame(&st, "127.0.0.1:3000", 80, 24, 3, 0);
        assert!(frame.contains("○ polling"));
        assert!(frame.contains("no sessions"));
    }
}
