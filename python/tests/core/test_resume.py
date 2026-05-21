"""Tests for GraphRunner.resume_from_state — checkpoint resume behaviour."""

from __future__ import annotations

from typing import Any
from unittest.mock import AsyncMock

import pytest

from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import GraphRunner


# --- Helpers ---


def make_executor(outputs: dict[str, dict]) -> AsyncMock:
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

    return AsyncMock(side_effect=execute)


# --- Tests ---


class TestResumeFromState:
    """Test that resume_from_state restores state and continues execution."""

    @pytest.fixture
    def three_node_graph(self):
        return GraphDef(
            id="g-resume",
            name="ResumeTest",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/manual"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
                NodeDef(id="n3", tool_type="data/db_write"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n3"),
            ],
        )

    @pytest.mark.asyncio
    async def test_resume_from_state(self, three_node_graph):
        """Resume from n2 with n1 state already in snapshot. Only n2+n3 execute."""
        call_order: list[str] = []

        async def execute(node, inputs, context):
            call_order.append(node.id)
            return {"n2": {"response": "resumed"}, "n3": {"written": True}}[node.id]

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)

        # Snapshot has n1 already completed
        snapshot = {"n1": {"payload": "original"}}

        result = await runner.resume_from_state(
            three_node_graph,
            context=None,
            state_snapshot=snapshot,
            entry_node_id="n2",
            start_step=1,
        )

        assert result.status == "completed"
        # n1 should NOT have been executed — only n2 and n3
        assert call_order == ["n2", "n3"]
        # State should contain all three nodes
        assert result.state.get("n1") == {"payload": "original"}
        assert result.state.get("n2") == {"response": "resumed"}
        assert result.state.get("n3") == {"written": True}

    @pytest.mark.asyncio
    async def test_resume_preserves_state(self, three_node_graph):
        """State from snapshot is available to resumed nodes via data_map."""
        graph = GraphDef(
            id="g-resume-state",
            name="ResumePreserve",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/manual"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
            ],
            edges=[
                EdgeDef(
                    id="e1", source="n1", target="n2",
                    data_map={"context": "n1.data"},
                ),
            ],
        )

        received_inputs: dict[str, Any] = {}

        async def execute(node, inputs, context):
            if node.id == "n2":
                received_inputs.update(inputs)
            return {"response": "ok"}

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)

        snapshot = {"n1": {"data": "from-checkpoint"}}

        result = await runner.resume_from_state(
            graph,
            context=None,
            state_snapshot=snapshot,
            entry_node_id="n2",
        )

        assert result.status == "completed"
        assert received_inputs.get("context") == "from-checkpoint"

    @pytest.mark.asyncio
    async def test_resume_generates_new_checkpoints(self, three_node_graph):
        """Resumed execution creates its own checkpoints."""
        checkpoints_saved: list[dict] = []

        async def checkpoint_cb(
            session_id: str, step: int, node_id: str,
            state_snapshot: dict, cursor_position: str | None,
        ) -> str:
            checkpoints_saved.append({
                "session_id": session_id,
                "step": step,
                "node_id": node_id,
                "cursor_position": cursor_position,
            })
            return f"cp-{step}"

        executor = make_executor({
            "n2": {"response": "resumed"},
            "n3": {"written": True},
        })

        runner = GraphRunner(
            executor=executor,
            session_id="new-session-123",
            checkpoint_callback=checkpoint_cb,
        )

        snapshot = {"n1": {"payload": "original"}}

        result = await runner.resume_from_state(
            three_node_graph,
            context=None,
            state_snapshot=snapshot,
            entry_node_id="n2",
            start_step=1,
        )

        assert result.status == "completed"
        # Should have created checkpoints for n2 and n3
        assert len(checkpoints_saved) >= 2
        # All checkpoints reference the new session
        for cp in checkpoints_saved:
            assert cp["session_id"] == "new-session-123"

    @pytest.mark.asyncio
    async def test_resume_from_middle_with_conditional(self):
        """Resume from a node that has conditional outgoing edges."""
        graph = GraphDef(
            id="g-cond",
            name="CondResume",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/manual"),
                NodeDef(id="n2", tool_type="logic/condition"),
                NodeDef(id="n3", tool_type="ai/llm_call"),
                NodeDef(id="n4", tool_type="data/db_write"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(
                    id="e2", source="n2", target="n3",
                    condition={"field": "route", "op": "eq", "value": "a"},
                ),
                EdgeDef(
                    id="e3", source="n2", target="n4",
                    condition={"field": "route", "op": "eq", "value": "b"},
                ),
            ],
        )

        executor = make_executor({
            "n2": {"route": "b"},
            "n4": {"saved": True},
        })

        runner = GraphRunner(executor=executor)

        snapshot = {"n1": {"started": True}}

        result = await runner.resume_from_state(
            graph,
            context=None,
            state_snapshot=snapshot,
            entry_node_id="n2",
        )

        assert result.status == "completed"
        assert "n4" in result.state
        assert "n3" not in result.state
