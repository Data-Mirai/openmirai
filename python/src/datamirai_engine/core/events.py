"""Event system for graph execution — streaming, checkpointing, telemetry."""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Protocol, runtime_checkable


class EventType(str, Enum):
    # Session lifecycle
    SESSION_STARTED = "session.started"
    SESSION_COMPLETED = "session.completed"
    SESSION_FAILED = "session.failed"
    SESSION_INTERRUPTED = "session.interrupted"
    SESSION_RESUMED = "session.resumed"

    # Block lifecycle
    BLOCK_STARTED = "block.started"
    BLOCK_COMPLETED = "block.completed"
    BLOCK_ERROR = "block.error"

    # LLM
    LLM_TOKEN = "llm.token"
    LLM_COMPLETED = "llm.completed"

    # Checkpointing
    CHECKPOINT_CREATED = "checkpoint.created"

    # Hooks
    HOOK_FIRED = "hook.fired"
    HOOK_BLOCKED = "hook.blocked"

    # Interrupts
    INTERRUPT_CREATED = "interrupt.created"
    INTERRUPT_RESOLVED = "interrupt.resolved"

    # Browser — FEAT-023 Tunnel Vision
    BROWSER_SCREENSHOT = "browser.screenshot"
    BROWSER_ACTION = "browser.action"
    BROWSER_NAVIGATION = "browser.navigation"
    BROWSER_COMPLETED = "browser.completed"


@dataclass
class ExecutionEvent:
    """Single event emitted during graph execution."""

    type: EventType
    timestamp: float = field(default_factory=time.time)
    session_id: str = ""
    node_id: str | None = None
    data: dict[str, Any] = field(default_factory=dict)

    # Auto-incrementing ID for SSE Last-Event-ID support
    event_id: int = 0

    def to_sse(self) -> str:
        """Format as Server-Sent Event."""
        import json

        lines = [f"id: {self.event_id}", f"event: {self.type.value}"]
        payload = {"timestamp": self.timestamp}
        if self.session_id:
            payload["session_id"] = self.session_id
        if self.node_id:
            payload["node_id"] = self.node_id
        payload.update(self.data)
        lines.append(f"data: {json.dumps(payload)}")
        return "\n".join(lines) + "\n\n"


class EventEmitter:
    """Broadcasts execution events to subscribers.

    Thread-safe. Supports multiple concurrent subscribers per session.
    Uses the main event loop for queue operations to work across threads.
    """

    def __init__(self) -> None:
        self._subscribers: dict[str, list[asyncio.Queue]] = {}
        self._event_counters: dict[str, int] = {}
        self._event_buffers: dict[str, list[ExecutionEvent]] = {}
        self._buffer_size = 100  # Keep last N events for reconnection
        self._main_loop: asyncio.AbstractEventLoop | None = None

    def set_loop(self, loop: asyncio.AbstractEventLoop) -> None:
        """Set the main event loop for thread-safe emit."""
        self._main_loop = loop

    def subscribe(self, session_id: str) -> asyncio.Queue:
        """Subscribe to events for a session. Returns a queue to read from."""
        if session_id not in self._subscribers:
            self._subscribers[session_id] = []
        queue: asyncio.Queue = asyncio.Queue()
        self._subscribers[session_id].append(queue)
        return queue

    def unsubscribe(self, session_id: str, queue: asyncio.Queue) -> None:
        """Remove a subscriber."""
        if session_id in self._subscribers:
            self._subscribers[session_id] = [
                q for q in self._subscribers[session_id] if q is not queue
            ]
            if not self._subscribers[session_id]:
                del self._subscribers[session_id]

    async def emit(self, event: ExecutionEvent) -> None:
        """Emit event to all subscribers of the session.

        Thread-safe: if called from a different thread than the main loop,
        uses call_soon_threadsafe to put events into subscriber queues.
        """
        sid = event.session_id
        # Assign event ID
        self._event_counters[sid] = self._event_counters.get(sid, 0) + 1
        event.event_id = self._event_counters[sid]

        # Buffer for reconnection
        if sid not in self._event_buffers:
            self._event_buffers[sid] = []
        self._event_buffers[sid].append(event)
        if len(self._event_buffers[sid]) > self._buffer_size:
            self._event_buffers[sid] = self._event_buffers[sid][-self._buffer_size :]

        # Broadcast — thread-safe
        current_loop = None
        try:
            current_loop = asyncio.get_running_loop()
        except RuntimeError:
            pass

        for queue in self._subscribers.get(sid, []):
            if current_loop is not None and current_loop == self._main_loop:
                # Same loop — direct put
                await queue.put(event)
            elif self._main_loop is not None and self._main_loop.is_running():
                # Different thread — use call_soon_threadsafe
                self._main_loop.call_soon_threadsafe(queue.put_nowait, event)
            else:
                # Fallback — try direct
                try:
                    queue.put_nowait(event)
                except Exception:
                    pass

    def get_events_since(self, session_id: str, last_event_id: int) -> list[ExecutionEvent]:
        """Get buffered events after a given event ID (for reconnection)."""
        return [
            e
            for e in self._event_buffers.get(session_id, [])
            if e.event_id > last_event_id
        ]

    def cleanup(self, session_id: str) -> None:
        """Remove all data for a completed session."""
        self._subscribers.pop(session_id, None)
        self._event_counters.pop(session_id, None)
        # Keep buffer for a while (reconnection after completion)


@runtime_checkable
class CheckpointCallback(Protocol):
    """Called after each block execution to persist state."""

    async def __call__(
        self,
        session_id: str,
        step_number: int,
        node_id: str,
        state_snapshot: dict[str, Any],
        cursor_position: str | None,
    ) -> str:
        """Save checkpoint, return checkpoint_id."""
        ...


@runtime_checkable
class HookHandler(Protocol):
    """Hook that can intercept execution flow."""

    async def __call__(
        self,
        hook_type: str,
        node_id: str | None,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        """Returns action dict: {"action": "continue|skip|abort", ...}."""
        ...
