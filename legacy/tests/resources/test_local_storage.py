"""Tests for LocalStorageResource — filesystem-based object storage."""

from __future__ import annotations

import pytest

from datamirai_engine.resources.local_storage import LocalStorageResource


@pytest.fixture
def storage(tmp_path):
    return LocalStorageResource(tmp_path / "storage")


class TestLocalStorageProtocol:
    @pytest.mark.asyncio
    async def test_has_methods(self, storage):
        assert hasattr(storage, "get")
        assert hasattr(storage, "put")
        assert hasattr(storage, "delete")
        assert hasattr(storage, "presign")


class TestLocalStorageInit:
    def test_creates_directory(self, tmp_path):
        path = tmp_path / "new" / "dir" / "storage"
        s = LocalStorageResource(path)
        assert s.path.exists()


class TestLocalStoragePutGet:
    @pytest.mark.asyncio
    async def test_put_and_get(self, storage):
        await storage.put("test.txt", b"hello world")
        data = await storage.get("test.txt")
        assert data == b"hello world"

    @pytest.mark.asyncio
    async def test_put_nested_path(self, storage):
        await storage.put("docs/sub/file.txt", b"nested")
        data = await storage.get("docs/sub/file.txt")
        assert data == b"nested"

    @pytest.mark.asyncio
    async def test_get_not_found(self, storage):
        with pytest.raises(FileNotFoundError):
            await storage.get("nonexistent.txt")

    @pytest.mark.asyncio
    async def test_overwrite(self, storage):
        await storage.put("f.txt", b"v1")
        await storage.put("f.txt", b"v2")
        assert await storage.get("f.txt") == b"v2"


class TestLocalStorageDelete:
    @pytest.mark.asyncio
    async def test_delete_existing(self, storage):
        await storage.put("f.txt", b"data")
        await storage.delete("f.txt")
        with pytest.raises(FileNotFoundError):
            await storage.get("f.txt")

    @pytest.mark.asyncio
    async def test_delete_nonexistent(self, storage):
        # Should not raise
        await storage.delete("nope.txt")


class TestLocalStoragePresign:
    @pytest.mark.asyncio
    async def test_presign_returns_file_uri(self, storage):
        url = await storage.presign("test.txt")
        assert url.startswith("file://")
        assert "test.txt" in url


class TestLocalStorageList:
    @pytest.mark.asyncio
    async def test_list_keys(self, storage):
        await storage.put("a.txt", b"1")
        await storage.put("b.txt", b"2")
        await storage.put("sub/c.txt", b"3")
        keys = await storage.list_keys()
        assert len(keys) == 3
        assert "a.txt" in keys
        assert "sub/c.txt" in keys

    @pytest.mark.asyncio
    async def test_list_keys_with_prefix(self, storage):
        await storage.put("docs/a.txt", b"1")
        await storage.put("docs/b.txt", b"2")
        await storage.put("other/c.txt", b"3")
        keys = await storage.list_keys("docs")
        assert len(keys) == 2

    @pytest.mark.asyncio
    async def test_list_empty(self, storage):
        keys = await storage.list_keys()
        assert keys == []
