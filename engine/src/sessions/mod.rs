//! Orchestrated Claude Code sessions over tmux (PRD-013).
//!
//! This module lets the engine spawn, direct and observe interactive
//! `claude` CLI sessions running inside tmux:
//!
//! - [`SessionBackend`]: trait over the tmux operations (fake-able in tests).
//! - [`TmuxBackend`]: real implementation via shell-out to `tmux`.
//! - [`SessionManager`]: create / list / send / stop sessions, with a
//!   persistent JSON registry (`~/.openmirai/orchestrator_sessions.json`)
//!   that survives server restarts and is reconciled against real tmux
//!   state on startup.
//! - [`SessionStatus`] + [`detect_status`]: heuristic status detection by
//!   inspecting `tmux capture-pane` output of the Claude Code UI.
//!
//! Each orchestrated session is one tmux session named `mirai-<id>`.
//! A human can always take over manually with `tmux attach -t mirai-<id>`.

mod backend;
pub mod hooks;
mod manager;
mod status;

pub use backend::{SessionBackend, TmuxBackend};
pub use manager::{SessionManager, SessionRecord, SpawnParams};
pub use status::{detect_status, SessionStatus};

#[cfg(test)]
pub(crate) use backend::fake::FakeBackend;

/// Errors from session orchestration.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session not found: {0}")]
    NotFound(String),

    #[error("session is stopped: {0}")]
    Stopped(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("registry io error: {0}")]
    Io(#[from] std::io::Error),
}
