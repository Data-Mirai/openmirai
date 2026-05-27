//! Graph execution engine — runs agentic DAG workflows.
//!
//! Split into focused modules:
//! - [`types`]: All data types (errors, policies, results, traces)
//! - [`traits`]: Hook, checkpoint, and executor abstractions
//! - [`graph_runner`]: The `GraphRunner` struct and execution loop
//! - [`helpers`]: Shared utility functions

mod graph_runner;
mod helpers;
mod traits;
mod types;

mod tests;

// Re-export everything that was public before the split.
// External code using `use datamirai_engine::GraphRunner` or
// `use datamirai_engine::core::runner::*` continues to work unchanged.

pub use graph_runner::GraphRunner;
pub use traits::{CheckpointCallback, HookHandler, ToolExecutor};
pub use types::{
    BackoffStrategy, Checkpoint, ExecutionResult, ExecutionStatus, FailureMode, HookResult,
    InterruptInfo, RetryPolicy, RunnerError, SharedState, ToolError, TraceEntry, TraceStatus,
    TranscriptEntry,
};
