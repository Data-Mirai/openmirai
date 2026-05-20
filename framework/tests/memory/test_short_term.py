"""Tests for ShortTermMemory — session trace and context during execution."""

from __future__ import annotations

import pytest

from datamirai_engine.memory.short_term import ShortTermMemory


class TestShortTermMemory:
    @pytest.fixture
    def mem(self):
        return ShortTermMemory(session_id="sess-001", agent_id="agent-001")

    def test_create(self, mem):
        assert mem.session_id == "sess-001"
        assert mem.agent_id == "agent-001"

    def test_log_entry(self, mem):
        mem.log("Decided to use Claude for long prompt")
        assert len(mem.get_logs()) == 1
        assert "Claude" in mem.get_logs()[0]["message"]

    def test_log_multiple(self, mem):
        mem.log("Step 1")
        mem.log("Step 2")
        mem.log("Step 3")
        assert len(mem.get_logs()) == 3

    def test_log_has_timestamp(self, mem):
        mem.log("test")
        entry = mem.get_logs()[0]
        assert "timestamp" in entry

    def test_record_decision(self, mem):
        mem.record_decision(
            node_id="n1",
            decision="Took path A because output was success",
            alternatives=["path B", "path C"],
        )
        decisions = mem.get_decisions()
        assert len(decisions) == 1
        assert decisions[0]["node_id"] == "n1"
        assert len(decisions[0]["alternatives"]) == 2

    def test_record_block_metrics(self, mem):
        mem.record_metrics(
            node_id="n1",
            tool_type="ai/llm_call",
            duration_ms=150.5,
            tokens_used=42,
        )
        metrics = mem.get_metrics()
        assert len(metrics) == 1
        assert metrics[0]["duration_ms"] == 150.5
        assert metrics[0]["tokens_used"] == 42

    def test_get_trace_summary(self, mem):
        mem.log("Started execution")
        mem.record_decision("n1", "Chose path A")
        mem.record_metrics("n1", "ai/llm_call", duration_ms=100, tokens_used=10)
        mem.log("Finished")

        trace = mem.get_trace()
        assert "logs" in trace
        assert "decisions" in trace
        assert "metrics" in trace
        assert trace["session_id"] == "sess-001"
        assert trace["agent_id"] == "agent-001"

    def test_to_persistable_dict(self, mem):
        mem.log("test log")
        mem.record_decision("n1", "test decision")
        mem.record_metrics("n1", "a/b", duration_ms=50, tokens_used=5)

        data = mem.to_dict()
        assert data["session_id"] == "sess-001"
        assert len(data["logs"]) == 1
        assert len(data["decisions"]) == 1
        assert len(data["metrics"]) == 1

    def test_empty_trace(self, mem):
        trace = mem.get_trace()
        assert trace["logs"] == []
        assert trace["decisions"] == []
        assert trace["metrics"] == []
