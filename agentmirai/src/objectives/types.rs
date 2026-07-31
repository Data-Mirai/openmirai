//! Objective SoT types — the fleet's high-level goals and their agent links.
//!
//! An **objective** is a user-stated goal ("ship the SQLite fleet SoT") that the
//! wizard decomposes into agents. The center persists every objective in the
//! `objectives` SQLite table and wires it to the fleet members working on it via
//! the `objective_agents` bridge table, so the fleet view can show which agents
//! belong to which goal.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ObjectiveStatus
// ---------------------------------------------------------------------------

/// Lifecycle status of an objective.
///
/// `Open` — created, not yet being worked. `Working` — agents are on it.
/// `Done` — the goal is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    /// Created, no agent working it yet.
    Open,
    /// Agents are actively working the objective.
    Working,
    /// The objective is complete.
    Done,
}

impl ObjectiveStatus {
    /// Canonical lowercase wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectiveStatus::Open => "open",
            ObjectiveStatus::Working => "working",
            ObjectiveStatus::Done => "done",
        }
    }

    /// Parse a wire string into a status (case-insensitive, whitespace-trimmed),
    /// using the same snake_case mapping serde uses.
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_value::<Self>(serde_json::Value::String(raw.trim().to_lowercase())).ok()
    }
}

impl Default for ObjectiveStatus {
    fn default() -> Self {
        ObjectiveStatus::Open
    }
}

impl std::fmt::Display for ObjectiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Objective — the persisted SoT record
// ---------------------------------------------------------------------------

/// A single objective, exactly as stored in the `objectives` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    /// Stable, server-generated id.
    pub id: String,
    /// Free-text statement of the goal.
    pub text: String,
    /// Lifecycle status.
    pub status: ObjectiveStatus,
    /// Working directory / project the objective is attached to. Empty otherwise.
    pub project_dir: String,
    /// Epoch seconds the objective was created.
    pub created_at: f64,
    /// Epoch seconds of the last mutation (status change, …).
    pub updated_at: f64,
}

// ---------------------------------------------------------------------------
// NewObjective — the create request
// ---------------------------------------------------------------------------

/// Fields needed to create an objective. `id`, `created_at`, `updated_at` are
/// assigned by the store; everything else comes from the caller.
#[derive(Debug, Clone)]
pub struct NewObjective {
    pub text: String,
    /// Empty when unspecified.
    pub project_dir: String,
    /// Defaults to [`ObjectiveStatus::Open`] when the caller omits it.
    pub status: ObjectiveStatus,
    /// Fleet member ids to link to this objective at creation time (may be empty).
    pub agents: Vec<String>,
}
