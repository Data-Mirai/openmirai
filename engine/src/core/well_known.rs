//! Well-known constants used across the engine.
//!
//! Centralises magic strings so that typos become compile errors and
//! semantics are documented in one place.

// ---------------------------------------------------------------------------
// Tool-type identifiers the runner needs to recognise
// ---------------------------------------------------------------------------

/// The human-input tool pauses execution and returns an interrupt.
pub const HUMAN_INPUT_TOOL: &str = "logic/human_input";

/// Tool-type prefix for AI/LLM blocks — used to fire pre/post LLM hooks.
pub const AI_TOOL_PREFIX: &str = "ai/";

// ---------------------------------------------------------------------------
// Special state keys
// ---------------------------------------------------------------------------

/// Key injected into node output when `FailureMode::RouteToError` triggers.
pub const ERROR_FIELD: &str = "__error__";

// ---------------------------------------------------------------------------
// strict_completion (PRD-022)
// ---------------------------------------------------------------------------

/// Prefix every `strict_completion` failure message carries. Stable on
/// purpose: it is the greppable surface for whoever operates a fleet.
pub const STRICT_PREFIX: &str = "strict_completion: ";

// ---------------------------------------------------------------------------
// Transcript entry types
// ---------------------------------------------------------------------------

pub const TRANSCRIPT_STARTED: &str = "started";
pub const TRANSCRIPT_COMPLETED: &str = "completed";
pub const TRANSCRIPT_BLOCK_START: &str = "block_start";
pub const TRANSCRIPT_BLOCK_END: &str = "block_end";
pub const TRANSCRIPT_ERROR: &str = "error";
pub const TRANSCRIPT_DECISION: &str = "decision";
pub const TRANSCRIPT_FANOUT_START: &str = "fanout_start";
pub const TRANSCRIPT_FANOUT_NODE_DONE: &str = "fanout_node_done";
pub const TRANSCRIPT_FANOUT_NODE_ERROR: &str = "fanout_node_error";
pub const TRANSCRIPT_FANOUT_END: &str = "fanout_end";
