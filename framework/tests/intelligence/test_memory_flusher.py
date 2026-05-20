"""Tests for MemoryFlusher — auto-persist critical data before context compression."""

import pytest

from datamirai_engine.intelligence.memory_flusher import MemoryFlusher


# --- should_flush tests ---


def test_should_flush_above_threshold():
    """Flush triggers when tokens exceed threshold."""
    mf = MemoryFlusher(threshold=0.75, context_window=100000)

    # 75000 tokens = exactly 75% → should trigger (>=)
    assert mf.should_flush(75000) is True
    # Above threshold
    assert mf.should_flush(80000) is True
    assert mf.should_flush(100000) is True


def test_should_not_flush_below_threshold():
    """No flush when tokens are below threshold."""
    mf = MemoryFlusher(threshold=0.75, context_window=100000)

    assert mf.should_flush(0) is False
    assert mf.should_flush(50000) is False
    assert mf.should_flush(74999) is False


def test_max_one_flush_per_session():
    """REGLA-58: After one flush, should_flush always returns False."""
    mf = MemoryFlusher(threshold=0.75, context_window=100000)

    # First check: should flush
    assert mf.should_flush(80000) is True

    # Simulate flush
    import asyncio
    asyncio.get_event_loop().run_until_complete(
        mf.flush(
            session_summary="test summary",
            agent_id="agent-1",
            session_id="session-1",
        )
    )

    # After flush: should NOT trigger again even with more tokens
    assert mf.should_flush(100000) is False
    assert mf.should_flush(999999) is False


@pytest.mark.asyncio
async def test_flush_calls_save_fn():
    """save_fn is called with correct args during flush."""
    mf = MemoryFlusher()

    calls = []

    async def mock_save(**kwargs):
        calls.append(kwargs)

    result = await mf.flush(
        session_summary="Important findings",
        agent_id="agent-42",
        session_id="session-7",
        save_fn=mock_save,
    )

    assert result is True
    assert len(calls) == 1
    assert calls[0]["agent_id"] == "agent-42"
    assert calls[0]["session_id"] == "session-7"
    assert "[Auto-flush]" in calls[0]["summary"]
    assert "Important findings" in calls[0]["summary"]
    assert calls[0]["tags"] == ["auto-flush"]


@pytest.mark.asyncio
async def test_flush_without_save_fn_still_marks_flushed():
    """Flush without save_fn still marks as flushed (no crash)."""
    mf = MemoryFlusher()

    assert mf.has_flushed is False

    result = await mf.flush(
        session_summary="summary",
        agent_id="a",
        session_id="s",
        save_fn=None,
    )

    assert result is True
    assert mf.has_flushed is True


@pytest.mark.asyncio
async def test_flush_second_time_returns_false():
    """Second flush attempt returns False (REGLA-58)."""
    mf = MemoryFlusher()

    first = await mf.flush(
        session_summary="first",
        agent_id="a",
        session_id="s",
    )
    assert first is True

    second = await mf.flush(
        session_summary="second",
        agent_id="a",
        session_id="s",
    )
    assert second is False


def test_has_flushed_property():
    """has_flushed reflects flush state correctly."""
    mf = MemoryFlusher()

    # Initially not flushed
    assert mf.has_flushed is False

    # After flush
    import asyncio
    asyncio.get_event_loop().run_until_complete(
        mf.flush(
            session_summary="test",
            agent_id="a",
            session_id="s",
        )
    )

    assert mf.has_flushed is True


def test_custom_threshold():
    """Custom threshold values work correctly."""
    mf = MemoryFlusher(threshold=0.5, context_window=10000)

    # 50% of 10000 = 5000
    assert mf.should_flush(4999) is False
    assert mf.should_flush(5000) is True
    assert mf.should_flush(5001) is True


def test_custom_context_window():
    """Custom context_window is respected."""
    mf = MemoryFlusher(threshold=0.75, context_window=200000)

    # 75% of 200000 = 150000
    assert mf.should_flush(149999) is False
    assert mf.should_flush(150000) is True
