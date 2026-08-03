//! Native folder picker (PRD-013 M9).
//!
//! `POST /api/v1/orchestrator/pick-folder` opens the HOST's native folder
//! dialog and returns the chosen path:
//!
//! - **macOS**: `osascript` with `choose folder` (System Events is activated
//!   first so the dialog comes to the front; verified on this machine).
//! - **Linux**: `zenity --file-selection --directory`.
//! - Anything else (or zenity missing) → `Unsupported` (HTTP 501).
//!
//! Rules: one dialog at a time (`Busy` → 409), ~120s timeout kills the
//! process and reports `Cancelled`, and the command runs async so the server
//! runtime is never blocked while the dialog is open. The command runner is
//! injectable (`DialogRunner`) so every state is unit-testable without a GUI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;

/// Default dialog timeout (~2 minutes, per the pinned contract).
pub const PICK_TIMEOUT: Duration = Duration::from_secs(120);

/// macOS `choose folder` cancel: AppleScript error -128 ("User canceled").
const OSA_CANCEL_MARKER: &str = "-128";

// ---------------------------------------------------------------------------
// Runner abstraction (injectable for tests)
// ---------------------------------------------------------------------------

/// Result of running the dialog command.
#[derive(Debug, Clone)]
pub enum RunOutcome {
    /// Process finished (exit code, stdout, stderr).
    Completed {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// Timeout hit; the process was killed.
    TimedOut,
    /// The program is not installed / not in PATH.
    SpawnNotFound,
    /// Any other spawn failure.
    SpawnError(String),
}

/// Runs the native dialog command. Real impl = tokio process with timeout.
#[async_trait]
pub trait DialogRunner: Send + Sync {
    async fn run(&self, program: &str, args: &[String], timeout: Duration) -> RunOutcome;
}

/// Real runner: `tokio::process` + timeout + kill (never blocks the runtime).
pub struct TokioRunner;

#[async_trait]
impl DialogRunner for TokioRunner {
    async fn run(&self, program: &str, args: &[String], timeout: Duration) -> RunOutcome {
        let mut child = match tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Code review: if this future is dropped (client disconnected,
            // task aborted), don't leave an orphan osascript/zenity dialog.
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RunOutcome::SpawnNotFound,
            Err(e) => return RunOutcome::SpawnError(e.to_string()),
        };

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();

        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                use tokio::io::AsyncReadExt;
                let mut out = String::new();
                let mut err = String::new();
                if let Some(s) = stdout.as_mut() {
                    let _ = s.read_to_string(&mut out).await;
                }
                if let Some(s) = stderr.as_mut() {
                    let _ = s.read_to_string(&mut err).await;
                }
                RunOutcome::Completed {
                    code: status.code(),
                    stdout: out,
                    stderr: err,
                }
            }
            Ok(Err(e)) => RunOutcome::SpawnError(e.to_string()),
            Err(_) => {
                // Timeout: kill the dialog process, report as such.
                let _ = child.kill().await;
                RunOutcome::TimedOut
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FolderPicker
// ---------------------------------------------------------------------------

/// Outcome of a pick request (the handler maps these to HTTP responses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickOutcome {
    /// 200 `{path}` — the user chose a directory.
    Picked(String),
    /// 200 `{cancelled: true}` — user cancelled or the dialog timed out.
    Cancelled,
    /// 409 — another dialog is already open.
    Busy,
    /// 501 — no native picker on this host.
    Unsupported(String),
    /// 500 — the dialog command failed unexpectedly.
    Failed(String),
}

pub struct FolderPicker {
    runner: Box<dyn DialogRunner>,
    busy: AtomicBool,
    timeout: Duration,
}

/// RAII guard: releases the busy flag even if the future is dropped.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Default for FolderPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl FolderPicker {
    /// Picker over the real host dialog.
    pub fn new() -> Self {
        Self::with_runner(Box::new(TokioRunner), PICK_TIMEOUT)
    }

    /// Injectable runner + timeout (tests).
    pub fn with_runner(runner: Box<dyn DialogRunner>, timeout: Duration) -> Self {
        Self {
            runner,
            busy: AtomicBool::new(false),
            timeout,
        }
    }

    /// Open the native picker for `os` (`std::env::consts::OS`) and wait for
    /// the user. One dialog at a time.
    pub async fn pick(&self, os: &str, start: Option<&str>) -> PickOutcome {
        let Some((program, args)) = build_command(os, start) else {
            return PickOutcome::Unsupported(format!(
                "no native folder picker on this host (os: {os})"
            ));
        };

        // One dialog at a time.
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return PickOutcome::Busy;
        }
        let _guard = BusyGuard(&self.busy);

