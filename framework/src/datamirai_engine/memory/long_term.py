"""LongTermMemory + SharedLog -- persistent memory across sessions.

Supports pluggable backends via MemoryBackend ABC.
Default (no backend): in-memory, backward compatible with original behavior.
With backend: delegates to InMemoryBackend, SQLiteMemoryBackend, etc.
"""

from __future__ import annotations

import itertools
import time
from typing import Any

from .backend import MemoryBackend

_counter = itertools.count()


class LongTermMemory:
    """Persistent agent memory -- learnings and conclusions across sessions.

    With backend=None (default): original in-memory behavior, fully backward
    compatible. Tests that create LongTermMemory() keep working unchanged.

    With backend=MemoryBackend: delegates to the backend for persistence.
    """

    def __init__(self, backend: MemoryBackend | None = None) -> None:
        self._backend = backend
        # In-memory fallback for backward compat
        self._entries: list[dict[str, Any]] = []

    async def save_learning(
        self,
        session_id: str,
        agent_id: str,
        summary: str,
        decisions: list[str] | None = None,
        tags: list[str] | None = None,
    ) -> str | None:
        """Save a learning. Returns memory_id if backend, None if in-memory."""
        if self._backend:
            return await self._backend.save_learning(
                agent_id=agent_id,
                session_id=session_id,
                summary=summary,
                decisions=decisions,
                tags=tags,
            )
        self._entries.append({
            "session_id": session_id,
            "agent_id": agent_id,
            "summary": summary,
            "decisions": decisions or [],
            "tags": tags or [],
            "timestamp": time.time(),
            "_seq": next(_counter),
        })
        return None

    async def get_recent(
        self, limit: int = 10, agent_id: str | None = None
    ) -> list[dict[str, Any]]:
        if self._backend and agent_id:
            entries = await self._backend.get_recent(agent_id, limit)
            return [_entry_to_dict(e) for e in entries]
        sorted_entries = sorted(
            self._entries, key=lambda e: e["_seq"], reverse=True
        )
        return sorted_entries[:limit]

    async def search(
        self, query: str, limit: int = 10, agent_id: str | None = None
    ) -> list[dict[str, Any]]:
        """Text search. Backend uses FTS; fallback uses simple substring match."""
        if self._backend and agent_id:
            entries = await self._backend.search(agent_id, query, limit)
            return [_entry_to_dict(e) for e in entries]
        query_lower = query.lower()
        matches = [
            e for e in self._entries
            if query_lower in e["summary"].lower()
            or any(query_lower in t.lower() for t in e.get("tags", []))
            or any(query_lower in d.lower() for d in e.get("decisions", []))
        ]
        return sorted(matches, key=lambda e: e["_seq"], reverse=True)[:limit]


def _entry_to_dict(entry: Any) -> dict[str, Any]:
    """Convert a MemoryEntry dataclass to a dict (for backward compat)."""
    return {
        "id": entry.id,
        "session_id": entry.session_id,
        "agent_id": entry.agent_id,
        "summary": entry.summary,
        "decisions": entry.decisions,
        "learnings": entry.learnings,
        "tags": entry.tags,
        "score": entry.score,
        "created_at": entry.created_at,
    }


class SharedLog:
    """Shared log with authorship -- all agents write, any agent can read.

    Journal-style: each entry has author (agent_id). Enables agent coordination
    without direct coupling.

    With backend: delegates to backend.save_log / backend.get_logs.
    Without backend: original in-memory behavior.
    """

    def __init__(self, backend: MemoryBackend | None = None) -> None:
        self._backend = backend
        self._entries: list[dict[str, Any]] = []

    async def write(
        self,
        agent_id: str,
        session_id: str,
        message: str,
        metadata: dict[str, Any] | None = None,
    ) -> str | None:
        """Write a log entry. Returns log_id if backend, None if in-memory."""
        if self._backend:
            return await self._backend.save_log(
                agent_id=agent_id,
                session_id=session_id,
                message=message,
                metadata=metadata,
            )
        self._entries.append({
            "agent_id": agent_id,
            "session_id": session_id,
            "message": message,
            "metadata": metadata or {},
            "timestamp": time.time(),
            "_seq": next(_counter),
        })
        return None

    async def read(
        self,
        limit: int = 50,
        agent_id: str | None = None,
    ) -> list[dict[str, Any]]:
        if self._backend and agent_id:
            entries = await self._backend.get_logs(agent_id, limit)
            return [_log_to_dict(e) for e in entries]
        entries = self._entries
        if agent_id:
            entries = [e for e in entries if e["agent_id"] == agent_id]
        sorted_entries = sorted(entries, key=lambda e: e["_seq"], reverse=True)
        return sorted_entries[:limit]


def _log_to_dict(entry: Any) -> dict[str, Any]:
    """Convert a LogEntry dataclass to a dict (for backward compat)."""
    return {
        "id": entry.id,
        "agent_id": entry.agent_id,
        "session_id": entry.session_id,
        "message": entry.message,
        "metadata": entry.metadata,
        "created_at": entry.created_at,
    }
