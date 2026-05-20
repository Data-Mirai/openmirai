"""Memory Backend -- abstract interface for persistent agent memory.

Defines the ABC that all memory backends implement, plus data models
for memory entries and log entries.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any


@dataclass
class MemoryEntry:
    """A single memory (learning, decision, conclusion) persisted by an agent."""

    id: str
    agent_id: str
    session_id: str
    summary: str
    decisions: list[str] = field(default_factory=list)
    learnings: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)
    score: float | None = None  # search relevance score
    created_at: str = ""


@dataclass
class LogEntry:
    """A single log entry written by an agent (SharedLog equivalent)."""

    id: str
    agent_id: str
    session_id: str
    message: str
    metadata: dict[str, Any] = field(default_factory=dict)
    created_at: str = ""


class MemoryBackend(ABC):
    """Abstract base class for memory persistence backends.

    Implementations: InMemoryBackend (dev/testing), SQLiteMemoryBackend (production).
    """

    @abstractmethod
    async def save_learning(
        self,
        agent_id: str,
        session_id: str,
        summary: str,
        decisions: list[str] | None = None,
        learnings: list[str] | None = None,
        tags: list[str] | None = None,
    ) -> str:
        """Save a learning. Returns memory_id."""

    @abstractmethod
    async def search(
        self, agent_id: str, query: str, limit: int = 10
    ) -> list[MemoryEntry]:
        """Search memories by text (FTS). Semantic search if embeddings available."""

    @abstractmethod
    async def get_recent(
        self, agent_id: str, limit: int = 10
    ) -> list[MemoryEntry]:
        """Get most recent memories."""

    @abstractmethod
    async def delete(self, memory_id: str) -> bool:
        """Delete a specific memory. Returns True if deleted, False if not found."""

    @abstractmethod
    async def save_log(
        self,
        agent_id: str,
        session_id: str,
        message: str,
        metadata: dict[str, Any] | None = None,
    ) -> str:
        """Save a log entry. Returns log_id."""

    @abstractmethod
    async def get_logs(
        self,
        agent_id: str,
        limit: int = 50,
        session_id: str | None = None,
    ) -> list[LogEntry]:
        """Get log entries, optionally filtered by session."""

    @abstractmethod
    async def count(self, agent_id: str) -> int:
        """Count memories for an agent."""
