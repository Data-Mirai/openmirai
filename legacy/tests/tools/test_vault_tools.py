"""Tests for vault_write and vault_read tools."""

import pytest

from datamirai_engine.core.context import AuthContext
from datamirai_engine.resources.context import SimpleExecutionContext
from datamirai_engine.resources.local_storage import LocalStorageResource
from datamirai_engine.resources.sqlite_db import SQLiteDBResource
from datamirai_engine.resources.llm import MockLLMResource
from datamirai_engine.resources.storage import InMemoryStorageResource
from datamirai_engine.resources.vector import InMemoryVectorResource
from datamirai_engine.vault.service import VaultService
from datamirai_engine.tools.builtin.data.vault_write import VaultWriteTool
from datamirai_engine.tools.builtin.data.vault_read import VaultReadTool


@pytest.fixture
def ctx(tmp_path):
    """Context with real vault (SQLite + local storage)."""
    db = SQLiteDBResource(tmp_path / "agent.db")
    storage = LocalStorageResource(tmp_path / "storage")
    vault = VaultService(storage=storage, db=db)
    return SimpleExecutionContext(
        db=db,
        vector=InMemoryVectorResource(),
        storage=storage,
        llm=MockLLMResource(),
        auth=AuthContext(user_id="test", role="OWNER"),
        vault=vault,
        session_id="test-session-001",
    )


class TestVaultWriteTool:
    @pytest.mark.asyncio
    async def test_basic_write(self, ctx):
        tool = VaultWriteTool()
        result = await tool.run(
            {"content": "# My Report\n\nSome analysis here."},
            {"folder": "vault/analysis", "tags": "finance,report"},
            ctx,
        )
        assert result["path"].startswith("vault/analysis/")
        assert result["path"].endswith(".md")
        assert result["title"] == "My Report"

    @pytest.mark.asyncio
    async def test_custom_title(self, ctx):
        tool = VaultWriteTool()
        result = await tool.run(
            {"content": "Content here", "title": "Custom Title"},
            {"folder": "vault/notes"},
            ctx,
        )
        assert result["title"] == "Custom Title"

    @pytest.mark.asyncio
    async def test_collision_appends_suffix(self, ctx):
        tool = VaultWriteTool()
        r1 = await tool.run(
            {"content": "First", "title": "Same Title"},
            {"folder": "vault/notes", "filename_pattern": "{slug}"},
            ctx,
        )
        r2 = await tool.run(
            {"content": "Second", "title": "Same Title"},
            {"folder": "vault/notes", "filename_pattern": "{slug}"},
            ctx,
        )
        assert r1["path"] != r2["path"]
        assert "-2" in r2["path"]

    @pytest.mark.asyncio
    async def test_wiki_links_in_output(self, ctx):
        tool = VaultWriteTool()
        result = await tool.run(
            {"content": "Links to [[fed-news]] and [[sp500]]."},
            {"folder": "vault/analysis"},
            ctx,
        )
        assert "fed-news" in result["outlinks"]
        assert "sp500" in result["outlinks"]

    @pytest.mark.asyncio
    async def test_injects_session_id(self, ctx):
        tool = VaultWriteTool()
        result = await tool.run(
            {"content": "Content"},
            {"folder": "vault/notes"},
            ctx,
        )
        # Read the note back to verify frontmatter
        note = await ctx.vault.read_note(result["path"])
        assert note.frontmatter.get("session_id") == "test-session-001"

    @pytest.mark.asyncio
    async def test_no_vault_raises(self):
        ctx_no_vault = SimpleExecutionContext.default()
        tool = VaultWriteTool()
        with pytest.raises(RuntimeError, match="Knowledge Vault"):
            await tool.run({"content": "test"}, {}, ctx_no_vault)


class TestVaultReadTool:
    @pytest.mark.asyncio
    async def test_read_mode(self, ctx):
        # Write a note first
        write = VaultWriteTool()
        w_result = await write.run(
            {"content": "# Test\n\nContent here."},
            {"folder": "vault/notes", "filename_pattern": "test-note"},
            ctx,
        )

        read = VaultReadTool()
        r_result = await read.run(
            {"note_path": w_result["path"]},
            {"mode": "read"},
            ctx,
        )
        assert r_result["count"] == 1
        assert "Content here." in r_result["notes"][0]["content"]

    @pytest.mark.asyncio
    async def test_search_mode(self, ctx):
        write = VaultWriteTool()
        await write.run(
            {"content": "Finance report"},
            {"folder": "vault/notes", "tags": "finance", "filename_pattern": "fin-1"},
            ctx,
        )
        await write.run(
            {"content": "Tech report"},
            {"folder": "vault/notes", "tags": "tech", "filename_pattern": "tech-1"},
            ctx,
        )

        read = VaultReadTool()
        result = await read.run(
            {},
            {"mode": "search", "tags": "finance"},
            ctx,
        )
        assert result["count"] == 1

    @pytest.mark.asyncio
    async def test_recent_mode(self, ctx):
        write = VaultWriteTool()
        for i in range(5):
            await write.run(
                {"content": f"Note {i}", "title": f"Note {i}"},
                {"folder": "vault/notes", "filename_pattern": f"note-{i}"},
                ctx,
            )

        read = VaultReadTool()
        result = await read.run({}, {"mode": "recent", "limit": "3"}, ctx)
        assert result["count"] == 3

    @pytest.mark.asyncio
    async def test_backlinks_mode(self, ctx):
        write = VaultWriteTool()
        await write.run(
            {"content": "I am the target.", "title": "Target"},
            {"folder": "vault/notes", "filename_pattern": "target"},
            ctx,
        )
        await write.run(
            {"content": "Links to [[target]]."},
            {"folder": "vault/notes", "filename_pattern": "source"},
            ctx,
        )

        read = VaultReadTool()
        result = await read.run(
            {"note_path": "vault/notes/target.md"},
            {"mode": "backlinks"},
            ctx,
        )
        assert result["count"] >= 1

    @pytest.mark.asyncio
    async def test_backlinks_without_path_raises(self, ctx):
        read = VaultReadTool()
        with pytest.raises(ValueError, match="note_path"):
            await read.run({}, {"mode": "backlinks"}, ctx)

    @pytest.mark.asyncio
    async def test_content_truncation(self, ctx):
        long_content = "A" * 20_000
        write = VaultWriteTool()
        result = await write.run(
            {"content": long_content, "title": "Long Note"},
            {"folder": "vault/notes", "filename_pattern": "long"},
            ctx,
        )

        read = VaultReadTool()
        r = await read.run(
            {"note_path": result["path"]},
            {"mode": "read", "include_content": True},
            ctx,
        )
        assert len(r["notes"][0]["content"]) <= 10_000 + 20  # margin for truncation text

    @pytest.mark.asyncio
    async def test_no_vault_raises(self):
        ctx_no_vault = SimpleExecutionContext.default()
        read = VaultReadTool()
        with pytest.raises(RuntimeError, match="Knowledge Vault"):
            await read.run({}, {"mode": "recent"}, ctx_no_vault)
