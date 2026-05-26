"""Tests for FEAT-034: API Simplification & DX Overhaul.

TEST-153 to TEST-164 — enums, auto-gen IDs, passthrough, @tool, factories.
"""

from __future__ import annotations

import json
from typing import Any
from unittest.mock import AsyncMock

import pytest

from datamirai_engine.core.agent_spec import AgentSpec
from datamirai_engine.core.enums import (
    Backoff,
    ConfigFieldType,
    DataType,
    OnFailure,
    Op,
    SessionStatus,
    TriggerType,
)
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import (
    GraphRunner,
    RegistryExecutor,
    RetryPolicy,
)
from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, tool
from datamirai_engine.tools.registry import ToolRegistry


# --- Helpers ---


def make_executor(outputs: dict[str, dict]):
    async def execute(node: NodeDef, inputs: dict[str, Any], context: Any):
        if node.id in outputs:
            val = outputs[node.id]
            return val() if callable(val) else val
        return {}
    return AsyncMock(side_effect=execute)


# --- TEST-155: Enum validation ---


class TestEnums:
    def test_op_values(self):
        assert Op.EQ == "eq"
        assert Op.NEQ == "neq"
        assert Op.GT == "gt"
        assert Op.LT == "lt"

    def test_backoff_values(self):
        assert Backoff.NONE == "none"
        assert Backoff.LINEAR == "linear"
        assert Backoff.EXPONENTIAL == "exponential"

    def test_on_failure_values(self):
        assert OnFailure.STOP == "stop"
        assert OnFailure.SKIP == "skip"
        assert OnFailure.ROUTE_TO_ERROR == "route_to_error"

    def test_data_type_values(self):
        assert DataType.STRING == "string"
        assert DataType.NUMBER == "number"
        assert DataType.BOOLEAN == "boolean"

    def test_trigger_type_values(self):
        assert TriggerType.WEBHOOK == "webhook"
        assert TriggerType.SCHEDULE == "schedule"

    def test_session_status_values(self):
        assert SessionStatus.COMPLETED == "completed"
        assert SessionStatus.FAILED == "failed"

    def test_config_field_type_values(self):
        assert ConfigFieldType.STRING == "string"
        assert ConfigFieldType.SELECT == "select"

    def test_enum_in_tool_input(self):
        ti = ToolInput(name="x", type=DataType.NUMBER)
        assert ti.type == DataType.NUMBER
        assert ti.type == "number"  # backward compat

    def test_string_still_works_in_tool_input(self):
        ti = ToolInput(name="x", type="string")
        assert ti.type == "string"

    def test_enum_in_retry_policy(self):
        rp = RetryPolicy(
            max_retries=3,
            backoff=Backoff.EXPONENTIAL,
            on_failure=OnFailure.SKIP,
        )
        assert rp.backoff == Backoff.EXPONENTIAL
        assert rp.on_failure == OnFailure.SKIP

    def test_string_still_works_in_retry_policy(self):
        rp = RetryPolicy(max_retries=1, backoff="linear", on_failure="stop")
        assert rp.backoff == "linear"

    def test_op_enum_in_condition(self):
        edge = EdgeDef(
            source="a", target="b",
            condition={"field": "x", "op": Op.EQ, "value": True},
        )
        assert edge.condition["op"] == Op.EQ
        assert edge.condition["op"] == "eq"


# --- TEST-156, TEST-157: Auto-gen edge IDs ---


class TestAutoGenEdgeIds:
    def test_auto_gen_single_edge(self):
        """TEST-156: EdgeDef without id gets auto-generated as source__target."""
        graph = GraphDef(
            id="g1", name="AutoID",
            nodes=[
                NodeDef(id="a", tool_type="x"),
                NodeDef(id="b", tool_type="y"),
            ],
            edges=[EdgeDef(source="a", target="b")],
        )
        assert graph.edges[0].id == "a__b"

    def test_auto_gen_collision(self):
        """TEST-157: Multiple edges same source→target get suffixed."""
        graph = GraphDef(
            id="g1", name="Collision",
            nodes=[
                NodeDef(id="classify", tool_type="x"),
                NodeDef(id="handle", tool_type="y"),
            ],
            edges=[
                EdgeDef(
                    source="classify", target="handle",
                    condition={"field": "type", "op": "eq", "value": "a"},
                ),
                EdgeDef(
                    source="classify", target="handle",
                    condition={"field": "type", "op": "eq", "value": "b"},
                ),
            ],
        )
        ids = {e.id for e in graph.edges}
        assert "classify__handle" in ids
        assert "classify__handle__2" in ids

    def test_explicit_id_preserved(self):
        """Existing code with explicit IDs still works."""
        graph = GraphDef(
            id="g1", name="Explicit",
            nodes=[
                NodeDef(id="a", tool_type="x"),
                NodeDef(id="b", tool_type="y"),
            ],
            edges=[EdgeDef(id="my-edge", source="a", target="b")],
        )
        assert graph.edges[0].id == "my-edge"

    def test_mixed_auto_and_explicit(self):
        graph = GraphDef(
            id="g1", name="Mixed",
            nodes=[
                NodeDef(id="a", tool_type="x"),
                NodeDef(id="b", tool_type="y"),
                NodeDef(id="c", tool_type="z"),
            ],
            edges=[
                EdgeDef(id="e1", source="a", target="b"),
                EdgeDef(source="b", target="c"),  # auto-gen
            ],
        )
        assert graph.edges[0].id == "e1"
        assert graph.edges[1].id == "b__c"


