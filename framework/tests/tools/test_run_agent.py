"""Tests for agent/run_agent tool — sub-graph execution."""

from __future__ import annotations

import pytest

from datamirai_engine.tools.builtin.agent.run_agent import (
    CircularAgentReference,
    NestingDepthExceeded,
    RunAgentTool,
    _MAX_NESTING_DEPTH,
    _NESTING_KEY,
)
from datamirai_engine.resources.context import SimpleExecutionContext


@pytest.fixture
def ctx():
    return SimpleExecutionContext.default()


def _ctx_with_chain(chain: list[str]) -> SimpleExecutionContext:
    """Create a context with an agent call chain in metadata."""
    ctx = SimpleExecutionContext.default()
    ctx.metadata = {_NESTING_KEY: chain}  # type: ignore[attr-defined]
    return ctx


class TestRunAgentSpec:
    def test_spec_metadata(self):
        assert RunAgentTool.spec.tool_type == "agent/run_agent"
        assert RunAgentTool.spec.category == "agent"
        assert RunAgentTool.spec.icon == "users"
        assert RunAgentTool.spec.version == "1.0.0"

    def test_spec_inputs(self):
        input_names = {i.name for i in RunAgentTool.spec.inputs}
        assert "agent_id" in input_names
        assert "input_data" in input_names

    def test_spec_outputs(self):
        output_names = {o.name for o in RunAgentTool.spec.outputs}
        assert "agent_id" in output_names
        assert "session_id" in output_names
        assert "status" in output_names
        assert "result" in output_names
        assert "duration_ms" in output_names

    def test_spec_config(self):
        config_names = {c.name for c in RunAgentTool.spec.config}
        assert "agent_id" in config_names
        assert "timeout_seconds" in config_names

    def test_spec_intents(self):
        assert len(RunAgentTool.spec.intents) >= 2


class TestRunAgentExecution:
    @pytest.mark.asyncio
    async def test_output_format(self, ctx):
        """Output contains all declared fields."""
        tool = RunAgentTool()
        result = await tool.run(
            {"agent_id": "agent-123"},
            {"timeout_seconds": 60},
            ctx,
        )
        assert "agent_id" in result
        assert result["agent_id"] == "agent-123"
        assert "session_id" in result
        assert "status" in result
        assert "result" in result
        assert "duration_ms" in result
        assert isinstance(result["duration_ms"], (int, float))

    @pytest.mark.asyncio
    async def test_agent_id_from_config(self, ctx):
        """agent_id can come from config when not in inputs."""
        tool = RunAgentTool()
        result = await tool.run(
            {},
            {"agent_id": "agent-from-config"},
            ctx,
        )
        assert result["agent_id"] == "agent-from-config"

    @pytest.mark.asyncio
    async def test_agent_id_input_overrides_config(self, ctx):
        """Input agent_id takes priority over config."""
        tool = RunAgentTool()
        result = await tool.run(
            {"agent_id": "agent-from-input"},
            {"agent_id": "agent-from-config"},
            ctx,
        )
        assert result["agent_id"] == "agent-from-input"

    @pytest.mark.asyncio
    async def test_missing_agent_id_raises(self, ctx):
        tool = RunAgentTool()
        with pytest.raises(ValueError, match=r"agent_id"):
            await tool.run({}, {}, ctx)

    @pytest.mark.asyncio
    async def test_input_data_passed_through(self, ctx):
        """input_data is included in result for app layer to use."""
        tool = RunAgentTool()
        result = await tool.run(
            {"agent_id": "a1", "input_data": {"key": "val"}},
            {},
            ctx,
        )
        assert result["_input_data"] == {"key": "val"}

    @pytest.mark.asyncio
    async def test_timeout_config(self, ctx):
        """Timeout from config is passed through."""
        tool = RunAgentTool()
        result = await tool.run(
            {"agent_id": "a1"},
            {"timeout_seconds": 120},
            ctx,
        )
        assert result["_timeout_seconds"] == 120

    @pytest.mark.asyncio
    async def test_default_timeout(self, ctx):
        """Default timeout is 300 seconds."""
        tool = RunAgentTool()
        result = await tool.run(
            {"agent_id": "a1"},
            {},
            ctx,
        )
        assert result["_timeout_seconds"] == 300


class TestNestingDepthCheck:
    @pytest.mark.asyncio
    async def test_depth_1_ok(self):
        """Depth 1 (parent calling child) is allowed."""
        ctx = _ctx_with_chain(["parent-agent"])
        tool = RunAgentTool()
        result = await tool.run({"agent_id": "child"}, {}, ctx)
        assert result["agent_id"] == "child"

    @pytest.mark.asyncio
    async def test_depth_2_ok(self):
        """Depth 2 (grandchild) is allowed."""
        ctx = _ctx_with_chain(["grandparent", "parent"])
        tool = RunAgentTool()
        result = await tool.run({"agent_id": "child"}, {}, ctx)
        assert result["agent_id"] == "child"

    @pytest.mark.asyncio
    async def test_depth_3_exceeds(self):
        """Depth 3 (great-grandchild) exceeds max nesting (REGLA-68)."""
        ctx = _ctx_with_chain(["a", "b", "c"])
        tool = RunAgentTool()
        with pytest.raises(NestingDepthExceeded, match=r"nesting depth exceeded"):
            await tool.run({"agent_id": "d"}, {}, ctx)

    @pytest.mark.asyncio
    async def test_no_context_is_depth_0(self):
        """No context = top level, depth 0, always OK."""
        tool = RunAgentTool()
        result = await tool.run({"agent_id": "child"}, {}, None)
        assert result["agent_id"] == "child"

    @pytest.mark.asyncio
    async def test_max_nesting_value(self):
        assert _MAX_NESTING_DEPTH == 3


class TestCircularReferenceCheck:
    @pytest.mark.asyncio
    async def test_circular_direct(self):
        """Agent A calling itself is circular (REGLA-70)."""
        ctx = _ctx_with_chain(["agent-A"])
        tool = RunAgentTool()
        with pytest.raises(CircularAgentReference, match=r"Circular"):
            await tool.run({"agent_id": "agent-A"}, {}, ctx)

    @pytest.mark.asyncio
    async def test_circular_indirect(self):
        """A -> B -> A is circular."""
        ctx = _ctx_with_chain(["agent-A", "agent-B"])
        tool = RunAgentTool()
        with pytest.raises(CircularAgentReference, match=r"Circular"):
            await tool.run({"agent_id": "agent-A"}, {}, ctx)

    @pytest.mark.asyncio
    async def test_no_circular_distinct_agents(self):
        """A -> B -> C is fine (no cycle)."""
        ctx = _ctx_with_chain(["agent-A", "agent-B"])
        tool = RunAgentTool()
        result = await tool.run({"agent_id": "agent-C"}, {}, ctx)
        assert result["agent_id"] == "agent-C"

    @pytest.mark.asyncio
    async def test_call_chain_propagated(self):
        """The call chain in output includes the new agent for app layer."""
        ctx = _ctx_with_chain(["parent"])
        tool = RunAgentTool()
        result = await tool.run({"agent_id": "child"}, {}, ctx)
        assert result["_call_chain"] == ["parent", "child"]
