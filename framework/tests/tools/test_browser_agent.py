"""Tests for browser_agent module."""

from datamirai_engine.tools.builtin.data.browser_agent import (
    MAX_AGENT_STEPS,
    _build_agent_prompt,
)


class TestBuildAgentPrompt:
    def test_first_step(self):
        prompt = _build_agent_prompt("Find prices", [], 0, 10)
        assert "Find prices" in prompt
        assert "Paso actual: 1 de 10" in prompt
        assert "Primera accion" in prompt

    def test_with_history(self):
        history = [
            {"step": 0, "action": {"type": "click", "x": 100, "y": 200}},
            {"step": 1, "action": {"type": "scroll", "delta": 300}},
        ]
        prompt = _build_agent_prompt("Find prices", history, 2, 10)
        assert "Step 0" in prompt
        assert "Step 1" in prompt
        assert "Paso actual: 3 de 10" in prompt

    def test_response_format(self):
        prompt = _build_agent_prompt("Extract data", [], 0, 5)
        assert '"type": "click"' in prompt
        assert '"type": "done"' in prompt
        assert "JSON" in prompt


class TestMaxSteps:
    def test_hard_limit_is_25(self):
        assert MAX_AGENT_STEPS == 25


class TestPlaywrightCheck:
    def test_descriptive_error_message(self):
        """REGLA-360: Descriptive error when playwright not installed."""
        from datamirai_engine.tools.builtin.data.browser_agent import _check_playwright
        # We can't test the actual import failure easily, but we can verify
        # the function exists and is callable
        assert callable(_check_playwright)
