"""Tests for concrete resource implementations."""

from __future__ import annotations

import pytest

from datamirai_engine.core.context import (
    DBResource,
    LLMResource,
    StorageResource,
    VectorResource,
)
from datamirai_engine.resources.db import InMemoryDBResource
from datamirai_engine.resources.llm import MockLLMResource
from datamirai_engine.resources.storage import InMemoryStorageResource
from datamirai_engine.resources.vector import InMemoryVectorResource


class TestInMemoryDBResource:
    @pytest.fixture
    def db(self):
        return InMemoryDBResource()

    @pytest.mark.asyncio
    async def test_protocol_compliance(self, db):
        assert isinstance(db, DBResource)

    @pytest.mark.asyncio
    async def test_execute_insert_and_fetch(self, db):
        await db.execute(
            "INSERT INTO users (id, name) VALUES (:id, :name)",
            {"id": "1", "name": "Alice", "__table__": "users"},
        )
        result = await db.fetch_one(
            "SELECT * FROM users WHERE id = :id",
            {"id": "1", "__table__": "users"},
        )
        assert result is not None
        assert result["name"] == "Alice"

    @pytest.mark.asyncio
    async def test_fetch_all(self, db):
        await db.execute("INSERT", {"id": "1", "name": "A", "__table__": "t"})
        await db.execute("INSERT", {"id": "2", "name": "B", "__table__": "t"})
        results = await db.fetch_all("SELECT *", {"__table__": "t"})
        assert len(results) == 2

    @pytest.mark.asyncio
    async def test_fetch_one_not_found(self, db):
        result = await db.fetch_one("SELECT", {"id": "999", "__table__": "t"})
        assert result is None

    @pytest.mark.asyncio
    async def test_execute_delete(self, db):
        await db.execute("INSERT", {"id": "1", "name": "A", "__table__": "t"})
        await db.execute("DELETE", {"id": "1", "__table__": "t"})
        result = await db.fetch_one("SELECT", {"id": "1", "__table__": "t"})
        assert result is None


class TestInMemoryVectorResource:
    @pytest.fixture
    def vector(self):
        return InMemoryVectorResource()

    @pytest.mark.asyncio
    async def test_protocol_compliance(self, vector):
        assert isinstance(vector, VectorResource)

    @pytest.mark.asyncio
    async def test_upsert_and_search(self, vector):
        await vector.upsert("doc1", [1.0, 0.0, 0.0], metadata={"title": "A"})
        await vector.upsert("doc2", [0.0, 1.0, 0.0], metadata={"title": "B"})
        results = await vector.search([1.0, 0.1, 0.0], limit=1)
        assert len(results) == 1
        assert results[0]["id"] == "doc1"

    @pytest.mark.asyncio
    async def test_search_returns_scores(self, vector):
        await vector.upsert("doc1", [1.0, 0.0], metadata={})
        results = await vector.search([1.0, 0.0], limit=5)
        assert "score" in results[0]

    @pytest.mark.asyncio
    async def test_search_empty(self, vector):
        results = await vector.search([1.0, 0.0])
        assert results == []

    @pytest.mark.asyncio
    async def test_upsert_overwrites(self, vector):
        await vector.upsert("doc1", [1.0, 0.0], metadata={"v": 1})
        await vector.upsert("doc1", [0.0, 1.0], metadata={"v": 2})
        results = await vector.search([0.0, 1.0], limit=1)
        assert results[0]["metadata"]["v"] == 2


class TestInMemoryStorageResource:
    @pytest.fixture
    def storage(self):
        return InMemoryStorageResource()

    @pytest.mark.asyncio
    async def test_protocol_compliance(self, storage):
        assert isinstance(storage, StorageResource)

    @pytest.mark.asyncio
    async def test_put_and_get(self, storage):
        await storage.put("file.txt", b"hello world", content_type="text/plain")
        data = await storage.get("file.txt")
        assert data == b"hello world"

    @pytest.mark.asyncio
    async def test_get_nonexistent_raises(self, storage):
        with pytest.raises(FileNotFoundError):
            await storage.get("nonexistent.txt")

    @pytest.mark.asyncio
    async def test_delete(self, storage):
        await storage.put("file.txt", b"data")
        await storage.delete("file.txt")
        with pytest.raises(FileNotFoundError):
            await storage.get("file.txt")

    @pytest.mark.asyncio
    async def test_presign(self, storage):
        await storage.put("file.txt", b"data")
        url = await storage.presign("file.txt", expires_in=3600)
        assert "file.txt" in url

    @pytest.mark.asyncio
    async def test_list_keys(self, storage):
        await storage.put("a/1.txt", b"1")
        await storage.put("a/2.txt", b"2")
        await storage.put("b/3.txt", b"3")
        keys = await storage.list_keys(prefix="a/")
        assert set(keys) == {"a/1.txt", "a/2.txt"}


class TestMockLLMResource:
    @pytest.fixture
    def llm(self):
        return MockLLMResource()

    @pytest.mark.asyncio
    async def test_protocol_compliance(self, llm):
        assert isinstance(llm, LLMResource)

    @pytest.mark.asyncio
    async def test_call_returns_response(self, llm):
        result = await llm.call(model="claude", prompt="hello")
        assert hasattr(result, "response")
        assert hasattr(result, "tokens_used")

    @pytest.mark.asyncio
    async def test_call_with_custom_response(self):
        llm = MockLLMResource(
            responses={"hello": {"text": "world", "tokens": 5}}
        )
        result = await llm.call(model="claude", prompt="hello")
        assert result.response == "world"

    @pytest.mark.asyncio
    async def test_embed_returns_vector(self, llm):
        embedding = await llm.embed("test text")
        assert isinstance(embedding, list)
        assert len(embedding) > 0
        assert all(isinstance(x, float) for x in embedding)

    @pytest.mark.asyncio
    async def test_embed_deterministic(self, llm):
        e1 = await llm.embed("same text")
        e2 = await llm.embed("same text")
        assert e1 == e2

    @pytest.mark.asyncio
    async def test_embed_different_texts_differ(self, llm):
        e1 = await llm.embed("text one")
        e2 = await llm.embed("text two")
        assert e1 != e2
