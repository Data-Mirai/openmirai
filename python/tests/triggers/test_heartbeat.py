"""Tests for HeartbeatTriggerTool — condition evaluation, quiet hours, output format."""

from __future__ import annotations

from datetime import datetime, timezone
from unittest.mock import patch

import pytest

from datamirai_engine.tools.builtin.trigger.heartbeat import (
    ConditionEvaluator,
    HeartbeatTriggerTool,
    _is_in_quiet_hours,
    _parse_interval,
    safe_eval_expression,
)


@pytest.fixture
def tool():
    return HeartbeatTriggerTool()


class TestSpecMetadata:
    """Verify ToolSpec is correctly defined."""

    def test_spec_metadata(self, tool):
        spec = tool.spec
        assert spec.tool_type == "trigger/heartbeat"
        assert spec.version == "1.0.0"
        assert spec.category == "trigger"
        assert spec.display_name == "Heartbeat"
        assert spec.icon == "heartbeat"
        assert len(spec.intents) > 0

    def test_spec_has_expected_outputs(self, tool):
        output_names = [o.name for o in tool.spec.outputs]
        assert "triggered" in output_names
        assert "condition_result" in output_names
        assert "evaluated_at" in output_names

    def test_spec_has_expected_config(self, tool):
        config_names = [c.name for c in tool.spec.config]
        assert "interval" in config_names
        assert "condition_type" in config_names
        assert "condition_config" in config_names
        assert "quiet_hours_start" in config_names
        assert "quiet_hours_end" in config_names

    def test_spec_inputs_optional(self, tool):
        for inp in tool.spec.inputs:
            if inp.name == "condition_result":
                assert inp.required is False


class TestAlwaysTrueCondition:
    @pytest.mark.asyncio
    async def test_always_true_condition(self, tool):
        result = await tool.run(
            {},
            {"condition_type": "always_true"},
            None,
        )
        assert result["triggered"] is True

    @pytest.mark.asyncio
    async def test_always_true_ignores_config_str(self, tool):
        result = await tool.run(
            {},
            {"condition_type": "always_true", "condition_config": "anything here"},
            None,
        )
        assert result["triggered"] is True


class TestAlwaysFalseCondition:
    @pytest.mark.asyncio
    async def test_always_false_condition(self, tool):
        result = await tool.run(
            {},
            {"condition_type": "always_false"},
            None,
        )
        assert result["triggered"] is False


class TestCustomExpression:
    @pytest.mark.asyncio
    async def test_custom_expression_true(self, tool):
        result = await tool.run(
            {"condition_result": 42},
            {"condition_type": "custom_expression", "condition_config": "condition_result > 10"},
            None,
        )
        assert result["triggered"] is True

    @pytest.mark.asyncio
    async def test_custom_expression_false(self, tool):
        result = await tool.run(
            {"condition_result": 5},
            {"condition_type": "custom_expression", "condition_config": "condition_result > 10"},
            None,
        )
        assert result["triggered"] is False

    @pytest.mark.asyncio
    async def test_custom_expression_equality(self, tool):
        result = await tool.run(
            {"condition_result": "ready"},
            {"condition_type": "custom_expression", "condition_config": "condition_result == 'ready'"},
            None,
        )
        assert result["triggered"] is True

    @pytest.mark.asyncio
    async def test_empty_expression_is_true(self, tool):
        result = await tool.run(
            {},
            {"condition_type": "custom_expression", "condition_config": ""},
            None,
        )
        assert result["triggered"] is True

    @pytest.mark.asyncio
    async def test_custom_expression_boolean_logic(self, tool):
        result = await tool.run(
            {"condition_result": 15},
            {"condition_type": "custom_expression", "condition_config": "condition_result > 10 and condition_result < 20"},
            None,
        )
        assert result["triggered"] is True

    def test_safe_eval_blocks_import(self):
        with pytest.raises(ValueError, match="Forbidden keyword"):
            safe_eval_expression("import os")

    def test_safe_eval_blocks_exec(self):
        with pytest.raises(ValueError, match="Forbidden keyword"):
            safe_eval_expression("exec('print(1)')")

    def test_safe_eval_blocks_function_calls(self):
        with pytest.raises(ValueError, match="Function calls are not allowed"):
            safe_eval_expression("len('hello')")

    def test_safe_eval_blocks_dunder(self):
        with pytest.raises(ValueError, match="Forbidden keyword"):
            safe_eval_expression("__builtins__")


