//! Session backends — the tmux operations behind the SessionManager.

use super::SessionError;

/// Prefix used for every orchestrated tmux session.
pub const TMUX_SESSION_PREFIX: &str = "mirai-";

/// Abstraction over the terminal-multiplexer operations the manager needs.
///
/// The real implementation is [`TmuxBackend`]; tests use a fake so the
/// manager logic (registry, reconciliation, status transitions) is testable
/// without tmux installed.
pub trait SessionBackend: Send + Sync {
    /// Spawn a detached session named `tmux_session`, with `project_dir` as
    /// cwd, running `command` (e.g. `claude --model opus`). `env` pairs are
    /// injected into the session's environment (PRD-013 M6: the spawned
    /// claude — and its hook subprocesses — see MIRAI_SESSION_ID/MIRAI_PORT).
    fn spawn(
        &self,
        tmux_session: &str,
        project_dir: &str,
        command: &str,
        env: &[(String, String)],
    ) -> Result<(), SessionError>;

    /// Type `text` into the session and press Enter.
    fn send_text(&self, tmux_session: &str, text: &str) -> Result<(), SessionError>;

    /// Capture the last `lines` lines of the session's pane.
    fn capture_output(&self, tmux_session: &str, lines: usize)
        -> Result<Vec<String>, SessionError>;

    /// Whether the tmux session is still alive.
    fn session_exists(&self, tmux_session: &str) -> bool;

    /// Kill the tmux session.
    fn kill(&self, tmux_session: &str) -> Result<(), SessionError>;

    /// List live tmux session names managed by us (prefix `mirai-`).
    fn list(&self) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// TmuxBackend — real implementation via shell-out to `tmux`
// ---------------------------------------------------------------------------

/// Real backend: one tmux session per orchestrated session.
///
/// The user can always attach manually: `tmux attach -t mirai-<id>`.
/// Targets are passed as `=name` so tmux does exact (not prefix) matching;
/// pane-level commands (send-keys, capture-pane) need the trailing colon
/// (`=name:` → the session's current window/active pane) — verified against
/// tmux 3.6b, where `capture-pane -t =name` fails with "can't find pane".
#[derive(Debug, Default, Clone)]
pub struct TmuxBackend;

impl TmuxBackend {
    pub fn new() -> Self {
        Self
    }

