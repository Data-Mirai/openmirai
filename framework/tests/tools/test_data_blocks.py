"""Tests for Data blocks — db_read, db_write, storage_read, storage_write."""

from __future__ import annotations

import pytest

from datamirai_engine.tools.builtin.data.db_read import DBReadTool
from datamirai_engine.tools.builtin.data.db_write import DBWriteTool
from datamirai_engine.tools.builtin.data.storage_read import StorageReadTool
from datamirai_engine.tools.builtin.data.storage_write import StorageWriteTool
from datamirai_engine.resources.context import SimpleExecutionContext


@pytest.fixture
def ctx():
    return SimpleExecutionContext.default()


class TestDBWriteTool:
    @pytest.mark.asyncio
    async def test_write_row(self, ctx):
        tool = DBWriteTool()
        result = await tool.run(
            {"data": {"id": "1", "name": "Alice"}},
            {"table": "users"},
            ctx,
        )
        assert result["written"] is True
        assert result["table"] == "users"

    @pytest.mark.asyncio
    async def test_spec(self):
        assert DBWriteTool.spec.tool_type == "data/db_write"


class TestDBReadTool:
    @pytest.mark.asyncio
    async def test_read_after_write(self, ctx):
        # Write first
        writer = DBWriteTool()
        await writer.run(
            {"data": {"id": "1", "name": "Alice"}},
            {"table": "users"},
            ctx,
        )
        # Then read
        reader = DBReadTool()
        result = await reader.run(
            {"query_params": {"id": "1"}},
            {"table": "users", "mode": "one"},
            ctx,
        )
        assert result["data"] is not None
        assert result["data"]["name"] == "Alice"

    @pytest.mark.asyncio
    async def test_read_all(self, ctx):
        writer = DBWriteTool()
        await writer.run({"data": {"id": "1", "name": "A"}}, {"table": "t"}, ctx)
        await writer.run({"data": {"id": "2", "name": "B"}}, {"table": "t"}, ctx)

        reader = DBReadTool()
        result = await reader.run(
            {"query_params": {}},
            {"table": "t", "mode": "all"},
            ctx,
        )
        assert result["count"] == 2

    @pytest.mark.asyncio
    async def test_read_not_found(self, ctx):
        reader = DBReadTool()
        result = await reader.run(
            {"query_params": {"id": "999"}},
            {"table": "empty", "mode": "one"},
            ctx,
        )
        assert result["data"] is None


class TestStorageWriteTool:
    @pytest.mark.asyncio
    async def test_write_file(self, ctx):
        tool = StorageWriteTool()
        result = await tool.run(
            {"key": "docs/file.txt", "content": "hello world"},
            {},
            ctx,
        )
        assert result["written"] is True
        assert result["key"] == "docs/file.txt"

    @pytest.mark.asyncio
    async def test_write_bytes(self, ctx):
        tool = StorageWriteTool()
        result = await tool.run(
            {"key": "data.bin", "content": "binary data"},
            {"content_type": "application/octet-stream"},
            ctx,
        )
        assert result["written"] is True


class TestStorageReadTool:
    @pytest.mark.asyncio
    async def test_read_after_write(self, ctx):
        writer = StorageWriteTool()
        await writer.run({"key": "test.txt", "content": "hello"}, {}, ctx)

        reader = StorageReadTool()
        result = await reader.run({"key": "test.txt"}, {}, ctx)
        assert result["content"] == "hello"
        assert result["found"] is True

    @pytest.mark.asyncio
    async def test_read_not_found(self, ctx):
        reader = StorageReadTool()
        result = await reader.run({"key": "nonexistent.txt"}, {}, ctx)
        assert result["found"] is False
        assert result["content"] is None

    @pytest.mark.asyncio
    async def test_presigned_url(self, ctx):
        writer = StorageWriteTool()
        await writer.run({"key": "file.pdf", "content": "pdf data"}, {}, ctx)

        reader = StorageReadTool()
        result = await reader.run(
            {"key": "file.pdf"},
            {"mode": "presign", "expires_in": 7200},
            ctx,
        )
        assert result["url"] is not None
        assert "file.pdf" in result["url"]