class TestQuietHours:
    @pytest.mark.asyncio
    async def test_quiet_hours_respected(self, tool):
        """During quiet hours, triggered should be False even with always_true."""
        # Mock datetime to 23:00 UTC
        mock_now = datetime(2026, 5, 9, 23, 0, 0, tzinfo=timezone.utc)
        with patch("datamirai_engine.tools.builtin.trigger.heartbeat.datetime") as mock_dt:
            mock_dt.now.return_value = mock_now
            mock_dt.side_effect = lambda *a, **kw: datetime(*a, **kw)

            result = await tool.run(
                {},
                {
                    "condition_type": "always_true",
                    "quiet_hours_start": "22:00",
                    "quiet_hours_end": "06:00",
                },
                None,
            )
            assert result["triggered"] is False

    @pytest.mark.asyncio
    async def test_outside_quiet_hours_triggers(self, tool):
        """Outside quiet hours, condition should evaluate normally."""
        mock_now = datetime(2026, 5, 9, 12, 0, 0, tzinfo=timezone.utc)
        with patch("datamirai_engine.tools.builtin.trigger.heartbeat.datetime") as mock_dt:
            mock_dt.now.return_value = mock_now
            mock_dt.side_effect = lambda *a, **kw: datetime(*a, **kw)

            result = await tool.run(
                {},
                {
                    "condition_type": "always_true",
                    "quiet_hours_start": "22:00",
                    "quiet_hours_end": "06:00",
                },
                None,
            )
            assert result["triggered"] is True

    def test_quiet_hours_same_day(self):
        now = datetime(2026, 5, 9, 14, 30, tzinfo=timezone.utc)
        assert _is_in_quiet_hours(now, "14:00", "15:00") is True
        assert _is_in_quiet_hours(now, "15:00", "16:00") is False

    def test_quiet_hours_crosses_midnight(self):
        late_night = datetime(2026, 5, 9, 23, 30, tzinfo=timezone.utc)
        early_morning = datetime(2026, 5, 10, 4, 0, tzinfo=timezone.utc)
        mid_day = datetime(2026, 5, 10, 12, 0, tzinfo=timezone.utc)
        assert _is_in_quiet_hours(late_night, "22:00", "06:00") is True
        assert _is_in_quiet_hours(early_morning, "22:00", "06:00") is True
        assert _is_in_quiet_hours(mid_day, "22:00", "06:00") is False

    def test_empty_quiet_hours_returns_false(self):
        now = datetime(2026, 5, 9, 12, 0, tzinfo=timezone.utc)
        assert _is_in_quiet_hours(now, "", "") is False
        assert _is_in_quiet_hours(now, "22:00", "") is False


class TestOutputFormat:
    @pytest.mark.asyncio
    async def test_output_format(self, tool):
        result = await tool.run(
            {"condition_result": {"key": "value"}},
            {"condition_type": "always_true"},
            None,
        )
        # All expected keys present
        assert "triggered" in result
        assert "condition_result" in result
        assert "evaluated_at" in result

        # Types are correct
        assert isinstance(result["triggered"], bool)
        assert isinstance(result["evaluated_at"], str)

        # condition_result passes through
        assert result["condition_result"] == {"key": "value"}

        # evaluated_at is a valid ISO timestamp
        parsed = datetime.fromisoformat(result["evaluated_at"])
        assert parsed.tzinfo is not None

    @pytest.mark.asyncio
    async def test_output_with_no_condition_result(self, tool):
        result = await tool.run(
            {},
            {"condition_type": "always_true"},
            None,
        )
        assert result["condition_result"] is None


class TestIntervalParsing:
    def test_parse_seconds(self):
        assert _parse_interval("30s") == 30

    def test_parse_minutes(self):
        assert _parse_interval("30m") == 1800

    def test_parse_hours(self):
        assert _parse_interval("1h") == 3600

    def test_parse_days(self):
        assert _parse_interval("2d") == 172800

    def test_invalid_format(self):
        with pytest.raises(ValueError, match="Invalid interval format"):
            _parse_interval("invalid")


class TestConditionEvaluator:
    def test_list_evaluators(self):
        evaluators = ConditionEvaluator.list_evaluators()
        assert "always_true" in evaluators
        assert "always_false" in evaluators
        assert "custom_expression" in evaluators

    def test_unknown_evaluator_raises(self):
        with pytest.raises(ValueError, match="Unknown condition type"):
            ConditionEvaluator.evaluate("nonexistent", "", {})
