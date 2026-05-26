"""InMemoryBackend -- default backend for dev/testing.

Same behavior as original LongTermMemory + SharedLog, wrapped in the
MemoryBackend interface. No persistence -- data lives in RAM only.
"""

from __future__ import annotations

import uuid
from datetime import datetime, timezone
from typing import Any

from .backend import LogEntry, MemoryBackend, MemoryEntry


class InMemoryBackend(MemoryBackend):
    """In-memory backend. For dev/testing. Same behavior as current LongTermMemory."""

    def __init__(self) -> None:
        self._memories: list[MemoryEntry] = []
        self._logs: list[LogEntry] = []

    async def save_learning(
        self,
        agent_id: str,
        session_id: str,
        summary: str,
        decisions: list[str] | None = None,
        learnings: list[str] | None = None,
        tags: list[str] | None = None,
    ) -> str:
        memory_id = str(uuid.uuid4())
        entry = MemoryEntry(
            id=memory_id,
            agent_id=agent_id,
            session_id=session_id,
            summary=summary,
            decisions=decisions or [],
            learnings=learnings or [],
            tags=tags or [],
            created_at=datetime.now(timezone.utc).isoformat(),
        )
        self._memories.append(entry)
        return memory_id

    async def search(
        self, agent_id: str, query: str, limit: int = 10
    ) -> list[MemoryEntry]:
        query_lower = query.lower()
        matches: list[MemoryEntry] = []
        for e in self._memories:
            if e.agent_id != agent_id:
                continue
            if (
                query_lower in e.summary.lower()
                or any(query_lower in t.lower() for t in e.tags)
                or any(query_lower in d.lower() for d in e.decisions)
                or any(query_lower in l.lower() for l in e.learnings)
            ):
                matches.append(e)
        # Most recent first
        matches.sort(key=lambda m: m.created_at, reverse=True)
        return matches[:limit]

    async def get_recent(
        self, agent_id: str, limit: int = 10
    ) -> list[MemoryEntry]:
        agent_entries = [e for e in self._memories if e.agent_id == agent_id]
        agent_entries.sort(key=lambda m: m.created_at, reverse=True)
        return agent_entries[:limit]

    async def delete(self, memory_id: str) -> bool:
        for i, e in enumerate(self._memories):
            if e.id == memory_id:
                self._memories.pop(i)
                return True
        return False

    async def save_log(
        self,
        agent_id: str,
        session_id: str,
        message: str,
        metadata: dict[str, Any] | None = None,
    ) -> str:
        log_id = str(uuid.uuid4())
        entry = LogEntry(
            id=log_id,
            agent_id=agent_id,
            session_id=session_id,
            message=message,
            metadata=metadata or {},
            created_at=datetime.now(timezone.utc).isoformat(),
        )
        self._logs.append(entry)
        return log_id

    async def get_logs(
        self,
        agent_id: str,
        limit: int = 50,
        session_id: str | None = None,
    ) -> list[LogEntry]:
        entries = [e for e in self._logs if e.agent_id == agent_id]
        if session_id:
            entries = [e for e in entries if e.session_id == session_id]
        entries.sort(key=lambda e: e.created_at, reverse=True)
        return entries[:limit]

    async def count(self, agent_id: str) -> int:
        return sum(1 for e in self._memories if e.agent_id == agent_id)
