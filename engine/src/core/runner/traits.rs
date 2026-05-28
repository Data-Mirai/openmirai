//! Trait definitions for execution hooks, checkpoints, and tool execution.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::core::context::ExecutionContext;
use crate::core::graph::{GraphDef, NodeDef};
use crate::core::state::SharedState;

use super::types::{HookResult, ToolError, RunnerError, Checkpoint};
/// Seven interception points during graph execution.
///
/// All methods have default implementations that return [`HookResult::Continue`],
/// so implementors only need to override the hooks they care about.
///
/// ```ignore
/// use datamirai_engine::{HookHandler, HookResult, ToolError};
/// use datamirai_engine::core::graph::NodeDef;
/// use datamirai_engine::core::context::ExecutionContext;
///
/// struct MyHook;
///
/// #[async_trait::async_trait]
/// impl HookHandler for MyHook {
///     // Only override on_error — the other 6 methods use defaults.
///     async fn on_error(
///         &self,
///         _node: &NodeDef,
///         error: &ToolError,
///         _ctx: &dyn ExecutionContext,
///     ) -> HookResult {
///         eprintln!("Error: {error}");
///         HookResult::Continue
///     }
/// }
/// ```
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn on_graph_start(
        &self,
        _graph: &GraphDef,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn on_graph_end(
        &self,
        _graph: &GraphDef,
        _state: &SharedState,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn pre_block_exec(
        &self,
        _node: &NodeDef,
        _inputs: &mut HashMap<String, Value>,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn post_block_exec(
        &self,
        _node: &NodeDef,
        _output: &mut HashMap<String, Value>,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn pre_llm_call(
        &self,
        _node: &NodeDef,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn post_llm_call(
        &self,
        _node: &NodeDef,
        _response: &mut Value,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }

    async fn on_error(
        &self,
        _node: &NodeDef,
        _error: &ToolError,
        _ctx: &dyn ExecutionContext,
    ) -> HookResult {
        HookResult::Continue
    }
}

/// Callback invoked after each successful block to persist state.
#[async_trait]
pub trait CheckpointCallback: Send + Sync {
    async fn save_checkpoint(&self, checkpoint: Checkpoint) -> Result<String, RunnerError>;
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single node.
    ///
    /// The runner provides the node definition, resolved inputs (from incoming
    /// edges / data_map) and the execution context.  The executor looks up the
    /// concrete tool implementation by `node.tool_type` and delegates to it.
    async fn execute(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}
