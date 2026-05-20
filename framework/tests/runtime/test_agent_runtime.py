"""Tests for AgentRuntime — agent lifecycle, triggers, scheduling."""

from __future__ import annotations

import asyncio

import pytest

from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.runtime.agent_runtime import AgentRuntime


def _make_registry() -> ToolRegistry:
    reg = ToolRegistry()
    for module in [
        "datamirai_engine.tools.builtin.trigger.triggers",
        "datamirai_engine.tools.builtin.logic.condition",
        "datamirai_engine.tools.builtin.logic.merge",
        "datamirai_engine.tools.builtin.logic.wait",
    ]:
        reg.discover(module)
    return reg


def _make_graph() -> GraphDef:
    return GraphDef(
        id="g1",
        name="Test Graph",
        nodes=[
            NodeDef(id="t1", tool_type="trigger/webhook"),
            NodeDef(id="n1", tool_type="logic/merge"),
        ],
        edges=[EdgeDef(id="e1", source="t1", target="n1")],
    )


class TestAgentLifecycle:
    @pytest.fixture
    def runtime(self):
        rt = AgentRuntime(registry=_make_registry())
        rt.register_graph(_make_graph())
        return rt

    @pytest.mark.asyncio
    async def test_deploy_agent(self, runtime):
        agent = await runtime.deploy_agent("test-agent", "g1")
        assert agent["name"] == "test-agent"
        assert agent["status"] == "disabled"
        assert agent["graph_id"] == "g1"

    @pytest.mark.asyncio
    async def test_deploy_invalid_graph_raises(self, runtime):
        with pytest.raises(ValueError, match="not found"):
            await runtime.deploy_agent("bad", "nonexistent")

    @pytest.mark.asyncio
    async def test_list_agents(self, runtime):
        await runtime.deploy_agent("a1", "g1")
        await runtime.deploy_agent("a2", "g1")
        assert len(runtime.list_agents()) == 2

    @pytest.mark.asyncio
    async def test_enable_disable(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        agent_id = agent["id"]

        await runtime.enable_agent(agent_id)
        assert runtime.get_agent(agent_id)["status"] == "enabled"

        await runtime.disable_agent(agent_id)
        assert runtime.get_agent(agent_id)["status"] == "disabled"

    @pytest.mark.asyncio
    async def test_destroy_agent(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        await runtime.destroy_agent(agent["id"])
        assert runtime.get_agent(agent["id"]) is None


class TestExecution:
    @pytest.fixture
    def runtime(self):
        rt = AgentRuntime(registry=_make_registry())
        rt.register_graph(_make_graph())
        return rt

    @pytest.mark.asyncio
    async def test_execute_agent(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        session = await runtime.execute_agent(agent["id"])
        assert session["status"] == "completed"
        assert session["agent_name"] == "test"
        assert "trace" in session
        assert "transcript" in session
        assert "duration_ms" in session

    @pytest.mark.asyncio
    async def test_execute_with_trigger_data(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        session = await runtime.execute_agent(
            agent["id"],
            trigger_data={"event": "file_uploaded", "file": "doc.pdf"},
        )
        assert session["status"] == "completed"

    @pytest.mark.asyncio
    async def test_transcript_generated(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        session = await runtime.execute_agent(agent["id"])
        transcript = session["transcript"]
        assert len(transcript) > 0
        # Should have started and completed events
        types = [e["type"] for e in transcript]
        assert "started" in types
        assert "completed" in types

    @pytest.mark.asyncio
    async def test_sessions_tracked(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        await runtime.execute_agent(agent["id"])
        await runtime.execute_agent(agent["id"])
        sessions = runtime.list_sessions()
        assert len(sessions) == 2

    @pytest.mark.asyncio
    async def test_sessions_filter_by_agent(self, runtime):
        a1 = await runtime.deploy_agent("a1", "g1")
        a2 = await runtime.deploy_agent("a2", "g1")
        await runtime.execute_agent(a1["id"])
        await runtime.execute_agent(a2["id"])
        await runtime.execute_agent(a1["id"])
        assert len(runtime.list_sessions(agent_id=a1["id"])) == 2
        assert len(runtime.list_sessions(agent_id=a2["id"])) == 1

    @pytest.mark.asyncio
    async def test_get_session(self, runtime):
        agent = await runtime.deploy_agent("test", "g1")
        session = await runtime.execute_agent(agent["id"])
        found = runtime.get_session(session["id"])
        assert found is not None
        assert found["id"] == session["id"]

    @pytest.mark.asyncio
    async def test_get_session_not_found(self, runtime):
        assert runtime.get_session("nonexistent") is None


class TestWebhookTrigger:
    @pytest.mark.asyncio
    async def test_webhook_routing(self):
        runtime = AgentRuntime(registry=_make_registry())
        runtime.register_graph(_make_graph())
        agent = await runtime.deploy_agent(
            "webhook-agent", "g1",
            triggers=[{"type": "webhook", "path": "/webhooks/test"}],
        )
        await runtime.enable_agent(agent["id"])

        assert "/webhooks/test" in runtime.get_webhook_routes()

        result = await runtime.handle_webhook(
            "/webhooks/test",
            body={"event": "ping"},
        )
        assert result is not None
        assert result["status"] == "completed"

    @pytest.mark.asyncio
    async def test_webhook_unknown_path_returns_none(self):
        runtime = AgentRuntime(registry=_make_registry())
        result = await runtime.handle_webhook("/webhooks/unknown", body={})
        assert result is None

    @pytest.mark.asyncio
    async def test_webhook_disabled_agent_ignored(self):
        runtime = AgentRuntime(registry=_make_registry())
        runtime.register_graph(_make_graph())
        agent = await runtime.deploy_agent(
            "agent", "g1",
            triggers=[{"type": "webhook", "path": "/webhooks/test"}],
        )
        await runtime.enable_agent(agent["id"])
        await runtime.disable_agent(agent["id"])

        result = await runtime.handle_webhook("/webhooks/test", body={})
        assert result is None


class TestScheduleTrigger:
    @pytest.mark.asyncio
    async def test_schedule_fires_automatically(self):
        runtime = AgentRuntime(registry=_make_registry())
        runtime.register_graph(_make_graph())
        agent = await runtime.deploy_agent(
            "scheduled", "g1",
            triggers=[{"type": "schedule", "interval_seconds": 0.2}],
        )

        await runtime.start()
        await runtime.enable_agent(agent["id"])

        # Wait for 2-3 executions
        await asyncio.sleep(0.7)
        await runtime.stop()

        sessions = runtime.list_sessions(agent_id=agent["id"])
        assert len(sessions) >= 2, f"Expected >=2 sessions, got {len(sessions)}"

    @pytest.mark.asyncio
    async def test_schedule_stops_on_disable(self):
        runtime = AgentRuntime(registry=_make_registry())
        runtime.register_graph(_make_graph())
        agent = await runtime.deploy_agent(
            "scheduled", "g1",
            triggers=[{"type": "schedule", "interval_seconds": 0.2}],
        )

        await runtime.start()
        await runtime.enable_agent(agent["id"])
        await asyncio.sleep(0.5)

        count_before = len(runtime.list_sessions(agent_id=agent["id"]))
        await runtime.disable_agent(agent["id"])
        await asyncio.sleep(0.5)
        count_after = len(runtime.list_sessions(agent_id=agent["id"]))

        await runtime.stop()

        # Should not have grown (or grown by at most 1 due to timing)
        assert count_after - count_before <= 1


class TestRuntimeStatus:
    @pytest.mark.asyncio
    async def test_status(self):
        runtime = AgentRuntime(registry=_make_registry())
        runtime.register_graph(_make_graph())
        await runtime.deploy_agent("a1", "g1")

        status = runtime.status()
        assert status["agents_total"] == 1
        assert status["agents_enabled"] == 0
