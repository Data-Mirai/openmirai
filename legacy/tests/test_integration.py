"""Integration tests — End-to-end graph execution with real blocks.

These tests validate that GraphRunner + ToolRegistry + real blocks
work together correctly for complete graph flows.
"""

from __future__ import annotations

import pytest

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec
from datamirai_engine.tools.builtin.logic.condition import ConditionTool
from datamirai_engine.tools.builtin.logic.merge import MergeTool
from datamirai_engine.tools.builtin.logic.wait import WaitTool
from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import GraphRunner, RegistryExecutor

# --- Custom blocks for integration tests ---


class TriggerWebhookBlock(BaseTool):
    spec = ToolSpec(
        tool_type="trigger/webhook",
        version="1.0.0",
        display_name="Webhook Trigger",
        description="Simulates webhook trigger",
        category="trigger",
        outputs=[ToolOutput(name="payload", type="object")],
    )

    async def execute(self, inputs, config, context):
        return {"payload": config.get("mock_payload", {"event": "test"})}


class TransformBlock(BaseTool):
    spec = ToolSpec(
        tool_type="test/transform",
        version="1.0.0",
        display_name="Transform",
        description="Transforms input data",
        category="test",
        inputs=[ToolInput(name="data", type="object", required=True)],
        outputs=[ToolOutput(name="result", type="string")],
    )

    async def execute(self, inputs, config, context):
        data = inputs["data"]
        return {"result": f"processed:{data}"}


class CounterBlock(BaseTool):
    """Stateful block for testing loops — counts via config."""

    spec = ToolSpec(
        tool_type="test/counter",
        version="1.0.0",
        display_name="Counter",
        description="Increments counter",
        category="test",
        inputs=[ToolInput(name="count", type="number", required=False, default=0)],
        outputs=[
            ToolOutput(name="count", type="number"),
            ToolOutput(name="done", type="boolean"),
        ],
    )

    async def execute(self, inputs, config, context):
        count = inputs.get("count", 0) + 1
        target = config.get("target", 3)
        return {"count": count, "done": count >= target}


class ErrorBlock(BaseTool):
    spec = ToolSpec(
        tool_type="test/error_handler",
        version="1.0.0",
        display_name="Error Handler",
        description="Handles errors",
        category="test",
        outputs=[ToolOutput(name="handled", type="boolean")],
    )

    async def execute(self, inputs, config, context):
        return {"handled": True, "error_info": inputs.get("__error__", "unknown")}


class FailingBlock(BaseTool):
    spec = ToolSpec(
        tool_type="test/failing",
        version="1.0.0",
        display_name="Failing Block",
        description="Always fails",
        category="test",
    )

    async def execute(self, inputs, config, context):
        raise RuntimeError("Intentional failure")


# --- Fixtures ---


@pytest.fixture
def registry():
    reg = ToolRegistry()
    reg.register(TriggerWebhookBlock)
    reg.register(ConditionTool)
    reg.register(MergeTool)
    reg.register(WaitTool)
    reg.register(TransformBlock)
    reg.register(CounterBlock)
    reg.register(ErrorBlock)
    reg.register(FailingBlock)
    return reg


@pytest.fixture
def runner(registry):
    executor = RegistryExecutor(registry)
    return GraphRunner(executor=executor)


# --- E2E: Linear flow ---


class TestE2ELinearFlow:
    """Webhook → Transform → done."""

    @pytest.mark.asyncio
    async def test_simple_linear(self, runner):
        graph = GraphDef(
            id="e2e-linear",
            name="Linear Flow",
            nodes=[
                NodeDef(
                    id="trigger",
                    tool_type="trigger/webhook",
                    config={"mock_payload": {"name": "test"}},
                ),
                NodeDef(id="transform", tool_type="test/transform"),
            ],
            edges=[
                EdgeDef(
                    id="e1",
                    source="trigger",
                    target="transform",
                    data_map={"data": "trigger.payload"},
                ),
            ],
        )
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert result.state.get("trigger")["payload"] == {"name": "test"}
        assert result.state.get("transform")["result"] == "processed:{'name': 'test'}"
        assert len(result.trace) == 2
        assert all(t["status"] == "success" for t in result.trace)


# --- E2E: Conditional branching ---


class TestE2EConditionalBranching:
    """Webhook → Condition → (true: Transform) | (false: Merge)."""

    def _make_graph(self, payload_value: str) -> GraphDef:
        return GraphDef(
            id="e2e-branch",
            name="Branch Flow",
            nodes=[
                NodeDef(
                    id="trigger",
                    tool_type="trigger/webhook",
                    config={"mock_payload": {"status": payload_value}},
                ),
                NodeDef(
                    id="check",
                    tool_type="logic/condition",
                    config={"operator": "eq", "compare_to": "success"},
                ),
                NodeDef(id="happy_path", tool_type="test/transform"),
                NodeDef(id="sad_path", tool_type="test/transform"),
            ],
            edges=[
                EdgeDef(
                    id="e1",
                    source="trigger",
                    target="check",
                    data_map={"value": "trigger.payload.status"},
                ),
                EdgeDef(
                    id="e2",
                    source="check",
                    target="happy_path",
                    condition={"field": "result", "op": "eq", "value": True},
                    data_map={"data": "trigger.payload"},
                ),
                EdgeDef(
                    id="e3",
                    source="check",
                    target="sad_path",
                    condition={"field": "result", "op": "eq", "value": False},
                    data_map={"data": "trigger.payload"},
                ),
            ],
        )

    @pytest.mark.asyncio
    async def test_takes_happy_path(self, runner):
        graph = self._make_graph("success")
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert "happy_path" in result.state
        assert "sad_path" not in result.state

    @pytest.mark.asyncio
    async def test_takes_sad_path(self, runner):
        graph = self._make_graph("error")
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert "sad_path" in result.state
        assert "happy_path" not in result.state


