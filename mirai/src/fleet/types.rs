//! Fleet SoT types — the source-of-truth records for the agent fleet.
//!
//! A **fleet member** is any agent / node / bridge that lives somewhere in the
//! Mirai fleet (this Mac, the VPS, a Claude Code subagent, …) and reports its
//! status back to the center. The center persists the last-known state of every
//! member in the `fleet` SQLite table so the fleet view survives restarts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// FleetStatus
// ---------------------------------------------------------------------------

/// Last-known lifecycle status of a fleet member.
///
/// Reported by the member over the `back-way status` endpoint. `Unknown` is the
/// default for a freshly-registered member that has not reported a real state
/// yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetStatus {
    /// Reachable and alive, no active task.
    Online,
    /// Explicitly reported itself as going away.
    Offline,
    /// Actively working on its objective.
    Working,
    /// Alive but blocked waiting (input / permission / dependency).
    Waiting,
    /// Alive, idle, ready for work.
    Idle,
    /// Finished its objective — the managed run completed successfully.
    Done,
    /// Reported an error condition.
    Error,
    /// No real status reported yet.
    Unknown,
}

impl FleetStatus {
    /// Canonical lowercase wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            FleetStatus::Online => "online",
            FleetStatus::Offline => "offline",
            FleetStatus::Working => "working",
            FleetStatus::Waiting => "waiting",
            FleetStatus::Idle => "idle",
            FleetStatus::Done => "done",
            FleetStatus::Error => "error",
            FleetStatus::Unknown => "unknown",
        }
    }

    /// Parse a wire string into a status via the same snake_case mapping serde
    /// uses. Case-insensitive; surrounding whitespace ignored.
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_value::<Self>(Value::String(raw.trim().to_lowercase())).ok()
    }
}

impl std::fmt::Display for FleetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// FleetMember — the persisted SoT record
// ---------------------------------------------------------------------------

/// A single fleet member, exactly as stored in the `fleet` table and returned
/// by `GET /api/v1/fleet/agents`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetMember {
    /// Stable member id (caller-provided; idempotent upsert key).
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Free classification: `agent` (default) | `node` | `bridge` | …
    pub kind: String,
    /// Host the member runs on (`mac`, `vps`, …). Empty when unspecified.
    pub host: String,
    /// Last-known lifecycle status.
    pub status: FleetStatus,
    /// Short current-activity label. Empty when unspecified.
    pub activity: String,
    /// What the member is trying to accomplish. Empty when unspecified.
    pub objective: String,
    /// Working directory / project the member is attached to. Empty otherwise.
    pub project_dir: String,
    /// Arbitrary JSON metadata blob (defaults to `{}`).
    pub metadata: Value,
    /// Epoch seconds of the first status report.
    pub first_seen: f64,
    /// Epoch seconds of the most recent status report (heartbeat).
    pub last_seen: f64,
}

// ---------------------------------------------------------------------------
// StatusUpdate — the `back-way status` request body
// ---------------------------------------------------------------------------

/// Incoming status report for a fleet member (the "back-way status" contract).
///
/// `id` + `status` are required. Every other field is optional and only
/// overwrites the stored value when present — omitted fields keep whatever the
/// SoT already has, so a lightweight heartbeat can send just `{id, status}`.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusUpdate {
    pub id: String,
    pub status: FleetStatus,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub activity: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

// ---------------------------------------------------------------------------
// FleetQuery — filters for GET /agents
// ---------------------------------------------------------------------------

/// Optional filters for listing fleet members.
#[derive(Debug, Clone, Default)]
pub struct FleetQuery {
    /// Only members with this status.
    pub status: Option<FleetStatus>,
    /// Only members on this host.
    pub host: Option<String>,
    /// Cap on the number of rows returned.
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// FleetEvent — broadcast to the SSE stream
// ---------------------------------------------------------------------------

/// What happened to a member — drives the SSE `event:` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetEventKind {
    /// First time this id is seen.
    Added,
    /// An existing member reported a new status.
    Updated,
    /// A member was removed from the SoT.
    Removed,
}

impl FleetEventKind {
    /// SSE event name emitted for this kind.
    pub fn event_name(&self) -> &'static str {
        match self {
            FleetEventKind::Added => "fleet_member_added",
            FleetEventKind::Updated => "fleet_member_updated",
            FleetEventKind::Removed => "fleet_member_removed",
        }
    }
}

/// An event broadcast to every `/api/v1/fleet/events` subscriber.
#[derive(Debug, Clone)]
pub struct FleetEvent {
    pub kind: FleetEventKind,
    pub member: FleetMember,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrips_through_wire_string() {
        for s in [
            FleetStatus::Online,
            FleetStatus::Offline,
            FleetStatus::Working,
            FleetStatus::Waiting,
            FleetStatus::Idle,
            FleetStatus::Done,
            FleetStatus::Error,
            FleetStatus::Unknown,
        ] {
            assert_eq!(FleetStatus::parse(s.as_str()), Some(s));
            // serde emits the same canonical string.
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
    }

    #[test]
    fn status_parse_is_case_insensitive_and_trims() {
        assert_eq!(FleetStatus::parse("  WORKING "), Some(FleetStatus::Working));
        assert_eq!(FleetStatus::parse("nope"), None);
    }

    #[test]
    fn event_names_are_stable() {
        assert_eq!(FleetEventKind::Added.event_name(), "fleet_member_added");
        assert_eq!(FleetEventKind::Updated.event_name(), "fleet_member_updated");
        assert_eq!(FleetEventKind::Removed.event_name(), "fleet_member_removed");
    }
}
