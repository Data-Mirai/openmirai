"""Tests for repositories — uses mock pool (no real Postgres needed)."""

from __future__ import annotations

import json
from unittest.mock import AsyncMock, MagicMock

import pytest

from datamirai_engine.db.repositories import GraphRepo, AgentRepo, SessionRepo, _row_to_dict


# --- Mock asyncpg pool/connection ---


def _make_pool():
    """Create a mock pool that returns a mock connection via acquire()."""
    conn = AsyncMock()
    pool = MagicMock()

    # pool.acquire() returns an async context manager yielding conn
    cm = AsyncMock()
    cm.__aenter__ = AsyncMock(return_value=conn)
    cm.__aexit__ = AsyncMock(return_value=False)
    pool.acquire.return_value = cm

    return pool, conn


# --- GraphRepo ---


class TestGraphRepo:
    @pytest.mark.asyncio
    async def test_save(self):
        pool, conn = _make_pool()
        repo = GraphRepo(pool)

        await repo.save({
            "id": "g1", "name": "Test", "version": "1.0",
            "nodes": [{"id": "n1"}], "edges": [], "metadata": {},
        })

        conn.execute.assert_called_once()
        args = conn.execute.call_args
        assert args[0][1] == "g1"  # id
        assert args[0][2] == "Test"  # name

    @pytest.mark.asyncio
    async def test_get_found(self):
        pool, conn = _make_pool()
        repo = GraphRepo(pool)

        # Mock a record-like return
        conn.fetchrow.return_value = {
            "id": "g1", "name": "Test", "version": "1.0",
            "nodes": json.dumps([]), "edges": json.dumps([]),
            "metadata": json.dumps({}), "created_at": 1000.0, "updated_at": 1000.0,
        }

        result = await repo.get("g1")
        assert result is not None
        assert result["id"] == "g1"
        assert result["nodes"] == []

    @pytest.mark.asyncio
    async def test_get_not_found(self):
        pool, conn = _make_pool()
        repo = GraphRepo(pool)
        conn.fetchrow.return_value = None

        result = await repo.get("nonexistent")
        assert result is None

    @pytest.mark.asyncio
    async def test_list_all(self):
        pool, conn = _make_pool()
        repo = GraphRepo(pool)
        conn.fetch.return_value = [
            {"id": "g1", "name": "A", "version": "1.0",
             "nodes": "[]", "edges": "[]", "metadata": "{}",
             "created_at": 1000.0, "updated_at": 1000.0},
        ]

        result = await repo.list_all()
        assert len(result) == 1
        assert result[0]["id"] == "g1"

    @pytest.mark.asyncio
    async def test_delete(self):
        pool, conn = _make_pool()
        repo = GraphRepo(pool)
        conn.execute.return_value = "DELETE 1"

        deleted = await repo.delete("g1")
        assert deleted is True


# --- AgentRepo ---


class TestAgentRepo:
    @pytest.mark.asyncio
    async def test_save(self):
        pool, conn = _make_pool()
        repo = AgentRepo(pool)

        await repo.save({
            "id": "a1", "name": "Agent", "graph_id": "g1",
            "status": "disabled", "triggers": [], "metadata": {},
        })

        conn.execute.assert_called_once()

    @pytest.mark.asyncio
    async def test_update_status(self):
        pool, conn = _make_pool()
        repo = AgentRepo(pool)

        await repo.update_status("a1", "enabled")
        conn.execute.assert_called_once()
        args = conn.execute.call_args[0]
        assert args[1] == "enabled"
        assert args[2] == "a1"


# --- SessionRepo ---


class TestSessionRepo:
    @pytest.mark.asyncio
    async def test_save(self):
        pool, conn = _make_pool()
        repo = SessionRepo(pool)

        await repo.save({
            "id": "s1", "agent_id": "a1", "agent_name": "Test",
            "graph_id": "g1", "status": "completed",
            "trace": [{"node_id": "n1"}],
            "transcript": [{"type": "started", "message": "Go"}],
            "state": {"n1": {"result": True}},
            "started_at": 1000.0, "finished_at": 1001.0,
            "duration_ms": 1000.0,
        })

        conn.execute.assert_called_once()

    @pytest.mark.asyncio
    async def test_get(self):
        pool, conn = _make_pool()
        repo = SessionRepo(pool)
        conn.fetchrow.return_value = {
            "id": "s1", "agent_id": "a1", "agent_name": "Test",
            "graph_id": "g1", "status": "completed",
            "trace": "[]", "transcript": "[]", "state": "{}",
            "error": None, "started_at": 1000.0,
            "finished_at": 1001.0, "duration_ms": 1000.0,
        }

        result = await repo.get("s1")
        assert result is not None
        assert result["id"] == "s1"
        assert result["trace"] == []
        assert result["transcript"] == []

    @pytest.mark.asyncio
    async def test_list_all(self):
        pool, conn = _make_pool()
        repo = SessionRepo(pool)
        conn.fetch.return_value = []

        result = await repo.list_all()
        assert result == []

    @pytest.mark.asyncio
    async def test_list_filtered(self):
        pool, conn = _make_pool()
        repo = SessionRepo(pool)
        conn.fetch.return_value = []

        await repo.list_all(agent_id="a1", limit=10)
        args = conn.fetch.call_args[0]
        assert "agent_id" in args[0].lower()


# --- _row_to_dict ---


class TestRowToDict:
    def test_parses_json_strings(self):
        row = {
            "id": "g1",
            "nodes": json.dumps([{"id": "n1"}]),
            "metadata": json.dumps({"key": "val"}),
        }
        result = _row_to_dict(row)
        assert result["nodes"] == [{"id": "n1"}]
        assert result["metadata"] == {"key": "val"}

    def test_leaves_non_json_fields(self):
        row = {"id": "g1", "name": "Test"}
        result = _row_to_dict(row)
        assert result["name"] == "Test"
