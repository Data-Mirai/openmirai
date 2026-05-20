"""Tests for GraphRunner — cursor execution, edge eval, retry, loop detection."""

from __future__ import annotations

from typing import Any
from unittest.mock import AsyncMock

import pytest

from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import (
    ToolExecutor,
    GraphExecutionError,
    GraphRunner,
    MaxIterationsError,
)

# --- Helpers ---


def make_executor(outputs: dict[str, dict]) -> ToolExecutor:
    """Create a mock executor that returns pre-defined outputs per node_id."""

    async def execute(
        node: NodeDef, inputs: dict[str, Any], context: Any
    ) -> dict[str, Any]:
        if node.id in outputs:
            val = outputs[node.id]
            if callable(val):
                return val()
            return val
        return {}

    mock = AsyncMock(side_effect=execute)
    return mock


def failing_executor(
    fail_nodes: dict[str, int], outputs: dict[str, dict] | None = None
) -> ToolExecutor:
    """Executor that fails N times for specific nodes, then succeeds."""
    counters: dict[str, int] = {k: 0 for k in fail_nodes}
    outputs = outputs or {}

    async def execute(
        node: NodeDef, inputs: dict[str, Any], context: Any
    ) -> dict[str, Any]:
        if node.id in fail_nodes:
            counters[node.id] += 1
            if counters[node.id] <= fail_nodes[node.id]:
                raise RuntimeError(f"Block {node.id} failed (attempt {counters[node.id]})")
        return outputs.get(node.id, {})

    return AsyncMock(side_effect=execute)


# --- Linear graph execution ---


class TestLinearExecution:
    @pytest.fixture
    def linear_graph(self):
        return GraphDef(
            id="g1",
            name="Linear",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
                NodeDef(id="n3", tool_type="data/db_write"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n3"),
            ],
        )

    @pytest.mark.asyncio
    async def test_linear_execution(self, linear_graph):
        executor = make_executor({
            "n1": {"payload": "hello"},
            "n2": {"response": "world"},
            "n3": {"written": True},
        })
        runner = GraphRunner(executor=executor)
        result = await runner.run(linear_graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert len(result.trace) == 3
        assert result.state.get("n1") == {"payload": "hello"}
        assert result.state.get("n2") == {"response": "world"}
        assert result.state.get("n3") == {"written": True}

    @pytest.mark.asyncio
    async def test_executor_called_in_order(self, linear_graph):
        call_order = []
        outputs = {"n1": {"a": 1}, "n2": {"b": 2}, "n3": {"c": 3}}

        async def execute(node, inputs, context):
            call_order.append(node.id)
            return outputs[node.id]

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)
        await runner.run(linear_graph, context=None, entry_node_id="n1")

        assert call_order == ["n1", "n2", "n3"]

    @pytest.mark.asyncio
    async def test_single_node_graph(self):
        graph = GraphDef(
            id="g1",
            name="Single",
            nodes=[NodeDef(id="n1", tool_type="trigger/manual")],
        )
        executor = make_executor({"n1": {"done": True}})
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert len(result.trace) == 1

    @pytest.mark.asyncio
    async def test_invalid_entry_node(self, linear_graph):
        executor = make_executor({})
        runner = GraphRunner(executor=executor)
        with pytest.raises(GraphExecutionError, match="not found"):
            await runner.run(linear_graph, context=None, entry_node_id="n999")


# --- Conditional branching ---


