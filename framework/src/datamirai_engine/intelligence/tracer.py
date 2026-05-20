"""Execution Tracer — passive capture of rich execution data per node.

Fire-and-forget: never blocks the graph execution pipeline.
Captures: inputs, outputs, data_map, duration, tokens, status, errors.
"""

from __future__ import annotations

import json
import time
from typing import Any, Callable, Awaitable


class ExecutionTracer:
    """Captures rich execution traces after each node execution.

    Registered as post_block_exec hook in GraphRunner.
    Writes to a callback (typically DB insert) via fire-and-forget.
    """

    MAX_SNAPSHOT_CHARS = 2000

    def __init__(
        self,
        persist_fn: Callable[..., Awaitable[None]] | Callable[..., None] | None = None,
    ):
        """
        Args:
            persist_fn: async or sync function to persist trace data.
                       Receives a dict with all trace fields.
                       If None, traces are stored in-memory (for testing).
        """
        self._persist_fn = persist_fn
        self._traces: list[dict[str, Any]] = []  # in-memory fallback

    def capture(
        self,
        *,
        session_id: str,
        agent_id: str,
        node_id: str,
        tool_type: str,
        inputs: dict[str, Any] | None = None,
        output: dict[str, Any] | None = None,
        data_map: dict[str, Any] | None = None,
        duration_ms: int = 0,
        tokens_input: int = 0,
        tokens_output: int = 0,
        status: str = "success",  # success / error / skip
        error_type: str | None = None,
        error_message: str | None = None,
        retry_count: int = 0,
    ) -> dict[str, Any]:
        """Capture a trace entry. Returns the trace dict.

        Fire-and-forget: if persist_fn is set, calls it but doesn't await/block.
        """
        trace = {
            "session_id": session_id,
            "agent_id": agent_id,
            "node_id": node_id,
            "tool_type": tool_type,
            "inputs_snapshot": self._truncate(inputs),
            "output_snapshot": self._truncate(output),
            "data_map_used": json.dumps(data_map) if data_map else None,
            "duration_ms": duration_ms,
            "tokens_input": tokens_input,
            "tokens_output": tokens_output,
            "status": status,
            "error_type": error_type,
            "error_message": error_message[:500] if error_message else None,
            "retry_count": retry_count,
            "created_at": time.time(),
        }

        self._traces.append(trace)

        if self._persist_fn:
            import asyncio

            try:
                result = self._persist_fn(trace)
                if asyncio.iscoroutine(result):
                    # Fire-and-forget async
                    try:
                        loop = asyncio.get_running_loop()
                        loop.create_task(result)
                    except RuntimeError:
                        pass  # No event loop, skip persistence
            except Exception:
                pass  # REGLA-40: never block execution

        return trace

    def get_traces(self) -> list[dict[str, Any]]:
        """Get in-memory traces (for testing)."""
        return list(self._traces)

    def clear(self) -> None:
        """Clear in-memory traces."""
        self._traces.clear()

    @classmethod
    def _truncate(cls, data: Any, max_chars: int | None = None) -> str | None:
        """Truncate data to max chars. Returns JSON string or None."""
        if data is None:
            return None
        max_chars = max_chars or cls.MAX_SNAPSHOT_CHARS
        try:
            serialized = json.dumps(data, ensure_ascii=False, default=str)
        except (TypeError, ValueError):
            serialized = str(data)

        if len(serialized) <= max_chars:
            return serialized

        # Truncate and mark
        truncated = serialized[: max_chars - 30]
        return truncated + '..."__truncated": true}'
