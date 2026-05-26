"""Data Mirai Engine — Open source motor for executing agentic graphs.

Quick start (FEAT-034):

    from datamirai_engine import AgentSpec

    agent = AgentSpec.from_yaml("agent.yaml")
    result = await agent.run({"query": "hello"})

Full control:

    from datamirai_engine import GraphDef, GraphRunner, ToolRegistry, RegistryExecutor
    from datamirai_engine import SimpleExecutionContext, AuthContext

    registry = ToolRegistry.default()  # all builtins pre-registered
    runner = GraphRunner(executor=RegistryExecutor(registry))
    context = SimpleExecutionContext(
        db=your_db, vector=your_vector, storage=your_storage,
        llm=your_llm, auth=AuthContext(user_id="u1", role="ADMIN"),
    )
    result = await runner.run(graph, context=context, entry_node_id="trigger")
"""

__version__ = "0.1.0"

# Enums (FEAT-034)
from datamirai_engine.core.enums import (
    Backoff,
    ConfigFieldType,
    DataType,
    OnFailure,
    Op,
    SessionStatus,
    TriggerType,
)

# Core — graph definition and execution
from datamirai_engine.core.agent_spec import AgentSpec

# Blocks — block system
from datamirai_engine.tools.base import BaseTool, ConfigField, ToolInput, ToolOutput, ToolSpec, tool
from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.auth import Permission, PermissionEvaluator, SingleUserAuth
from datamirai_engine.core.context import (
    AuthContext,
    DBResource,
    ExecutionContext,
    LLMResource,
    MemoryResource,
    StorageResource,
    VectorResource,
)
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import (
    ToolExecutor,
    ExecutionResult,
    GraphExecutionError,
    GraphRunner,
    MaxIterationsError,
    RegistryExecutor,
    RetryPolicy,
)
from datamirai_engine.core.state import SharedState
from datamirai_engine.memory.long_term import LongTermMemory, SharedLog

# Memory
from datamirai_engine.memory.short_term import ShortTermMemory

# Resources — concrete implementations
from datamirai_engine.resources.context import SimpleExecutionContext

# Runtime
from datamirai_engine.runtime.agent_runtime import AgentRuntime
from datamirai_engine.runtime.scheduler import Scheduler

__all__ = [
    # Enums (FEAT-034)
    "Backoff",
    "ConfigFieldType",
    "DataType",
    "OnFailure",
    "Op",
    "SessionStatus",
    "TriggerType",
    # Runtime
    "AgentRuntime",
    # Core
    "AgentSpec",
    "AuthContext",
    # Blocks
    "BaseTool",
    "ToolExecutor",
    "ToolInput",
    "ToolOutput",
    "ToolRegistry",
    "ToolSpec",
    "ConfigField",
    "tool",
    "DBResource",
    "EdgeDef",
    "ExecutionContext",
    "ExecutionResult",
    "GraphDef",
    "GraphExecutionError",
    "GraphRunner",
    "LLMResource",
    # Memory
    "LongTermMemory",
    "MaxIterationsError",
    "MemoryResource",
    "NodeDef",
    "Permission",
    "PermissionEvaluator",
    "RegistryExecutor",
    "RetryPolicy",
    "Scheduler",
    "SharedLog",
    "SharedState",
    "ShortTermMemory",
    # Resources
    "SimpleExecutionContext",
    "SingleUserAuth",
    "StorageResource",
    "VectorResource",
    # Version
    "__version__",
]