# --- E2E: Error handling with route_to_error ---


class TestE2EErrorHandling:
    """Trigger → FailingBlock (route_to_error) → ErrorHandler."""

    @pytest.mark.asyncio
    async def test_error_routes_to_handler(self, runner):
        graph = GraphDef(
            id="e2e-error",
            name="Error Flow",
            nodes=[
                NodeDef(
                    id="trigger",
                    tool_type="trigger/webhook",
                    config={"mock_payload": {"data": "x"}},
                ),
                NodeDef(
                    id="risky",
                    tool_type="test/failing",
                    config={
                        "retry_policy": {
                            "max_retries": 1,
                            "backoff": "none",
                            "initial_delay_seconds": 0,
                            "on_failure": "route_to_error",
                        }
                    },
                ),
                NodeDef(id="handler", tool_type="test/error_handler"),
            ],
            edges=[
                EdgeDef(id="e1", source="trigger", target="risky"),
                EdgeDef(
                    id="e_err",
                    source="risky",
                    target="handler",
                    condition={"type": "on_error"},
                ),
            ],
        )
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert result.state.get("handler")["handled"] is True
        # Verify trace shows the failure
        risky_trace = [t for t in result.trace if t["node_id"] == "risky"]
        assert risky_trace[0]["status"] == "failed"


# --- E2E: Loop with exit condition ---


class TestE2ELoop:
    """Counter loops until done=True, then exits to final node."""

    @pytest.mark.asyncio
    async def test_loop_exits_after_target(self, runner):
        graph = GraphDef(
            id="e2e-loop",
            name="Loop Flow",
            metadata={"max_iterations": 10},
            nodes=[
                NodeDef(
                    id="trigger",
                    tool_type="trigger/webhook",
                    config={"mock_payload": {"start": True}},
                ),
                NodeDef(
                    id="counter",
                    tool_type="test/counter",
                    config={"target": 3},
                ),
                NodeDef(id="done", tool_type="logic/merge"),
            ],
            edges=[
                EdgeDef(id="e1", source="trigger", target="counter"),
                EdgeDef(
                    id="e_loop",
                    source="counter",
                    target="counter",
                    condition={"field": "done", "op": "eq", "value": False},
                    data_map={"count": "counter.count"},
                ),
                EdgeDef(
                    id="e_exit",
                    source="counter",
                    target="done",
                    condition={"field": "done", "op": "eq", "value": True},
                ),
            ],
        )
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert result.state.get("counter")["done"] is True
        assert result.state.get("counter")["count"] == 3
        assert "done" in result.state


# --- E2E: Full pipeline ---


class TestE2EFullPipeline:
    """Trigger → Condition → (true: Transform → Merge) | (false: Wait → Merge) → done.

    Validates a complete diamond-shaped graph with conditional branching
    converging at a merge point.
    """

    @pytest.mark.asyncio
    async def test_diamond_graph_true_path(self, runner):
        graph = GraphDef(
            id="e2e-diamond",
            name="Diamond Flow",
            nodes=[
                NodeDef(
                    id="trigger",
                    tool_type="trigger/webhook",
                    config={"mock_payload": {"type": "fast"}},
                ),
                NodeDef(
                    id="check",
                    tool_type="logic/condition",
                    config={"operator": "eq", "compare_to": "fast"},
                ),
                NodeDef(id="fast_path", tool_type="test/transform"),
                NodeDef(id="slow_path", tool_type="logic/wait", config={"delay_seconds": 0}),
                NodeDef(id="merge", tool_type="logic/merge"),
            ],
            edges=[
                EdgeDef(
                    id="e1",
                    source="trigger",
                    target="check",
                    data_map={"value": "trigger.payload.type"},
                ),
                EdgeDef(
                    id="e_fast",
                    source="check",
                    target="fast_path",
                    condition={"field": "result", "op": "eq", "value": True},
                    data_map={"data": "trigger.payload"},
                ),
                EdgeDef(
                    id="e_slow",
                    source="check",
                    target="slow_path",
                    condition={"field": "result", "op": "eq", "value": False},
                ),
                EdgeDef(id="e_fast_merge", source="fast_path", target="merge"),
                EdgeDef(id="e_slow_merge", source="slow_path", target="merge"),
            ],
        )
        result = await runner.run(graph, context=None, entry_node_id="trigger")

        assert result.status == "completed"
        assert "fast_path" in result.state
        assert "slow_path" not in result.state
        assert "merge" in result.state
        # Trace should have: trigger → check → fast_path → merge
        node_ids = [t["node_id"] for t in result.trace]
        assert node_ids == ["trigger", "check", "fast_path", "merge"]