class TestConditionalBranching:
    @pytest.fixture
    def branching_graph(self):
        return GraphDef(
            id="g1",
            name="Branch",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="logic/condition"),
                NodeDef(id="n3", tool_type="ai/llm_call"),
                NodeDef(id="n4", tool_type="data/db_write"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(
                    id="e2",
                    source="n2",
                    target="n3",
                    condition={"field": "status", "op": "eq", "value": "success"},
                ),
                EdgeDef(
                    id="e3",
                    source="n2",
                    target="n4",
                    condition={"field": "status", "op": "eq", "value": "error"},
                ),
            ],
        )

    @pytest.mark.asyncio
    async def test_takes_true_branch(self, branching_graph):
        executor = make_executor({
            "n1": {"data": "x"},
            "n2": {"status": "success"},
            "n3": {"result": "ok"},
        })
        runner = GraphRunner(executor=executor)
        result = await runner.run(branching_graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert "n3" in result.state
        assert "n4" not in result.state

    @pytest.mark.asyncio
    async def test_takes_other_branch(self, branching_graph):
        executor = make_executor({
            "n1": {"data": "x"},
            "n2": {"status": "error"},
            "n4": {"logged": True},
        })
        runner = GraphRunner(executor=executor)
        result = await runner.run(branching_graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert "n4" in result.state
        assert "n3" not in result.state

    @pytest.mark.asyncio
    async def test_no_matching_condition_raises(self, branching_graph):
        executor = make_executor({
            "n1": {"data": "x"},
            "n2": {"status": "unknown"},
        })
        runner = GraphRunner(executor=executor)
        with pytest.raises(GraphExecutionError, match="No matching edge"):
            await runner.run(branching_graph, context=None, entry_node_id="n1")

    @pytest.mark.asyncio
    async def test_unconditional_edge_as_default(self):
        graph = GraphDef(
            id="g1",
            name="Default Edge",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(id="n2", tool_type="b"),
                NodeDef(id="n3", tool_type="c"),
            ],
            edges=[
                EdgeDef(
                    id="e1",
                    source="n1",
                    target="n2",
                    condition={"field": "x", "op": "eq", "value": "match"},
                ),
                EdgeDef(id="e2", source="n1", target="n3"),  # unconditional = default
            ],
        )
        executor = make_executor({
            "n1": {"x": "no_match"},
            "n3": {"fallback": True},
        })
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert "n3" in result.state
        assert "n2" not in result.state


# --- Edge condition operators ---


class TestEdgeConditionOperators:
    @pytest.fixture
    def make_graph_with_condition(self):
        def _make(condition: dict):
            return GraphDef(
                id="g1",
                name="OpTest",
                nodes=[
                    NodeDef(id="n1", tool_type="a"),
                    NodeDef(id="n2", tool_type="b"),
                ],
                edges=[
                    EdgeDef(id="e1", source="n1", target="n2", condition=condition),
                ],
            )
        return _make

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "op, field_val, cond_val, should_match",
        [
            ("eq", "hello", "hello", True),
            ("eq", "hello", "world", False),
            ("neq", "hello", "world", True),
            ("neq", "hello", "hello", False),
            ("gt", 10, 5, True),
            ("gt", 5, 10, False),
            ("lt", 5, 10, True),
            ("lt", 10, 5, False),
            ("gte", 10, 10, True),
            ("gte", 9, 10, False),
            ("lte", 10, 10, True),
            ("lte", 11, 10, False),
            ("in", "a", ["a", "b", "c"], True),
            ("in", "z", ["a", "b", "c"], False),
            ("contains", [1, 2, 3], 2, True),
            ("contains", [1, 2, 3], 9, False),
        ],
    )
    async def test_operator(
        self, make_graph_with_condition, op, field_val, cond_val, should_match
    ):
        graph = make_graph_with_condition({"field": "val", "op": op, "value": cond_val})
        executor = make_executor({"n1": {"val": field_val}, "n2": {"ok": True}})
        runner = GraphRunner(executor=executor)

        if should_match:
            result = await runner.run(graph, context=None, entry_node_id="n1")
            assert "n2" in result.state
        else:
            with pytest.raises(GraphExecutionError, match="No matching edge"):
                await runner.run(graph, context=None, entry_node_id="n1")


# --- Data map ---


class TestDataMap:
    @pytest.mark.asyncio
    async def test_data_map_resolves_fields(self):
        graph = GraphDef(
            id="g1",
            name="DataMap",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
            ],
            edges=[
                EdgeDef(
                    id="e1",
                    source="n1",
                    target="n2",
                    data_map={"prompt": "n1.text", "context": "n1.metadata"},
                ),
            ],
        )
        received_inputs = {}

        async def execute(node, inputs, context):
            if node.id == "n2":
                received_inputs.update(inputs)
            return {"n1": {"text": "hello", "metadata": "ctx"}, "n2": {"response": "ok"}}[node.id]

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)
        await runner.run(graph, context=None, entry_node_id="n1")

        assert received_inputs["prompt"] == "hello"
        assert received_inputs["context"] == "ctx"


# --- Retry policy ---


