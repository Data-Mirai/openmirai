"""Memory Flusher — auto-persist critical data before context compression."""

from __future__ import annotations

from typing import Any, Callable, Coroutine


class MemoryFlusher:
    """Monitors accumulated tokens and flushes to long-term memory.

    Trigger: tokens >= context_window * threshold (default 0.75)
    Max 1 flush per session (REGLA-58).
    """

    def __init__(
        self, threshold: float = 0.75, context_window: int = 128000
    ) -> None:
        self._threshold = threshold
        self._context_window = context_window
        self._flushed = False

    def should_flush(self, tokens_accumulated: int) -> bool:
        """Check if accumulated tokens exceed the flush threshold.

        Returns False if already flushed this session (REGLA-58: max 1 per session).
        """
        if self._flushed:
            return False  # REGLA-58: max 1 per session
        return tokens_accumulated >= self._context_window * self._threshold

    async def flush(
        self,
        *,
        session_summary: str,
        agent_id: str,
        session_id: str,
        save_fn: (
            Callable[..., Coroutine[Any, Any, Any]] | None
        ) = None,  # async fn to save to memory
    ) -> bool:
        """Flush session summary to long-term memory.

        Returns True if flush happened, False if already flushed or skipped.
        """
        if self._flushed:
            return False
        if save_fn:
            await save_fn(
                agent_id=agent_id,
                session_id=session_id,
                summary=f"[Auto-flush] {session_summary}",
                tags=["auto-flush"],
            )
        self._flushed = True
        return True

    @property
    def has_flushed(self) -> bool:
        """Whether a flush has occurred this session."""
        return self._flushed