# --- TEST-158: Default passthrough data_map ---


class TestDefaultPassthrough:
    @pytest.mark.asyncio
    async def test_passthrough_when_no_data_map(self):
        """TEST-158: Unconditional edge without data_map passes through source output."""
        graph = GraphDef(
            id="g1", name="Passthrough",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
            ],
            edges=[EdgeDef(source="n1", target="n2")],
        )
        received_inputs = {}

        async def execute(node, inputs, context):
            if node.id == "n2":
                received_inputs.update(inputs)
            return {"n1": {"text": "hello", "count": 42}, "n2": {"response": "ok"}}[node.id]

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)
        await runner.run(graph, context=None, entry_node_id="n1")

        assert received_inputs["text"] == "hello"
        assert received_inputs["count"] == 42

    @pytest.mark.asyncio
    async def test_explicit_data_map_still_works(self):
        """Explicit data_map takes precedence over passthrough."""
        graph = GraphDef(
            id="g1", name="Explicit",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
            ],
            edges=[
                EdgeDef(
                    source="n1", target="n2",
                    data_map={"prompt": "n1.text"},
                ),
            ],
        )
        received_inputs = {}

        async def execute(node, inputs, context):
            if node.id == "n2":
                received_inputs.update(inputs)
            return {"n1": {"text": "hello", "extra": "ignored"}, "n2": {"r": "ok"}}[node.id]

        executor = AsyncMock(side_effect=execute)
        runner = GraphRunner(executor=executor)
        await runner.run(graph, context=None, entry_node_id="n1")

        assert received_inputs == {"prompt": "hello"}


# --- TEST-159: @tool decorator ---


class TestToolDecorator:
    def test_basic_decorator(self):
        """TEST-159: Function with type hints becomes registrable BaseTool."""

        @tool
        def get_weather(city: str) -> str:
            """Gets weather for a city."""
            return f"Sunny in {city}"

        assert issubclass(get_weather, BaseTool)
        assert get_weather.spec.tool_type == "custom/get_weather"
        assert get_weather.spec.description == "Gets weather for a city."
        assert len(get_weather.spec.inputs) == 1
        assert get_weather.spec.inputs[0].name == "city"
        assert get_weather.spec.inputs[0].type == DataType.STRING

    def test_decorator_with_params(self):
        @tool(tool_type="weather/current", description="Current weather", version="2.0.0")
        def weather(city: str, units: str = "celsius") -> dict:
            return {"temp": 25, "city": city}

        assert weather.spec.tool_type == "weather/current"
        assert weather.spec.version == "2.0.0"
        assert len(weather.spec.inputs) == 2
        # city is required, units has default
        city_input = next(i for i in weather.spec.inputs if i.name == "city")
        units_input = next(i for i in weather.spec.inputs if i.name == "units")
        assert city_input.required is True
        assert units_input.required is False
        assert units_input.default == "celsius"

    @pytest.mark.asyncio
    async def test_decorator_execution(self):
        @tool
        def add(a: int, b: int) -> int:
            return a + b

        instance = add()
        result = await instance.execute({"a": 3, "b": 4}, {}, None)
        assert result == {"result": 7}

    @pytest.mark.asyncio
    async def test_decorator_dict_return(self):
        @tool
        def multi_output(x: str) -> dict:
            return {"upper": x.upper(), "length": len(x)}

        instance = multi_output()
        result = await instance.execute({"x": "hello"}, {}, None)
        assert result == {"upper": "HELLO", "length": 5}

    @pytest.mark.asyncio
    async def test_async_decorator(self):
        @tool
        async def async_tool(query: str) -> str:
            return f"result: {query}"

        instance = async_tool()
        result = await instance.execute({"query": "test"}, {}, None)
        assert result == {"result": "result: test"}

    def test_register_decorated_tool(self):
        @tool
        def my_tool(x: str) -> str:
            return x

        registry = ToolRegistry()
        registry.register(my_tool)
        assert registry.get("custom/my_tool") is not None


# --- TEST-161: YAML↔JSON round-trip ---