    /// Run `tmux <args>` and return stdout on success.
    fn tmux(&self, args: &[&str]) -> Result<String, SessionError> {
        let output = std::process::Command::new("tmux")
            .args(args)
            .output()
            .map_err(|e| SessionError::Backend(format!("failed to run tmux: {e}")))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(SessionError::Backend(format!(
                "tmux {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}

impl SessionBackend for TmuxBackend {
    fn spawn(
        &self,
        tmux_session: &str,
        project_dir: &str,
        command: &str,
        env: &[(String, String)],
    ) -> Result<(), SessionError> {
        // `-e KEY=VAL` (tmux ≥ 3.2) sets the session environment at creation,
        // so claude and every subprocess it spawns (hooks included) see it.
        let env_args: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut args: Vec<&str> = vec!["new-session", "-d", "-s", tmux_session, "-c", project_dir];
        for pair in &env_args {
            args.push("-e");
            args.push(pair);
        }
        args.push(command);
        self.tmux(&args).map(|_| ())
    }

    fn send_text(&self, tmux_session: &str, text: &str) -> Result<(), SessionError> {
        let target = format!("={tmux_session}:");
        // `-l` sends the text literally (no key-name interpretation).
        self.tmux(&["send-keys", "-t", &target, "-l", "--", text])?;
        // Enter goes as a key name in a second call.
        self.tmux(&["send-keys", "-t", &target, "Enter"])
            .map(|_| ())
    }

    fn capture_output(
        &self,
        tmux_session: &str,
        lines: usize,
    ) -> Result<Vec<String>, SessionError> {
        let target = format!("={tmux_session}:");
        let start = format!("-{lines}");
        let out = self.tmux(&["capture-pane", "-p", "-t", &target, "-S", &start])?;
        let mut captured: Vec<String> = out.lines().map(|l| l.to_string()).collect();
        // Drop trailing blank lines (pane padding), then keep the last N.
        while captured.last().is_some_and(|l| l.trim().is_empty()) {
            captured.pop();
        }
        if captured.len() > lines {
            captured.drain(..captured.len() - lines);
        }
        Ok(captured)
    }

    fn session_exists(&self, tmux_session: &str) -> bool {
        let target = format!("={tmux_session}");
        self.tmux(&["has-session", "-t", &target]).is_ok()
    }

    fn kill(&self, tmux_session: &str) -> Result<(), SessionError> {
        let target = format!("={tmux_session}");
        self.tmux(&["kill-session", "-t", &target]).map(|_| ())
    }

    fn list(&self) -> Vec<String> {
        self.tmux(&["list-sessions", "-F", "#{session_name}"])
            .map(|out| {
                out.lines()
                    .filter(|l| l.starts_with(TMUX_SESSION_PREFIX))
                    .map(|l| l.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// FakeBackend — test double (crate-visible in test builds)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory backend for tests: tracks spawned sessions, sent text and
    /// scripted pane output.
    #[derive(Default)]
    pub struct FakeBackend {
        /// session name → scripted pane lines
        pub panes: Mutex<HashMap<String, Vec<String>>>,
        /// (session name, text) pairs, in send order
        pub sent: Mutex<Vec<(String, String)>>,
        /// (session, project_dir, command, env) spawn calls
        #[allow(clippy::type_complexity)]
        pub spawned: Mutex<Vec<(String, String, String, Vec<(String, String)>)>>,
    }

    impl FakeBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Script the pane content of a session (simulates the claude UI).
        pub fn set_pane(&self, session: &str, lines: &[&str]) {
            self.panes.lock().unwrap().insert(
                session.to_string(),
                lines.iter().map(|s| s.to_string()).collect(),
            );
        }

        /// Simulate the tmux session dying (claude exited / killed manually).
        pub fn kill_externally(&self, session: &str) {
            self.panes.lock().unwrap().remove(session);
        }
    }

    impl SessionBackend for FakeBackend {
        fn spawn(
            &self,
            tmux_session: &str,
            project_dir: &str,
            command: &str,
            env: &[(String, String)],
        ) -> Result<(), SessionError> {
            self.spawned.lock().unwrap().push((
                tmux_session.to_string(),
                project_dir.to_string(),
                command.to_string(),
                env.to_vec(),
            ));
            self.panes
                .lock()
                .unwrap()
                .insert(tmux_session.to_string(), vec![]);
            Ok(())
        }

        fn send_text(&self, tmux_session: &str, text: &str) -> Result<(), SessionError> {
            if !self.session_exists(tmux_session) {
                return Err(SessionError::Backend(format!("no session {tmux_session}")));
            }
            self.sent
                .lock()
                .unwrap()
                .push((tmux_session.to_string(), text.to_string()));
            Ok(())
        }

        fn capture_output(
            &self,
            tmux_session: &str,
            lines: usize,
        ) -> Result<Vec<String>, SessionError> {
            let panes = self.panes.lock().unwrap();
            let pane = panes
                .get(tmux_session)
                .ok_or_else(|| SessionError::Backend(format!("no session {tmux_session}")))?;
            let start = pane.len().saturating_sub(lines);
            Ok(pane[start..].to_vec())
        }

        fn session_exists(&self, tmux_session: &str) -> bool {
            self.panes.lock().unwrap().contains_key(tmux_session)
        }

        fn kill(&self, tmux_session: &str) -> Result<(), SessionError> {
            self.panes.lock().unwrap().remove(tmux_session);
            Ok(())
        }

        fn list(&self) -> Vec<String> {
            self.panes.lock().unwrap().keys().cloned().collect()
        }
    }
}
