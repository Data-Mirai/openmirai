"""Tests for builtin logic blocks — Condition, Switch, Loop, Merge, Wait."""

from __future__ import annotations

import time

import pytest

from datamirai_engine.tools.builtin.logic.condition import ConditionTool
from datamirai_engine.tools.builtin.logic.loop import LoopTool
from datamirai_engine.tools.builtin.logic.merge import MergeTool
from datamirai_engine.tools.builtin.logic.switch import SwitchTool
from datamirai_engine.tools.builtin.logic.wait import WaitTool


class TestConditionTool:
    @pytest.mark.asyncio
    async def test_eq_true(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": "hello"},
            {"operator": "eq", "compare_to": "hello"},
            None,
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_eq_false(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": "hello"},
            {"operator": "eq", "compare_to": "world"},
            None,
        )
        assert result["result"] is False

    @pytest.mark.asyncio
    async def test_gt(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": 10}, {"operator": "gt", "compare_to": 5}, None
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_lt(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": 3}, {"operator": "lt", "compare_to": 5}, None
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_contains(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": "hello world"},
            {"operator": "contains", "compare_to": "world"},
            None,
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_in_list(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": "b"},
            {"operator": "in", "compare_to": ["a", "b", "c"]},
            None,
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_neq(self):
        tool = ConditionTool()
        result = await tool.run(
            {"value": 1}, {"operator": "neq", "compare_to": 2}, None
        )
        assert result["result"] is True

    @pytest.mark.asyncio
    async def test_missing_value_raises(self):
        tool = ConditionTool()
        with pytest.raises(ValueError, match=r"Missing required input"):
            await tool.run({}, {"operator": "eq", "compare_to": "x"}, None)


class TestSwitchTool:
    @pytest.mark.asyncio
    async def test_matches_case(self):
        tool = SwitchTool()
        result = await tool.run(
            {"value": "b"},
            {"cases": ["a", "b", "c"]},
            None,
        )
        assert result["matched_case"] == "b"
        assert result["case_index"] == 1

    @pytest.mark.asyncio
    async def test_no_match_returns_default(self):
        tool = SwitchTool()
        result = await tool.run(
            {"value": "z"},
            {"cases": ["a", "b", "c"], "default_case": "fallback"},
            None,
        )
        assert result["matched_case"] == "fallback"
        assert result["case_index"] == -1

    @pytest.mark.asyncio
    async def test_no_match_no_default(self):
        tool = SwitchTool()
        result = await tool.run(
            {"value": "z"},
            {"cases": ["a", "b"]},
            None,
        )
        assert result["matched_case"] is None
        assert result["case_index"] == -1


class TestLoopTool:
    @pytest.mark.asyncio
    async def test_increments_counter(self):
        tool = LoopTool()
        result = await tool.run(
            {"current_index": 0, "items": [10, 20, 30]}, {}, None
        )
        assert result["current_index"] == 1
        assert result["current_item"] == 20
        assert result["done"] is False

    @pytest.mark.asyncio
    async def test_completes_at_end(self):
        tool = LoopTool()
        result = await tool.run(
            {"current_index": 2, "items": [10, 20, 30]}, {}, None
        )
        assert result["done"] is True
        assert result["current_index"] == 3

    @pytest.mark.asyncio
    async def test_starts_at_zero(self):
        tool = LoopTool()
        result = await tool.run(
            {"items": ["a", "b"]}, {}, None
        )
        assert result["current_index"] == 1
        assert result["current_item"] == "b"
        assert result["done"] is False

    @pytest.mark.asyncio
    async def test_single_item_done_immediately(self):
        tool = LoopTool()
        result = await tool.run(
            {"current_index": 0, "items": ["only"]}, {}, None
        )
        assert result["done"] is True


class TestMergeTool:
    @pytest.mark.asyncio
    async def test_passes_through(self):
        tool = MergeTool()
        result = await tool.run({"data": {"key": "value"}}, {}, None)
        assert result["data"] == {"key": "value"}

    @pytest.mark.asyncio
    async def test_empty_input(self):
        tool = MergeTool()
        result = await tool.run({}, {}, None)
        assert result["data"] is None


class TestWaitTool:
    @pytest.mark.asyncio
    async def test_waits_configured_time(self):
        tool = WaitTool()
        start = time.monotonic()
        result = await tool.run({}, {"delay_seconds": 0.05}, None)
        elapsed = time.monotonic() - start
        assert elapsed >= 0.04  # small margin
        assert result["waited_seconds"] == 0.05

    @pytest.mark.asyncio
    async def test_default_delay(self):
        tool = WaitTool()
        result = await tool.run({}, {}, None)
        assert result["waited_seconds"] == 0  # default = 0