class TestRetryPolicy:
    @pytest.mark.asyncio
    async def test_retry_succeeds_after_failures(self):
        graph = GraphDef(
            id="g1",
            name="Retry",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(
                    id="n2",
                    tool_type="b",
                    config={
                        "retry_policy": {
                            "max_retries": 3,
                            "backoff": "none",
                            "initial_delay_seconds": 0,
                            "on_failure": "stop",
                        }
                    },
                ),
            ],
            edges=[EdgeDef(id="e1", source="n1", target="n2")],
        )
        executor = failing_executor(
            fail_nodes={"n2": 2},
            outputs={"n1": {"a": 1}, "n2": {"b": 2}},
        )
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert result.state.get("n2") == {"b": 2}

    @pytest.mark.asyncio
    async def test_retry_exhausted_stop(self):
        graph = GraphDef(
            id="g1",
            name="RetryFail",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(
                    id="n2",
                    tool_type="b",
                    config={
                        "retry_policy": {
                            "max_retries": 2,
                            "backoff": "none",
                            "initial_delay_seconds": 0,
                            "on_failure": "stop",
                        }
                    },
                ),
            ],
            edges=[EdgeDef(id="e1", source="n1", target="n2")],
        )
        executor = failing_executor(
            fail_nodes={"n2": 10},
            outputs={"n1": {"a": 1}},
        )
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "failed"

    @pytest.mark.asyncio
    async def test_retry_exhausted_skip(self):
        graph = GraphDef(
            id="g1",
            name="RetrySkip",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(
                    id="n2",
                    tool_type="b",
                    config={
                        "retry_policy": {
                            "max_retries": 1,
                            "backoff": "none",
                            "initial_delay_seconds": 0,
                            "on_failure": "skip",
                        }
                    },
                ),
                NodeDef(id="n3", tool_type="c"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n3"),
            ],
        )
        executor = failing_executor(
            fail_nodes={"n2": 10},
            outputs={"n1": {"a": 1}, "n3": {"c": 3}},
        )
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert result.state.get("n3") == {"c": 3}

    @pytest.mark.asyncio
    async def test_retry_exhausted_route_to_error(self):
        graph = GraphDef(
            id="g1",
            name="RetryRoute",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(
                    id="n2",
                    tool_type="b",
                    config={
                        "retry_policy": {
                            "max_retries": 1,
                            "backoff": "none",
                            "initial_delay_seconds": 0,
                            "on_failure": "route_to_error",
                        }
                    },
                ),
                NodeDef(id="n3", tool_type="c"),
                NodeDef(id="n_err", tool_type="notify/error"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n3"),
                EdgeDef(
                    id="e_err",
                    source="n2",
                    target="n_err",
                    condition={"type": "on_error"},
                ),
            ],
        )
        executor = failing_executor(
            fail_nodes={"n2": 10},
            outputs={"n1": {"a": 1}, "n_err": {"notified": True}},
        )
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert "n_err" in result.state
        assert "n3" not in result.state


# --- Loop detection ---


class TestLoopDetection:
    @pytest.mark.asyncio
    async def test_loop_respects_max_iterations(self):
        graph = GraphDef(
            id="g1",
            name="Loop",
            metadata={"max_iterations": 5},
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(id="n2", tool_type="b"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n1"),
            ],
        )
        call_count = {"n1": 0, "n2": 0}

        async def execute(node, inputs, context):
            call_count[node.id] += 1
            return {"iter": call_count[node.id]}

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)

        with pytest.raises(MaxIterationsError):
            await runner.run(graph, context=None, entry_node_id="n1")

    @pytest.mark.asyncio
    async def test_loop_with_exit_condition(self):
        graph = GraphDef(
            id="g1",
            name="LoopExit",
            metadata={"max_iterations": 10},
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(id="n2", tool_type="b"),
                NodeDef(id="n3", tool_type="c"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(
                    id="e2",
                    source="n2",
                    target="n1",
                    condition={"field": "done", "op": "eq", "value": False},
                ),
                EdgeDef(
                    id="e3",
                    source="n2",
                    target="n3",
                    condition={"field": "done", "op": "eq", "value": True},
                ),
            ],
        )
        counter = {"val": 0}

        async def execute(node, inputs, context):
            if node.id == "n2":
                counter["val"] += 1
                return {"done": counter["val"] >= 3}
            if node.id == "n3":
                return {"final": True}
            return {"start": True}

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert result.status == "completed"
        assert result.state.get("n3") == {"final": True}


# --- ExecutionResult and trace ---


class TestExecutionResult:
    @pytest.mark.asyncio
    async def test_trace_contains_all_nodes(self):
        graph = GraphDef(
            id="g1",
            name="Trace",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(id="n2", tool_type="b"),
            ],
            edges=[EdgeDef(id="e1", source="n1", target="n2")],
        )
        executor = make_executor({"n1": {"x": 1}, "n2": {"y": 2}})
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")

        assert len(result.trace) == 2
        assert result.trace[0]["node_id"] == "n1"
        assert result.trace[1]["node_id"] == "n2"
        assert result.trace[0]["status"] == "success"
        assert "started_at" in result.trace[0]
        assert "finished_at" in result.trace[0]
