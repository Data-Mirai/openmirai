"""Tests for SQLiteDBResource — local persistent database."""

from __future__ import annotations

import pytest
import tempfile
from pathlib import Path

from datamirai_engine.resources.sqlite_db import SQLiteDBResource


@pytest.fixture
def db(tmp_path):
    return SQLiteDBResource(tmp_path / "test.db")


class TestSQLiteDBResourceProtocol:
    """Verify the resource satisfies the DBResource protocol."""

    @pytest.mark.asyncio
    async def test_has_execute(self, db):
        assert hasattr(db, "execute")

    @pytest.mark.asyncio
    async def test_has_fetch_one(self, db):
        assert hasattr(db, "fetch_one")

    @pytest.mark.asyncio
    async def test_has_fetch_all(self, db):
        assert hasattr(db, "fetch_all")

    @pytest.mark.asyncio
    async def test_has_schema_methods(self, db):
        assert hasattr(db, "ensure_table")
        assert hasattr(db, "insert")
        assert hasattr(db, "query")


class TestSQLiteDBResourceInit:
    def test_creates_file(self, tmp_path):
        db_path = tmp_path / "sub" / "dir" / "test.db"
        db = SQLiteDBResource(db_path)
        assert db.path.parent.exists()

    def test_wal_mode(self, tmp_path):
        import sqlite3
        db = SQLiteDBResource(tmp_path / "test.db")
        conn = sqlite3.connect(str(db.path))
        mode = conn.execute("PRAGMA journal_mode").fetchone()[0]
        conn.close()
        assert mode == "wal"


class TestSQLiteDBResourceSchemaAware:
    @pytest.mark.asyncio
    async def test_ensure_table_creates(self, db):
        await db.ensure_table("news", [
            {"name": "url", "type": "text"},
            {"name": "title", "type": "text"},
            {"name": "score", "type": "float"},
        ])
        # Verify table exists
        import sqlite3
        conn = sqlite3.connect(str(db.path))
        tables = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()}
        conn.close()
        assert "news" in tables

    @pytest.mark.asyncio
    async def test_ensure_table_has_system_columns(self, db):
        await db.ensure_table("news", [{"name": "title", "type": "text"}])
        import sqlite3
        conn = sqlite3.connect(str(db.path))
        conn.row_factory = sqlite3.Row
        cols = {r["name"] for r in conn.execute("PRAGMA table_info(news)").fetchall()}
        conn.close()
        assert "id" in cols
        assert "created_at" in cols
        assert "session_id" in cols
        assert "node_id" in cols
        assert "title" in cols

    @pytest.mark.asyncio
    async def test_ensure_table_forward_migration(self, db):
        """Adding a column later should work via ALTER TABLE."""
        await db.ensure_table("data", [{"name": "a", "type": "text"}])
        await db.ensure_table("data", [
            {"name": "a", "type": "text"},
            {"name": "b", "type": "integer"},
        ])
        import sqlite3
        conn = sqlite3.connect(str(db.path))
        conn.row_factory = sqlite3.Row
        cols = {r["name"] for r in conn.execute("PRAGMA table_info(data)").fetchall()}
        conn.close()
        assert "a" in cols
        assert "b" in cols

    @pytest.mark.asyncio
    async def test_ensure_table_idempotent(self, db):
        """Calling ensure_table twice should not fail."""
        schema = [{"name": "x", "type": "text"}]
        await db.ensure_table("t", schema)
        await db.ensure_table("t", schema)

    @pytest.mark.asyncio
    async def test_insert_auto_id(self, db):
        await db.ensure_table("items", [{"name": "name", "type": "text"}])
        row = await db.insert("items", {"name": "Alice"})
        assert "id" in row
        assert len(row["id"]) > 0
        assert row["name"] == "Alice"
        assert "created_at" in row

    @pytest.mark.asyncio
    async def test_insert_with_meta(self, db):
        await db.ensure_table("items", [{"name": "val", "type": "text"}])
        row = await db.insert("items", {"val": "x"}, {"session_id": "s1", "node_id": "n1"})
        assert row["session_id"] == "s1"
        assert row["node_id"] == "n1"

    @pytest.mark.asyncio
    async def test_insert_explicit_id(self, db):
        await db.ensure_table("items", [{"name": "val", "type": "text"}])
        row = await db.insert("items", {"id": "custom-123", "val": "y"})
        assert row["id"] == "custom-123"

    @pytest.mark.asyncio
    async def test_query_all(self, db):
        await db.ensure_table("items", [{"name": "name", "type": "text"}])
        await db.insert("items", {"name": "A"})
        await db.insert("items", {"name": "B"})
        rows = await db.query("items")
        assert len(rows) == 2

    @pytest.mark.asyncio
    async def test_query_with_where(self, db):
        await db.ensure_table("items", [{"name": "name", "type": "text"}])
        await db.insert("items", {"name": "Alice"})
        await db.insert("items", {"name": "Bob"})
        rows = await db.query("items", where={"name": "Alice"})
        assert len(rows) == 1
        assert rows[0]["name"] == "Alice"

    @pytest.mark.asyncio
    async def test_query_with_limit(self, db):
        await db.ensure_table("items", [{"name": "val", "type": "integer"}])
        for i in range(5):
            await db.insert("items", {"val": i})
        rows = await db.query("items", limit=3)
        assert len(rows) == 3

    @pytest.mark.asyncio
    async def test_query_with_order(self, db):
        await db.ensure_table("items", [{"name": "name", "type": "text"}])
        await db.insert("items", {"name": "C"})
        await db.insert("items", {"name": "A"})
        await db.insert("items", {"name": "B"})
        rows = await db.query("items", order_by="name ASC")
        names = [r["name"] for r in rows]
        assert names == ["A", "B", "C"]

    @pytest.mark.asyncio
    async def test_query_empty_table(self, db):
        await db.ensure_table("empty", [{"name": "x", "type": "text"}])
        rows = await db.query("empty")
        assert rows == []


class TestSQLiteDBResourceLegacy:
    """Test backward compat with __table__ convention."""

    @pytest.mark.asyncio
    async def test_legacy_write_read(self, db):
        await db.execute("INSERT", {"__table__": "users", "id": "1", "name": "Alice"})
        row = await db.fetch_one("SELECT", {"__table__": "users", "id": "1"})
        assert row is not None
        assert row["name"] == "Alice"

    @pytest.mark.asyncio
    async def test_legacy_fetch_all(self, db):
        await db.execute("INSERT", {"__table__": "t", "id": "1", "a": "x"})
        await db.execute("INSERT", {"__table__": "t", "id": "2", "a": "y"})
        rows = await db.fetch_all("SELECT", {"__table__": "t"})
        assert len(rows) == 2

    @pytest.mark.asyncio
    async def test_legacy_delete(self, db):
        await db.execute("INSERT", {"__table__": "t", "id": "1", "a": "x"})
        await db.execute("DELETE", {"__table__": "t", "id": "1"})
        row = await db.fetch_one("SELECT", {"__table__": "t", "id": "1"})
        assert row is None
