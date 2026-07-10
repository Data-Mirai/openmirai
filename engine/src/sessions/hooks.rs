//! Claude Code hooks — the real source of `session_activity` events (PRD-013 M6).
//!
//! The engine materializes two artifacts under `~/.openmirai/hooks/`:
//!
//! 1. `report-activity.py` — a pure-stdlib python3 script registered as a
//!    PostToolUse hook. It reads the hook JSON from stdin, maps the tool to
//!    an action (`Read|Glob|Grep → read`, `Write|Edit|NotebookEdit → write`,
//!    `Bash → exec`, `WebFetch|WebSearch → net`) and POSTs to the engine's
//!    activity endpoint using the
//!    `MIRAI_SESSION_ID` / `MIRAI_PORT` env vars the TmuxBackend injects.
//!    EVERY failure is silent (exit 0 always) — the hook must never block or
//!    slow down the Claude session.
//! 2. `<session-id>-settings.json` — per-session settings passed to
//!    `claude --settings <file>`, registering the hook for the relevant tools.
//!    Format verified against Claude Code's local settings (`hooks` →
//!    `PostToolUse` → `[{matcher, hooks: [{type: "command", command}]}]`).

use std::path::{Path, PathBuf};

use super::SessionError;

/// Tools reported by the hook (used as the PostToolUse matcher).
pub const HOOK_MATCHER: &str = "Read|Glob|Grep|Write|Edit|NotebookEdit|Bash|WebFetch|WebSearch";

/// The PostToolUse reporter script. Pure python3 stdlib, fails silent.
pub const HOOK_SCRIPT: &str = r#"#!/usr/bin/env python3
"""OpenMirai PRD-013 — report Claude Code tool activity to the engine.

Registered as a PostToolUse hook. Reads the hook payload from stdin and
POSTs {tool, action, path} to the engine's orchestrator activity endpoint.
MUST never fail: any problem exits 0 silently (a broken reporter must not
block the Claude session).
"""
import json
import os
import sys
import urllib.request

ACTIONS = {
    "Read": "read",
    "Glob": "read",
    "Grep": "read",
    "Write": "write",
    "Edit": "write",
    "NotebookEdit": "write",
    "Bash": "exec",
    "WebFetch": "net",
    "WebSearch": "net",
}


def main():
    session_id = os.environ.get("MIRAI_SESSION_ID")
    if not session_id:
        return
    port = os.environ.get("MIRAI_PORT", "3000")

    data = json.load(sys.stdin)
    tool = data.get("tool_name") or ""
    action = ACTIONS.get(tool)
    if not action:
        return

    tool_input = data.get("tool_input") or {}
    path = (
        tool_input.get("file_path")
        or tool_input.get("notebook_path")
        or tool_input.get("path")
        or tool_input.get("url")
        or tool_input.get("query")
        or None
    )

    body = json.dumps({"tool": tool, "action": action, "path": path}).encode()
    url = "http://127.0.0.1:%s/api/v1/orchestrator/sessions/%s/activity" % (
        port,
        session_id,
    )
    req = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json"}, method="POST"
    )
    api_key = os.environ.get("MIRAI_API_KEY")
    if api_key:
        req.add_header("X-API-Key", api_key)
    urllib.request.urlopen(req, timeout=2)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        pass
    sys.exit(0)
"#;

/// Directory where hook artifacts live (`<base>/hooks`).
pub fn hooks_dir(base: &Path) -> PathBuf {
    base.join("hooks")
}

/// Write `report-activity.py` if missing or its content changed.
/// Returns the script path.
pub fn ensure_hook_script(base: &Path) -> Result<PathBuf, SessionError> {
    let dir = hooks_dir(base);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("report-activity.py");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current != HOOK_SCRIPT {
        std::fs::write(&path, HOOK_SCRIPT)?;
    }
    Ok(path)
}

/// Write the per-session settings file registering the PostToolUse hook.
/// Returns the settings path to pass as `claude --settings <path>`.
pub fn write_session_settings(base: &Path, session_id: &str) -> Result<PathBuf, SessionError> {
    let script = ensure_hook_script(base)?;
    let settings = serde_json::json!({
        "hooks": {
            "PostToolUse": [
                {
                    "matcher": HOOK_MATCHER,
                    "hooks": [
                        {
                            "type": "command",
                            "command": format!("python3 '{}'", script.display()),
                        }
                    ]
                }
            ]
        }
    });
    let path = hooks_dir(base).join(format!("{session_id}-settings.json"));
    let raw = serde_json::to_string_pretty(&settings)
        .map_err(|e| SessionError::Backend(format!("serialize hook settings: {e}")))?;
    std::fs::write(&path, raw)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mirai-hooks-test-{}", crate::utils::short_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_hook_script_writes_once_and_repairs_drift() {
        let base = temp_base();
        let path = ensure_hook_script(&base).unwrap();
        assert!(path.ends_with("hooks/report-activity.py"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("MIRAI_SESSION_ID"));
        assert!(content.contains("PostToolUse") || content.contains("report Claude Code"));

        // Idempotent: same content → same result.
        let mtime1 = std::fs::metadata(&path).unwrap().modified().unwrap();
        ensure_hook_script(&base).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            mtime1
        );

        // Drift (edited/corrupted) → repaired.
        std::fs::write(&path, "tampered").unwrap();
        ensure_hook_script(&base).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), HOOK_SCRIPT);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn session_settings_registers_post_tool_use_hook() {
        let base = temp_base();
        let path = write_session_settings(&base, "abc123").unwrap();
        assert!(path.ends_with("hooks/abc123-settings.json"));

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = &settings["hooks"]["PostToolUse"][0];
        assert_eq!(entry["matcher"], HOOK_MATCHER);
        assert_eq!(entry["hooks"][0]["type"], "command");
        let command = entry["hooks"][0]["command"].as_str().unwrap();
        assert!(command.starts_with("python3 '"));
        assert!(command.contains("report-activity.py"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn hook_script_maps_tools_and_fails_silent() {
        // The script must be valid python3 and exit 0 on any input.
        let base = temp_base();
        let script = ensure_hook_script(&base).unwrap();

        // Valid payload but no MIRAI_SESSION_ID in env → silent exit 0.
        let out = std::process::Command::new("python3")
            .arg(&script)
            .env_remove("MIRAI_SESSION_ID")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(br#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/x"}}"#)?;
                child.wait_with_output()
            })
            .expect("python3 available");
        assert!(out.status.success(), "hook must exit 0: {out:?}");
        assert!(out.stderr.is_empty(), "hook must be silent: {out:?}");

        // Garbage stdin → still exit 0, still silent.
        let out = std::process::Command::new("python3")
            .arg(&script)
            .env("MIRAI_SESSION_ID", "s1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(b"not json at all")?;
                child.wait_with_output()
            })
            .expect("python3 available");
        assert!(out.status.success(), "hook must exit 0 on garbage: {out:?}");
        assert!(out.stderr.is_empty());
        let _ = std::fs::remove_dir_all(base);
    }
}
