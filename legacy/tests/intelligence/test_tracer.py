"""Tests for ExecutionTracer — passive capture of rich execution data."""

import json

import pytest

from datamirai_engine.intelligence.tracer import ExecutionTracer


# --- Fixtures ---


def _base_kwargs():
    """Minimum required kwargs for capture()."""
    return {
        "session_id": "sess-1",
        "agent_id": "agent-1",
        "node_id": "node-1",
        "tool_type": "ai/llm_call",
    }


# --- Tests ---


def test_capture_basic():
    """capture() returns dict with all expected fields."""
    tracer = ExecutionTracer()
    trace = tracer.capture(
        **_base_kwargs(),
        inputs={"prompt": "hello"},
        output={"text": "world"},
        duration_ms=150,
        tokens_input=10,
        tokens_output=20,
    )

    assert trace["session_id"] == "sess-1"
    assert trace["agent_id"] == "agent-1"
    assert trace["node_id"] == "node-1"
    assert trace["tool_type"] == "ai/llm_call"
    assert trace["duration_ms"] == 150
    assert trace["tokens_input"] == 10
    assert trace["tokens_output"] == 20
    assert trace["status"] == "success"
    assert trace["retry_count"] == 0
    assert trace["error_type"] is None
    assert trace["error_message"] is None
    assert trace["created_at"] > 0
    # Snapshots are JSON strings
    assert json.loads(trace["inputs_snapshot"]) == {"prompt": "hello"}
    assert json.loads(trace["output_snapshot"]) == {"text": "world"}


def test_capture_truncates_large_inputs():
    """Inputs larger than MAX_SNAPSHOT_CHARS get truncated."""
    tracer = ExecutionTracer()
    large_input = {"data": "x" * 3000}
    trace = tracer.capture(**_base_kwargs(), inputs=large_input)

    assert len(trace["inputs_snapshot"]) <= ExecutionTracer.MAX_SNAPSHOT_CHARS
    assert "__truncated" in trace["inputs_snapshot"]


def test_capture_truncates_large_outputs():
    """Outputs larger than MAX_SNAPSHOT_CHARS get truncated."""
    tracer = ExecutionTracer()
    large_output = {"result": "y" * 3000}
    trace = tracer.capture(**_base_kwargs(), output=large_output)

    assert len(trace["output_snapshot"]) <= ExecutionTracer.MAX_SNAPSHOT_CHARS
    assert "__truncated" in trace["output_snapshot"]


def test_truncate_marks_truncated():
    """Truncated data contains the __truncated marker at the end."""
    tracer = ExecutionTracer()
    large_data = {"key": "a" * 5000}
    trace = tracer.capture(**_base_kwargs(), inputs=large_data)

    snapshot = trace["inputs_snapshot"]
    assert snapshot.endswith('..."__truncated": true}')


def test_capture_none_inputs():
    """None inputs produce None snapshot."""
    tracer = ExecutionTracer()
    trace = tracer.capture(**_base_kwargs(), inputs=None, output=None)

    assert trace["inputs_snapshot"] is None
    assert trace["output_snapshot"] is None


def test_capture_stores_in_memory():
    """Traces are accessible via get_traces()."""
    tracer = ExecutionTracer()
    tracer.capture(**_base_kwargs())
    kw2 = _base_kwargs()
    kw2["node_id"] = "node-2"
    tracer.capture(**kw2)

    traces = tracer.get_traces()
    assert len(traces) == 2
    assert traces[0]["node_id"] == "node-1"
    assert traces[1]["node_id"] == "node-2"


def test_capture_with_persist_fn():
    """persist_fn gets called with trace data."""
    captured = []

    def persist(trace_data):
        captured.append(trace_data)

    tracer = ExecutionTracer(persist_fn=persist)
    tracer.capture(**_base_kwargs())

    assert len(captured) == 1
    assert captured[0]["session_id"] == "sess-1"


def test_capture_persist_fn_error_ignored():
    """persist_fn throws, capture still works (REGLA-40)."""

    def broken_persist(trace_data):
        raise RuntimeError("DB is down")

    tracer = ExecutionTracer(persist_fn=broken_persist)
    # Should NOT raise
    trace = tracer.capture(**_base_kwargs())

    assert trace["status"] == "success"
    assert len(tracer.get_traces()) == 1


def test_capture_error_trace():
    """status=error with error_type and error_message."""
    tracer = ExecutionTracer()
    trace = tracer.capture(
        **_base_kwargs(),
        status="error",
        error_type="TimeoutError",
        error_message="LLM call timed out after 30s",
    )

    assert trace["status"] == "error"
    assert trace["error_type"] == "TimeoutError"
    assert trace["error_message"] == "LLM call timed out after 30s"


def test_clear_traces():
    """clear() empties in-memory traces."""
    tracer = ExecutionTracer()
    tracer.capture(**_base_kwargs())
    tracer.capture(**_base_kwargs())
    assert len(tracer.get_traces()) == 2

    tracer.clear()
    assert len(tracer.get_traces()) == 0


def test_error_message_truncated_to_500():
    """Long error messages get cut to 500 chars."""
    tracer = ExecutionTracer()
    long_msg = "E" * 1000
    trace = tracer.capture(
        **_base_kwargs(),
        status="error",
        error_type="ValueError",
        error_message=long_msg,
    )

    assert len(trace["error_message"]) == 500


def test_data_map_serialized():
    """data_map is stored as JSON string."""
    tracer = ExecutionTracer()
    dm = {"prompt": "scrape.content", "context": "prev.summary"}
    trace = tracer.capture(**_base_kwargs(), data_map=dm)

    assert trace["data_map_used"] is not None
    parsed = json.loads(trace["data_map_used"])
    assert parsed == dm


def test_data_map_none():
    """data_map=None produces None."""
    tracer = ExecutionTracer()
    trace = tracer.capture(**_base_kwargs(), data_map=None)

    assert trace["data_map_used"] is None


def test_small_data_not_truncated():
    """Data within limit is preserved exactly."""
    tracer = ExecutionTracer()
    small = {"key": "value"}
    trace = tracer.capture(**_base_kwargs(), inputs=small)

    assert json.loads(trace["inputs_snapshot"]) == small
    assert "__truncated" not in trace["inputs_snapshot"]