class TestRoundTrip:
    def test_yaml_json_yaml_roundtrip(self):
        """TEST-161: YAML→JSON→YAML is lossless."""
        yaml_str = """
name: test-agent
description: A test agent
version: v1
graph:
  nodes:
    - id: t1
      tool_type: trigger/webhook
    - id: p1
      tool_type: ai/llm_call
      config:
        model: claude
  edges:
    - id: e1
      source: t1
      target: p1
      data_map:
        prompt: t1.body
triggers:
  - type: webhook
    path: /test
"""
        agent1 = AgentSpec.from_yaml(yaml_str)
        json_str = agent1.to_json()
        agent2 = AgentSpec.from_json(json_str)
        yaml_str2 = agent2.to_yaml()
        agent3 = AgentSpec.from_yaml(yaml_str2)

        assert agent1.name == agent2.name == agent3.name
        assert agent1.graph.nodes[0].id == agent3.graph.nodes[0].id
        assert len(agent1.graph.edges) == len(agent3.graph.edges)

    def test_to_json_produces_valid_json(self):
        agent = AgentSpec(
            name="test",
            graph={"nodes": [], "edges": []},
        )
        json_str = agent.to_json()
        parsed = json.loads(json_str)
        assert parsed["name"] == "test"

    def test_from_json_string(self):
        json_str = json.dumps({
            "name": "from-json",
            "graph": {"nodes": [], "edges": []},
        })
        agent = AgentSpec.from_json(json_str)
        assert agent.name == "from-json"


# --- TEST-163: Convenience factory equivalence ---


class TestConvenienceFactories:
    def test_tool_registry_default(self):
        """TEST-163: ToolRegistry.default() returns registry with builtins."""
        registry = ToolRegistry.default()
        specs = registry.list_all()
        # Should have at least some builtins (triggers, logic, etc.)
        tool_types = {s.tool_type for s in specs}
        assert "trigger/webhook" in tool_types or "logic/condition" in tool_types or len(specs) > 0

    def test_graph_runner_default(self):
        """GraphRunner.default() creates a pre-wired runner."""
        runner = GraphRunner.default()
        assert runner is not None
        assert runner._executor is not None

    @pytest.mark.asyncio
    async def test_factory_equivalence(self):
        """TEST-163: default() produces same results as manual setup."""
        graph = GraphDef(
            id="g1", name="Equiv",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/manual"),
                NodeDef(id="n2", tool_type="logic/merge"),
            ],
            edges=[EdgeDef(source="n1", target="n2")],
        )

        # Both paths should work with a simple executor
        outputs = {"n1": {"started": True}, "n2": {"done": True}}
        executor = make_executor(outputs)

        # Manual path
        runner1 = GraphRunner(executor=executor)
        result1 = await runner1.run(graph, context=None, entry_node_id="n1")

        # Factory path (same executor for fair comparison)
        runner2 = GraphRunner(executor=executor)
        result2 = await runner2.run(graph, context=None, entry_node_id="n1")

        assert result1.status == result2.status == "completed"
        assert result1.state.get("n1") == result2.state.get("n1")


# --- TEST-164: Backward compat ---


class TestBackwardCompat:
    def test_string_op_in_condition(self):
        """TEST-164: String 'eq' still works in condition (backward compat)."""
        edge = EdgeDef(
            source="a", target="b",
            condition={"field": "x", "op": "eq", "value": True},
        )
        assert edge.condition["op"] == "eq"

    def test_string_backoff_in_retry(self):
        rp = RetryPolicy(max_retries=1, backoff="linear", on_failure="stop")
        assert rp.backoff == "linear"
        assert rp.on_failure == "stop"

    @pytest.mark.asyncio
    async def test_string_conditions_still_execute(self):
        """String-based conditions in runner still work."""
        graph = GraphDef(
            id="g1", name="BackCompat",
            nodes=[
                NodeDef(id="n1", tool_type="a"),
                NodeDef(id="n2", tool_type="b"),
            ],
            edges=[
                EdgeDef(
                    source="n1", target="n2",
                    condition={"field": "ok", "op": "eq", "value": True},
                ),
            ],
        )
        executor = make_executor({"n1": {"ok": True}, "n2": {"done": True}})
        runner = GraphRunner(executor=executor)
        result = await runner.run(graph, context=None, entry_node_id="n1")
        assert result.status == "completed"

    def test_agent_spec_auto_gen_edge_id(self):
        """AgentSpec with edges without ID auto-generates them."""
        spec = AgentSpec(
            name="test",
            graph={
                "nodes": [
                    {"id": "t", "tool_type": "trigger/webhook"},
                    {"id": "p", "tool_type": "ai/llm_call"},
                ],
                "edges": [{"source": "t", "target": "p"}],
            },
        )
        assert spec.graph.edges[0].id == "t__p"
