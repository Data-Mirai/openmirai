"""Repositories — async CRUD for graphs, agents, sessions.

Each repository takes an asyncpg pool and provides typed operations.
All methods are standalone async functions grouped by entity.
"""

from __future__ import annotations

import json
import logging
from typing import Any

logger = logging.getLogger(__name__)


# ── Graph Repository ──────────────────────────────────────────


class GraphRepo:
    """CRUD for graphs table."""

    def __init__(self, pool: Any) -> None:
        self._pool = pool

    async def save(self, graph: dict[str, Any]) -> None:
        """Insert or update a graph."""
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO graphs (id, name, version, nodes, edges, metadata, updated_at)
                VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6::jsonb, now())
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    version = EXCLUDED.version,
                    nodes = EXCLUDED.nodes,
                    edges = EXCLUDED.edges,
                    metadata = EXCLUDED.metadata,
                    updated_at = now()
                """,
                graph["id"],
                graph["name"],
                graph.get("version", "1.0"),
                json.dumps(graph.get("nodes", [])),
                json.dumps(graph.get("edges", [])),
                json.dumps(graph.get("metadata", {})),
            )

    async def get(self, graph_id: str) -> dict[str, Any] | None:
        """Get a graph by ID."""
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow("SELECT * FROM graphs WHERE id = $1", graph_id)
            return _row_to_dict(row) if row else None

    async def list_all(self) -> list[dict[str, Any]]:
        """List all graphs."""
        async with self._pool.acquire() as conn:
            rows = await conn.fetch("SELECT * FROM graphs ORDER BY created_at DESC")
            return [_row_to_dict(r) for r in rows]

    async def delete(self, graph_id: str) -> bool:
        """Delete a graph. Returns True if deleted."""
        async with self._pool.acquire() as conn:
            result = await conn.execute("DELETE FROM graphs WHERE id = $1", graph_id)
            return result == "DELETE 1"


# ── Agent Repository ──────────────────────────────────────────


class AgentRepo:
    """CRUD for agents table."""

    def __init__(self, pool: Any) -> None:
        self._pool = pool

    async def save(self, agent: dict[str, Any]) -> None:
        """Insert or update an agent."""
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO agents (id, name, graph_id, status, triggers, metadata, updated_at)
                VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, now())
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    graph_id = EXCLUDED.graph_id,
                    status = EXCLUDED.status,
                    triggers = EXCLUDED.triggers,
                    metadata = EXCLUDED.metadata,
                    updated_at = now()
                """,
                agent["id"],
                agent["name"],
                agent["graph_id"],
                agent.get("status", "disabled"),
                json.dumps(agent.get("triggers", [])),
                json.dumps(agent.get("metadata", {})),
            )

    async def get(self, agent_id: str) -> dict[str, Any] | None:
        """Get an agent by ID."""
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow("SELECT * FROM agents WHERE id = $1", agent_id)
            return _row_to_dict(row) if row else None

    async def list_all(self) -> list[dict[str, Any]]:
        """List all agents."""
        async with self._pool.acquire() as conn:
            rows = await conn.fetch("SELECT * FROM agents ORDER BY created_at DESC")
            return [_row_to_dict(r) for r in rows]

    async def update_status(self, agent_id: str, status: str) -> None:
        """Update agent status."""
        async with self._pool.acquire() as conn:
            await conn.execute(
                "UPDATE agents SET status = $1, updated_at = now() WHERE id = $2",
                status, agent_id,
            )

    async def delete(self, agent_id: str) -> bool:
        """Delete an agent. Returns True if deleted."""
        async with self._pool.acquire() as conn:
            result = await conn.execute("DELETE FROM agents WHERE id = $1", agent_id)
            return result == "DELETE 1"


# ── Session Repository ────────────────────────────────────────


class SessionRepo:
    """CRUD for sessions table."""

    def __init__(self, pool: Any) -> None:
        self._pool = pool

    async def save(self, session: dict[str, Any]) -> None:
        """Insert or update a session."""
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO sessions (
                    id, agent_id, agent_name, graph_id, status,
                    trace, transcript, state, error,
                    started_at, finished_at, duration_ms
                )
                VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb, $9, $10, $11, $12)
                ON CONFLICT (id) DO UPDATE SET
                    status = EXCLUDED.status,
                    trace = EXCLUDED.trace,
                    transcript = EXCLUDED.transcript,
                    state = EXCLUDED.state,
                    error = EXCLUDED.error,
                    finished_at = EXCLUDED.finished_at,
                    duration_ms = EXCLUDED.duration_ms
                """,
                session["id"],
                session["agent_id"],
                session["agent_name"],
                session["graph_id"],
                session["status"],
                json.dumps(session.get("trace", [])),
                json.dumps(session.get("transcript", [])),
                json.dumps(session.get("state", {})),
                session.get("error"),
                session["started_at"],
                session.get("finished_at"),
                session.get("duration_ms"),
            )

    async def get(self, session_id: str) -> dict[str, Any] | None:
        """Get a session by ID."""
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow("SELECT * FROM sessions WHERE id = $1", session_id)
            return _row_to_dict(row) if row else None

    async def list_all(
        self, agent_id: str | None = None, limit: int = 50
    ) -> list[dict[str, Any]]:
        """List sessions, optionally filtered by agent."""
        async with self._pool.acquire() as conn:
            if agent_id:
                rows = await conn.fetch(
                    "SELECT * FROM sessions WHERE agent_id = $1 "
                    "ORDER BY started_at DESC LIMIT $2",
                    agent_id, limit,
                )
            else:
                rows = await conn.fetch(
                    "SELECT * FROM sessions ORDER BY started_at DESC LIMIT $1",
                    limit,
                )
            return [_row_to_dict(r) for r in rows]

    async def delete(self, session_id: str) -> bool:
        """Delete a session. Returns True if deleted."""
        async with self._pool.acquire() as conn:
            result = await conn.execute("DELETE FROM sessions WHERE id = $1", session_id)
            return result == "DELETE 1"


# ── Helpers ───────────────────────────────────────────────────


def _row_to_dict(row: Any) -> dict[str, Any]:
    """Convert an asyncpg Record to a plain dict with JSON fields parsed."""
    d = dict(row)
    # asyncpg returns JSONB as strings — parse them
    for key in ("nodes", "edges", "metadata", "triggers", "trace", "transcript", "state"):
        if key in d and isinstance(d[key], str):
            d[key] = json.loads(d[key])
    # Convert datetime to timestamp for API compatibility
    for key in ("created_at", "updated_at"):
        if key in d and d[key] is not None and not isinstance(d[key], (int, float)):
            d[key] = d[key].timestamp()
    return d
