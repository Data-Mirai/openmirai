"""Tests for Reflector — LLM-powered trace analysis."""

import json

import pytest

from datamirai_engine.intelligence.reflector import Reflector, MAX_TRACES_PER_BATCH


# --- Helpers ---


def _make_trace(node_id: str = "node-1", status: str = "success", duration_ms: int = 100, **kwargs):
    """Create a minimal trace dict."""
    return {
        "session_id": "sess-1",
        "agent_id": "agent-1",
        "node_id": node_id,
        "tool_type": "ai/llm_call",
        "status": status,
        "duration_ms": duration_ms,
        "tokens_input": 10,
        "tokens_output": 20,
        "error_type": None,
        "error_message": None,
        "retry_count": 0,
        **kwargs,
    }


def _make_llm_fn(response: str):
    """Create a mock async LLM call function that returns a fixed response."""
    async def llm_call(prompt: str) -> str:
        return response
    return llm_call


# --- Tests ---


def test_build_prompt_includes_traces():
    """Prompt contains serialized trace data and agent name."""
    reflector = Reflector()
    traces = [_make_trace("node-A"), _make_trace("node-B", status="error")]
    prompt = reflector._build_prompt(traces, "TestBot")

    assert '"TestBot"' in prompt
    assert "node-A" in prompt
    assert "node-B" in prompt
    assert '"error"' in prompt
    assert "success_pattern" in prompt
    assert "failure_pattern" in prompt
    assert "optimization" in prompt
    assert "anomaly" in prompt


def test_build_batch_limits_to_50():
    """REGLA-45: batch is capped at MAX_TRACES_PER_BATCH."""
    reflector = Reflector()
    traces = [_make_trace(f"node-{i}") for i in range(80)]
    batch = reflector._build_batch(traces)

    assert len(batch) == MAX_TRACES_PER_BATCH
    # Should keep the most recent (last 50)
    assert batch[0]["node_id"] == "node-30"
    assert batch[-1]["node_id"] == "node-79"


def test_build_batch_small_list_unchanged():
    """Batch with fewer traces than limit returns all."""
    reflector = Reflector()
    traces = [_make_trace(f"node-{i}") for i in range(5)]
    batch = reflector._build_batch(traces)

    assert len(batch) == 5


def test_parse_response_valid_json():
    """Valid JSON array is parsed correctly."""
    reflector = Reflector()
    response = json.dumps([
        {
            "type": "success_pattern",
            "node_id": "node-1",
            "insight": "Node consistently completes in <100ms",
            "confidence": 0.85,
        },
        {
            "type": "failure_pattern",
            "node_id": "node-2",
            "insight": "Timeout errors on LLM calls",
            "confidence": 0.9,
        },
    ])

    result = reflector._parse_response(response)

    assert len(result) == 2
    assert result[0]["type"] == "success_pattern"
    assert result[0]["node_id"] == "node-1"
    assert result[0]["confidence"] == 0.85
    assert result[1]["type"] == "failure_pattern"


def test_parse_response_invalid_json_fallback():
    """Invalid JSON falls back to generic anomaly reflection."""
    reflector = Reflector()
    result = reflector._parse_response("This is not JSON at all, just text.")

    assert len(result) == 1
    assert result[0]["type"] == "anomaly"
    assert result[0]["confidence"] == 0.1
    assert "could not be parsed" in result[0]["insight"]


def test_parse_response_strips_markdown_fences():
    """JSON wrapped in markdown code fences is parsed correctly."""
    reflector = Reflector()
    response = '```json\n[{"type": "optimization", "node_id": "n1", "insight": "High token usage", "confidence": 0.7}]\n```'

    result = reflector._parse_response(response)

    assert len(result) == 1
    assert result[0]["type"] == "optimization"


def test_parse_response_filters_invalid_types():
    """Reflections with unknown types are filtered out."""
    reflector = Reflector()
    response = json.dumps([
        {"type": "success_pattern", "node_id": "n1", "insight": "Good", "confidence": 0.8},
        {"type": "invented_type", "node_id": "n2", "insight": "Bad", "confidence": 0.5},
    ])

    result = reflector._parse_response(response)

    assert len(result) == 1
    assert result[0]["type"] == "success_pattern"


def test_parse_response_clamps_confidence():
    """Confidence values are clamped to [0.0, 1.0]."""
    reflector = Reflector()
    response = json.dumps([
        {"type": "anomaly", "node_id": None, "insight": "Over", "confidence": 1.5},
        {"type": "anomaly", "node_id": None, "insight": "Under", "confidence": -0.3},
    ])

    result = reflector._parse_response(response)

    assert result[0]["confidence"] == 1.0
    assert result[1]["confidence"] == 0.0


def test_parse_response_empty_array():
    """Empty JSON array returns empty list."""
    reflector = Reflector()
    result = reflector._parse_response("[]")

    assert result == []


@pytest.mark.asyncio
async def test_reflect_returns_reflections():
    """reflect() orchestrates prompt → LLM → parse correctly."""
    reflector = Reflector()
    traces = [_make_trace("node-1"), _make_trace("node-2", status="error")]

    llm_response = json.dumps([
        {
            "type": "failure_pattern",
            "node_id": "node-2",
            "insight": "Repeated errors on node-2",
            "confidence": 0.9,
        },
    ])

    captured_prompts = []

    async def mock_llm(prompt: str) -> str:
        captured_prompts.append(prompt)
        return llm_response

    result = await reflector.reflect(
        agent_id="agent-1",
        agent_name="TestBot",
        traces=traces,
        llm_call_fn=mock_llm,
    )

    assert len(result) == 1
    assert result[0]["type"] == "failure_pattern"
    assert result[0]["node_id"] == "node-2"
    # Verify prompt was built correctly
    assert len(captured_prompts) == 1
    assert "TestBot" in captured_prompts[0]


@pytest.mark.asyncio
async def test_reflect_empty_traces_returns_empty():
    """No traces → no reflections, LLM is not called."""
    reflector = Reflector()
    call_count = 0

    async def mock_llm(prompt: str) -> str:
        nonlocal call_count
        call_count += 1
        return "[]"

    result = await reflector.reflect(
        agent_id="agent-1",
        agent_name="TestBot",
        traces=[],
        llm_call_fn=mock_llm,
    )

    assert result == []
    assert call_count == 0
