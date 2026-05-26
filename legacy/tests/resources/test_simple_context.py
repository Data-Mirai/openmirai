"""Tests for SimpleExecutionContext — wires all resources together."""

from __future__ import annotations

import pytest

from datamirai_engine.core.context import AuthContext, ExecutionContext
from datamirai_engine.resources.context import SimpleExecutionContext
from datamirai_engine.resources.db import InMemoryDBResource
from datamirai_engine.resources.llm import MockLLMResource
from datamirai_engine.resources.storage import InMemoryStorageResource
from datamirai_engine.resources.vector import InMemoryVectorResource


class TestSimpleExecutionContext:
    @pytest.fixture
    def ctx(self):
        return SimpleExecutionContext(
            db=InMemoryDBResource(),
            vector=InMemoryVectorResource(),
            storage=InMemoryStorageResource(),
            llm=MockLLMResource(),
            auth=AuthContext(user_id="u1", role="EDITOR"),
        )

    def test_is_execution_context(self, ctx):
        assert isinstance(ctx, ExecutionContext)

    def test_access_all_resources(self, ctx):
        assert ctx.db is not None
        assert ctx.vector is not None
        assert ctx.storage is not None
        assert ctx.llm is not None
        assert ctx.memory is not None
        assert ctx.auth is not None

    def test_auth_info(self, ctx):
        assert ctx.auth.user_id == "u1"
        assert ctx.auth.role == "EDITOR"

    @pytest.mark.asyncio
    async def test_e2e_db_through_context(self, ctx):
        await ctx.db.execute("INSERT", {"id": "1", "val": "x", "__table__": "t"})
        row = await ctx.db.fetch_one("SELECT", {"id": "1", "__table__": "t"})
        assert row["val"] == "x"

    @pytest.mark.asyncio
    async def test_e2e_storage_through_context(self, ctx):
        await ctx.storage.put("test.txt", b"content")
        data = await ctx.storage.get("test.txt")
        assert data == b"content"

    @pytest.mark.asyncio
    async def test_e2e_llm_through_context(self, ctx):
        result = await ctx.llm.call(model="test", prompt="hi")
        assert hasattr(result, "response")

    @pytest.mark.asyncio
    async def test_e2e_vector_through_context(self, ctx):
        await ctx.vector.upsert("doc1", [1.0, 0.0], metadata={"a": 1})
        results = await ctx.vector.search([1.0, 0.0], limit=1)
        assert len(results) == 1

    def test_default_context_factory(self):
        """SimpleExecutionContext.default() creates a fully wired dev context."""
        ctx = SimpleExecutionContext.default()
        assert isinstance(ctx, ExecutionContext)
        assert ctx.auth.role == "OWNER"
