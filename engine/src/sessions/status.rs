//! Session status + heuristic detection from Claude Code's terminal UI.

use serde::{Deserialize, Serialize};

/// Lifecycle status of an orchestrated session.
///
/// Semantics match the contract pinned in PRD-013 (same states the Swift
/// app uses): `starting | working | waiting | permission | stopped | error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Session was just spawned; claude is still booting.
    Starting,
    /// Claude is actively working (UI shows "esc to interrupt").
    Working,
    /// Claude is idle at the prompt, waiting for input.
    Waiting,
    /// Claude is blocked on a permission dialog and needs a human answer.
    Permission,
    /// The tmux session no longer exists (finished or killed).
    Stopped,
    /// Something went wrong spawning or driving the session.
    Error,
}

impl SessionStatus {
    /// True when the session still has a live tmux session behind it.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Stopped | Self::Error)
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Permission => "permission",
            Self::Stopped => "stopped",
            Self::Error => "error",
        };
        f.write_str(s)
    }
}

/// How many pane lines the heuristics inspect (from the bottom).
const DETECT_WINDOW: usize = 30;

/// Detect the session status from the last lines of a `tmux capture-pane`.
///
/// Heuristics over Claude Code's TUI (best-effort, ordered by specificity):
/// 1. A permission dialog ("Do you want …", "y/n", numbered Yes options)
///    → [`SessionStatus::Permission`]. Checked first: a dialog needs a human
///    even if older "working" text is still on screen.
/// 2. The busy indicator ("esc to interrupt") → [`SessionStatus::Working`].
/// 3. An empty ready prompt (`>` / `❯` box marker near the bottom)
///    → [`SessionStatus::Waiting`].
///
/// Returns `None` when nothing matches — the caller keeps the current status
/// (e.g. `starting` while the UI is still booting).
pub fn detect_status(lines: &[String]) -> Option<SessionStatus> {
    let window: Vec<&str> = lines
        .iter()
        .rev()
        .take(DETECT_WINDOW)
        .map(|l| l.as_str())
        .collect();
    let joined = window.join("\n").to_lowercase();

    // 1. Permission dialog.
    if joined.contains("do you want")
        || joined.contains("(y/n)")
        || joined.contains("y/n)")
        || joined.contains("don't ask again")
        || joined.contains("1. yes")
    {
        return Some(SessionStatus::Permission);
    }

    // 2. Actively working.
    if joined.contains("esc to interrupt") {
        return Some(SessionStatus::Working);
    }

    // 3. Idle prompt ready for input.
    let has_prompt = window.iter().any(|l| {
        let t = l.trim();
        t == ">"
            || t == "❯"
            || t.starts_with("> ")
            || t.starts_with("❯ ")
            || t.starts_with("│ >")
            || t.starts_with("│ ❯")
    });
    if has_prompt {
        return Some(SessionStatus::Waiting);
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_working_from_esc_to_interrupt() {
        let pane = lines(&["✻ Thinking…", "  (esc to interrupt)"]);
        assert_eq!(detect_status(&pane), Some(SessionStatus::Working));
    }

    #[test]
    fn detects_permission_dialog() {
        let pane = lines(&[
            "Do you want to run this command?",
            "  1. Yes",
            "  2. No, tell Claude what to do differently",
        ]);
        assert_eq!(detect_status(&pane), Some(SessionStatus::Permission));
    }

    #[test]
    fn permission_wins_over_working_text() {
        // A dialog on top of older "esc to interrupt" output still needs a human.
        let pane = lines(&["esc to interrupt", "Do you want to make this edit? (y/n)"]);
        assert_eq!(detect_status(&pane), Some(SessionStatus::Permission));
    }

    #[test]
    fn detects_waiting_from_empty_prompt() {
        let pane = lines(&["Some previous output", "│ > ", "? for shortcuts"]);
        assert_eq!(detect_status(&pane), Some(SessionStatus::Waiting));
    }

    #[test]
    fn detects_waiting_from_bare_prompt_char() {
        let pane = lines(&[">"]);
        assert_eq!(detect_status(&pane), Some(SessionStatus::Waiting));
    }

    #[test]
    fn no_match_returns_none() {
        let pane = lines(&["Loading claude…", "please wait"]);
        assert_eq!(detect_status(&pane), None);
    }

    #[test]
    fn empty_pane_returns_none() {
        assert_eq!(detect_status(&[]), None);
    }

    #[test]
    fn status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SessionStatus::Permission).unwrap(),
            "\"permission\""
        );
        let back: SessionStatus = serde_json::from_str("\"working\"").unwrap();
        assert_eq!(back, SessionStatus::Working);
    }

    #[test]
    fn is_active_semantics() {
        assert!(SessionStatus::Starting.is_active());
        assert!(SessionStatus::Working.is_active());
        assert!(SessionStatus::Waiting.is_active());
        assert!(SessionStatus::Permission.is_active());
        assert!(!SessionStatus::Stopped.is_active());
        assert!(!SessionStatus::Error.is_active());
    }
}