        let outcome = self.runner.run(&program, &args, self.timeout).await;
        parse_outcome(os, outcome)
    }
}

/// Build the dialog command for the host OS. `None` → unsupported.
///
/// `start` is only honored when it is an existing directory.
pub fn build_command(os: &str, start: Option<&str>) -> Option<(String, Vec<String>)> {
    let start = start
        .map(str::trim)
        .filter(|s| !s.is_empty() && std::path::Path::new(s).is_dir());

    match os {
        "macos" => {
            // `activate` first so the dialog comes to the FRONT (verified on
            // this machine — without it the dialog opens behind windows).
            let mut expr =
                String::from("POSIX path of (choose folder with prompt \"Selecciona el proyecto\"");
            if let Some(dir) = start {
                // AppleScript string escape: backslash FIRST, then quotes.
                // Reversed order would double the backslashes we just
                // inserted (`"` → `\"` → `\\"`), un-escaping the quote and
                // re-opening the injection this guards against.
                expr.push_str(&format!(
                    " default location POSIX file \"{}\"",
                    dir.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
            expr.push(')');
            Some((
                "osascript".to_string(),
                vec![
                    "-e".to_string(),
                    "tell application \"System Events\" to activate".to_string(),
                    "-e".to_string(),
                    expr,
                ],
            ))
        }
        "linux" => {
            let mut args = vec!["--file-selection".to_string(), "--directory".to_string()];
            if let Some(dir) = start {
                args.push(format!("--filename={}/", dir.trim_end_matches('/')));
            }
            Some(("zenity".to_string(), args))
        }
        _ => None,
    }
}

/// Map the command result to a pick outcome, per-OS semantics:
/// - macOS cancel: non-zero exit, stderr mentions error -128.
/// - zenity cancel: exit code 1. zenity missing: 501.
/// - timeout (process killed): reported as cancelled (pinned contract).
fn parse_outcome(os: &str, outcome: RunOutcome) -> PickOutcome {
    match outcome {
        RunOutcome::Completed {
            code,
            stdout,
            stderr,
        } => match code {
            Some(0) => {
                let path = stdout.trim();
                // `POSIX path of` returns a trailing slash — normalize it off.
                let path = if path.len() > 1 {
                    path.trim_end_matches('/')
                } else {
                    path
                };
                if path.is_empty() {
                    PickOutcome::Failed("picker returned an empty path".into())
                } else {
                    PickOutcome::Picked(path.to_string())
                }
            }
            Some(1) if os == "linux" => PickOutcome::Cancelled,
            _ if os == "macos" && stderr.contains(OSA_CANCEL_MARKER) => PickOutcome::Cancelled,
            other => {
                PickOutcome::Failed(format!("picker exited with {other:?}: {}", stderr.trim()))
            }
        },
        RunOutcome::TimedOut => PickOutcome::Cancelled,
        RunOutcome::SpawnNotFound => PickOutcome::Unsupported(if os == "linux" {
            "zenity is not installed (needed for the native folder picker on Linux)".into()
        } else {
            "native folder picker command not found".into()
        }),
        RunOutcome::SpawnError(e) => PickOutcome::Failed(format!("could not open picker: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Scripted runner: returns a canned outcome, records invocations,
    /// optionally holds for a while (to exercise the busy flag).
    struct FakeRunner {
        outcome: RunOutcome,
        hold_ms: u64,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl FakeRunner {
        fn new(outcome: RunOutcome) -> Self {
            Self {
                outcome,
                hold_ms: 0,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DialogRunner for FakeRunner {
        async fn run(&self, program: &str, args: &[String], _timeout: Duration) -> RunOutcome {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            if self.hold_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.hold_ms)).await;
            }
            self.outcome.clone()
        }
    }

    fn picker(outcome: RunOutcome) -> FolderPicker {
        FolderPicker::with_runner(Box::new(FakeRunner::new(outcome)), Duration::from_secs(1))
    }

    fn completed(code: i32, stdout: &str, stderr: &str) -> RunOutcome {
        RunOutcome::Completed {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    // -- build_command ---------------------------------------------------------

    #[test]
    fn macos_command_uses_osascript_with_activate_and_prompt() {
        let (prog, args) = build_command("macos", None).unwrap();
        assert_eq!(prog, "osascript");
        assert_eq!(args[1], "tell application \"System Events\" to activate");
        assert!(args[3].contains("choose folder with prompt \"Selecciona el proyecto\""));
        assert!(!args[3].contains("default location"));
    }

    #[test]
    fn macos_start_becomes_default_location_only_if_dir_exists() {
        let (_, args) = build_command("macos", Some("/tmp")).unwrap();
        assert!(args[3].contains("default location POSIX file \"/tmp\""));

        let (_, args) = build_command("macos", Some("/no/such/dir-xyz")).unwrap();
        assert!(!args[3].contains("default location"));
    }

    #[test]
    fn macos_default_location_escapes_backslashes_before_quotes() {
        // A dir name with `\` and `"` is creatable (e.g. via spawn with
        // create_dir:true) and must not break out of the AppleScript string.
        let dir = std::env::temp_dir().join("mirai-esc \\\" test");
        std::fs::create_dir_all(&dir).unwrap();
        let (_, args) = build_command("macos", dir.to_str()).unwrap();
        // `\` → `\\` and `"` → `\"`, so the literal `\"` becomes `\\\"`.
        assert!(
            args[3].contains("mirai-esc \\\\\\\" test"),
            "escaped expr: {}",
            args[3]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn linux_command_uses_zenity_directory_selection() {
        let (prog, args) = build_command("linux", None).unwrap();
        assert_eq!(prog, "zenity");
        assert_eq!(args, vec!["--file-selection", "--directory"]);

        let (_, args) = build_command("linux", Some("/tmp")).unwrap();
        assert!(args.contains(&"--filename=/tmp/".to_string()));
    }

    #[test]
    fn other_os_is_unsupported() {
        assert!(build_command("windows", None).is_none());
        assert!(build_command("freebsd", None).is_none());
    }

    // -- outcome parsing ---------------------------------------------------------

    #[tokio::test]
    async fn picked_path_is_trimmed_and_deslashed() {
        let p = picker(completed(0, "/Users/gabo/proyecto/\n", ""));
        assert_eq!(
            p.pick("macos", None).await,
            PickOutcome::Picked("/Users/gabo/proyecto".into())
        );
    }

    #[tokio::test]
    async fn macos_cancel_is_error_128() {
        let p = picker(completed(
            1,
            "",
            "execution error: El usuario ha cancelado. (-128)",
        ));
        assert_eq!(p.pick("macos", None).await, PickOutcome::Cancelled);
    }

    #[tokio::test]
    async fn zenity_cancel_is_exit_1_and_missing_zenity_is_unsupported() {
        let p = picker(completed(1, "", ""));
        assert_eq!(p.pick("linux", None).await, PickOutcome::Cancelled);

        let p = picker(RunOutcome::SpawnNotFound);
        match p.pick("linux", None).await {
            PickOutcome::Unsupported(msg) => assert!(msg.contains("zenity")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_reports_cancelled() {
        let p = picker(RunOutcome::TimedOut);
        assert_eq!(p.pick("macos", None).await, PickOutcome::Cancelled);
    }

    #[tokio::test]
    async fn unexpected_failure_is_failed() {
        let p = picker(completed(2, "", "boom"));
        match p.pick("macos", None).await {
            PickOutcome::Failed(msg) => assert!(msg.contains("boom")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unsupported_os_short_circuits_without_running() {
        let runner = FakeRunner::new(completed(0, "/x", ""));
        let p = FolderPicker::with_runner(Box::new(runner), Duration::from_secs(1));
        match p.pick("windows", None).await {
            PickOutcome::Unsupported(_) => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    // -- busy flag ----------------------------------------------------------------

    #[tokio::test]
    async fn second_concurrent_pick_is_busy_and_flag_releases_after() {
        let mut runner = FakeRunner::new(completed(0, "/tmp/a\n", ""));
        runner.hold_ms = 150;
        let p = std::sync::Arc::new(FolderPicker::with_runner(
            Box::new(runner),
            Duration::from_secs(1),
        ));

        let p1 = p.clone();
        let first = tokio::spawn(async move { p1.pick("macos", None).await });
        tokio::time::sleep(Duration::from_millis(30)).await;

        // While the dialog is "open", a second pick → Busy.
        assert_eq!(p.pick("macos", None).await, PickOutcome::Busy);

        // The first finishes normally, and the flag is released.
        assert_eq!(first.await.unwrap(), PickOutcome::Picked("/tmp/a".into()));
        assert_eq!(
            p.pick("macos", None).await,
            PickOutcome::Picked("/tmp/a".into())
        );
    }
}
