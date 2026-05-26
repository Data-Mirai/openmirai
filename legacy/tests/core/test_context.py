"""Tests for ExecutionContext and Resource Protocols."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import pytest

from datamirai_engine.core.context import (
    AuthContext,
    DBResource,
    ExecutionContext,
    LLMResource,
    MemoryResource,
    StorageResource,
    VectorResource,
)

# --- Mock implementations for protocol compliance ---


class MockDB:
    async def execute(self, query: str, params: dict[str, Any] | None = None) -> Any:
        return None

    async def fetch_one(self, query: str, params: dict[str, Any] | None = None) -> dict | None:
        return {"id": 1}

    async def fetch_all(self, query: str, params: dict[str, Any] | None = None) -> list[dict]:
        return [{"id": 1}]


class MockVector:
    async def search(
        self, embedding: list[float], *, limit: int = 10, filter: dict | None = None
    ) -> list[dict]:
        return [{"id": "doc1", "score": 0.95}]

    async def upsert(self, id: str, embedding: list[float], metadata: dict | None = None) -> None:
        pass


class MockStorage:
    async def get(self, key: str) -> bytes:
        return b"data"

    async def put(self, key: str, data: bytes, *, content_type: str | None = None) -> None:
        pass

    async def delete(self, key: str) -> None:
        pass

    async def presign(self, key: str, *, expires_in: int = 3600) -> str:
        return "https://example.com/presigned"


class MockLLM:
    async def call(
        self, *, model: str, prompt: str, context: str | None = None, **kwargs: Any
    ) -> dict:
        return {"text": "response", "tokens": 10}

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        return [0.1, 0.2, 0.3]


class MockMemory:
    @property
    def short_term(self) -> Any:
        return None

    @property
    def long_term(self) -> Any:
        return None


# --- Tests ---


class TestAuthContext:
    def test_create(self):
        auth = AuthContext(
            user_id="u1",
            role="ADMIN",
            universe_id="univ1",
            environment_id="env1",
        )
        assert auth.user_id == "u1"
        assert auth.role == "ADMIN"

    def test_defaults(self):
        auth = AuthContext(user_id="u1", role="VIEWER")
        assert auth.universe_id is None
        assert auth.environment_id is None

    def test_frozen(self):
        auth = AuthContext(user_id="u1", role="EDITOR")
        with pytest.raises(AttributeError):
            auth.role = "ADMIN"


class TestProtocolCompliance:
    """Verify mock implementations satisfy resource protocols."""

    def test_db_protocol(self):
        db: DBResource = MockDB()
        assert hasattr(db, "execute")
        assert hasattr(db, "fetch_one")
        assert hasattr(db, "fetch_all")

    def test_vector_protocol(self):
        vec: VectorResource = MockVector()
        assert hasattr(vec, "search")
        assert hasattr(vec, "upsert")

    def test_storage_protocol(self):
        st: StorageResource = MockStorage()
        assert hasattr(st, "get")
        assert hasattr(st, "put")
        assert hasattr(st, "delete")
        assert hasattr(st, "presign")

    def test_llm_protocol(self):
        llm: LLMResource = MockLLM()
        assert hasattr(llm, "call")
        assert hasattr(llm, "embed")

    def test_memory_protocol(self):
        mem: MemoryResource = MockMemory()
        assert hasattr(mem, "short_term")
        assert hasattr(mem, "long_term")


class TestExecutionContext:
    @pytest.fixture
    def ctx(self):
        @dataclass
        class ConcreteContext(ExecutionContext):
            _db: MockDB
            _vector: MockVector
            _storage: MockStorage
            _llm: MockLLM
            _memory: MockMemory
            _auth: AuthContext

            @property
            def db(self) -> DBResource:
                return self._db

            @property
            def vector(self) -> VectorResource:
                return self._vector

            @property
            def storage(self) -> StorageResource:
                return self._storage

            @property
            def llm(self) -> LLMResource:
                return self._llm

            @property
            def memory(self) -> MemoryResource:
                return self._memory

            @property
            def auth(self) -> AuthContext:
                return self._auth

        return ConcreteContext(
            _db=MockDB(),
            _vector=MockVector(),
            _storage=MockStorage(),
            _llm=MockLLM(),
            _memory=MockMemory(),
            _auth=AuthContext(
                user_id="u1", role="EDITOR", universe_id="univ1", environment_id="env1"
            ),
        )

    def test_access_db(self, ctx):
        assert ctx.db is not None

    def test_access_vector(self, ctx):
        assert ctx.vector is not None

    def test_access_storage(self, ctx):
        assert ctx.storage is not None

    def test_access_llm(self, ctx):
        assert ctx.llm is not None

    def test_access_memory(self, ctx):
        assert ctx.memory is not None

    def test_access_auth(self, ctx):
        assert ctx.auth.user_id == "u1"
        assert ctx.auth.role == "EDITOR"

    @pytest.mark.asyncio
    async def test_db_operations(self, ctx):
        result = await ctx.db.fetch_one("SELECT 1")
        assert result == {"id": 1}

    @pytest.mark.asyncio
    async def test_vector_operations(self, ctx):
        results = await ctx.vector.search([0.1, 0.2], limit=5)
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_storage_operations(self, ctx):
        data = await ctx.storage.get("file.txt")
        assert data == b"data"

    @pytest.mark.asyncio
    async def test_llm_operations(self, ctx):
        result = await ctx.llm.call(model="claude", prompt="hello")
        assert result["text"] == "response"

    @pytest.mark.asyncio
    async def test_llm_embed(self, ctx):
        embedding = await ctx.llm.embed("hello world")
        assert len(embedding) == 3
