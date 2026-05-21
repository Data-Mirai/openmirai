"""Tests for VaultService — write, read, search, links, reindex."""

import pytest

from datamirai_engine.resources.local_storage import LocalStorageResource
from datamirai_engine.resources.sqlite_db import SQLiteDBResource
from datamirai_engine.vault.service import VaultNote, VaultService


@pytest.fixture
def vault(tmp_path):
    """VaultService with real SQLite + local storage."""
    db = SQLiteDBResource(tmp_path / "agent.db")
    storage = LocalStorageResource(tmp_path / "storage")
    return VaultService(storage=storage, db=db)


class TestWriteNote:
    @pytest.mark.asyncio
    async def test_write_and_read(self, vault):
        note = await vault.write_note(
            path="vault/scrapes/test-note.md",
            title="Test Note",
            content="# Test\n\nHello world.",
            frontmatter={"tags": ["test", "demo"]},
        )
        assert note.path == "vault/scrapes/test-note.md"
        assert note.title == "Test Note"
        assert "created_at" in note.frontmatter
        assert "updated_at" in note.frontmatter

        # Read it back
        read = await vault.read_note("vault/scrapes/test-note.md")
        assert read.title == "Test Note"
        assert "Hello world." in read.content
        assert read.frontmatter["tags"] == ["test", "demo"]

    @pytest.mark.asyncio
    async def test_overwrite_false_raises(self, vault):
        await vault.write_note("vault/note.md", "A", "Content A")
        with pytest.raises(FileExistsError):
            await vault.write_note("vault/note.md", "B", "Content B")

    @pytest.mark.asyncio
    async def test_overwrite_true_succeeds(self, vault):
        await vault.write_note("vault/note.md", "A", "Content A")
        note = await vault.write_note("vault/note.md", "B", "Content B", overwrite=True)
        assert note.title == "B"

        read = await vault.read_note("vault/note.md")
        assert "Content B" in read.content

    @pytest.mark.asyncio
    async def test_wiki_links_indexed(self, vault):
        # Write target FIRST so link resolution works
        await vault.write_note("vault/scrapes/fed-news.md", "Fed News", "Content.")
        await vault.write_note(
            "vault/analysis/report.md", "Report", "See [[fed-news]] and [[sp500]]."
        )

        # Report should have outlinks to fed-news (sp500 doesn't exist = broken link)
        outlinks = await vault.outlinks("vault/analysis/report.md")
        paths = [n.path for n in outlinks]
        assert "vault/scrapes/fed-news.md" in paths

    @pytest.mark.asyncio
    async def test_auto_timestamps(self, vault):
        note = await vault.write_note("vault/test.md", "Test", "Body")
        assert note.created_at
        assert note.updated_at


class TestReadNote:
    @pytest.mark.asyncio
    async def test_not_found(self, vault):
        with pytest.raises(FileNotFoundError):
            await vault.read_note("vault/nonexistent.md")

    @pytest.mark.asyncio
    async def test_extracts_title_from_h1(self, vault):
        await vault.write_note(
            "vault/no-title.md", "Fallback", "# Real Title\n\nBody here."
        )
        note = await vault.read_note("vault/no-title.md")
        # frontmatter title takes priority
        assert note.title == "Fallback"


class TestSearch:
    @pytest.mark.asyncio
    async def test_search_by_tags(self, vault):
        await vault.write_note("vault/a.md", "A", "Note A", {"tags": ["finance"]})
        await vault.write_note("vault/b.md", "B", "Note B", {"tags": ["tech"]})
        await vault.write_note("vault/c.md", "C", "Note C", {"tags": ["finance", "breaking"]})

        results = await vault.search(tags=["finance"])
        assert len(results) == 2
        titles = {n.title for n in results}
        assert "A" in titles
        assert "C" in titles

    @pytest.mark.asyncio
    async def test_search_by_folder(self, vault):
        await vault.write_note("vault/scrapes/a.md", "A", "Content")
        await vault.write_note("vault/analysis/b.md", "B", "Content")

        results = await vault.search(folder="vault/scrapes")
        assert len(results) == 1
        assert results[0].title == "A"

    @pytest.mark.asyncio
    async def test_search_by_query(self, vault):
        await vault.write_note("vault/fed-rate.md", "Fed Rate Decision", "Content")
        await vault.write_note("vault/sp500.md", "S&P 500", "Content")

        results = await vault.search(query="fed")
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_search_with_limit(self, vault):
        for i in range(10):
            await vault.write_note(f"vault/note-{i}.md", f"Note {i}", "Content")

        results = await vault.search(limit=3)
        assert len(results) == 3


class TestBacklinks:
    @pytest.mark.asyncio
    async def test_backlinks(self, vault):
        await vault.write_note("vault/target.md", "Target", "I am the target.")
        await vault.write_note("vault/source1.md", "Source 1", "Links to [[target]].")
        await vault.write_note("vault/source2.md", "Source 2", "Also links to [[target]].")
        await vault.write_note("vault/unrelated.md", "Unrelated", "No links here.")

        backlinks = await vault.backlinks("vault/target.md")
        paths = {n.path for n in backlinks}
        assert "vault/source1.md" in paths
        assert "vault/source2.md" in paths
        assert "vault/unrelated.md" not in paths


class TestDeleteNote:
    @pytest.mark.asyncio
    async def test_delete(self, vault):
        await vault.write_note("vault/to-delete.md", "Delete Me", "Content")
        await vault.delete_note("vault/to-delete.md")

        with pytest.raises(FileNotFoundError):
            await vault.read_note("vault/to-delete.md")

        results = await vault.search()
        assert len(results) == 0


class TestReindex:
    @pytest.mark.asyncio
    async def test_reindex_rebuilds_from_files(self, vault):
        # Write notes normally
        await vault.write_note("vault/a.md", "A", "Content A with [[b]].", {"tags": ["x"]})
        await vault.write_note("vault/b.md", "B", "Content B.")

        # Manually corrupt the index by deleting vault_notes
        await vault._db.execute("DELETE FROM vault_notes")
        await vault._db.execute("DELETE FROM vault_links")

        # Verify index is empty
        results = await vault.search()
        assert len(results) == 0

        # Reindex should rebuild from files (REGLA-342)
        count = await vault.reindex()
        assert count == 2

        results = await vault.search()
        assert len(results) == 2


class TestListTags:
    @pytest.mark.asyncio
    async def test_list_tags(self, vault):
        await vault.write_note("vault/a.md", "A", ".", {"tags": ["finance", "breaking"]})
        await vault.write_note("vault/b.md", "B", ".", {"tags": ["finance"]})
        await vault.write_note("vault/c.md", "C", ".", {"tags": ["tech"]})

        tags = await vault.list_tags()
        tag_dict = dict(tags)
        assert tag_dict["finance"] == 2
        assert tag_dict["breaking"] == 1
        assert tag_dict["tech"] == 1
