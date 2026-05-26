"""Tests for memory backends: InMemoryBackend, SQLiteMemoryBackend, Factory."""

from __future__ import annotations

import pytest

from datamirai_engine.memory.backend import LogEntry, MemoryEntry
from datamirai_engine.memory.factory import MemoryBackendFactory
from datamirai_engine.memory.in_memory_backend import InMemoryBackend
from datamirai_engine.memory.long_term import LongTermMemory, SharedLog
from datamirai_engine.memory.sqlite_backend import SQLiteMemoryBackend


# ---------------------------------------------------------------------------
# InMemoryBackend
# ---------------------------------------------------------------------------


class TestInMemoryBackend:
    @pytest.fixture
    def backend(self):
        return InMemoryBackend()

    @pytest.mark.asyncio
    async def test_save_learning_returns_id(self, backend):
        mid = await backend.save_learning("a1", "s1", "Summary here")
        assert isinstance(mid, str)
        assert len(mid) > 0

    @pytest.mark.asyncio
    async def test_get_recent(self, backend):
        await backend.save_learning("a1", "s1", "First")
        await backend.save_learning("a1", "s2", "Second")
        await backend.save_learning("a1", "s3", "Third")

        recent = await backend.get_recent("a1", limit=2)
        assert len(recent) == 2
        assert recent[0].summary == "Third"
        assert recent[1].summary == "Second"

    @pytest.mark.asyncio
    async def test_get_recent_filters_by_agent(self, backend):
        await backend.save_learning("a1", "s1", "Agent 1 learning")
        await backend.save_learning("a2", "s2", "Agent 2 learning")

        recent = await backend.get_recent("a1")
        assert len(recent) == 1
        assert recent[0].agent_id == "a1"

    @pytest.mark.asyncio
    async def test_search_by_summary(self, backend):
        await backend.save_learning("a1", "s1", "Handle large files carefully")
        await backend.save_learning("a1", "s2", "Use Claude for long prompts")

        results = await backend.search("a1", "files")
        assert len(results) == 1
        assert "files" in results[0].summary.lower()

    @pytest.mark.asyncio
    async def test_search_by_tag(self, backend):
        await backend.save_learning(
            "a1", "s1", "Something", tags=["transcription", "limits"]
        )
        results = await backend.search("a1", "transcription")
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_search_by_decision(self, backend):
        await backend.save_learning(
            "a1", "s1", "Something", decisions=["Split files before transcribing"]
        )
        results = await backend.search("a1", "split")
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_search_by_learning(self, backend):
        await backend.save_learning(
            "a1", "s1", "Something", learnings=["Ollama is fast locally"]
        )
        results = await backend.search("a1", "ollama")
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_search_empty(self, backend):
        results = await backend.search("a1", "nonexistent xyz")
        assert results == []

    @pytest.mark.asyncio
    async def test_delete(self, backend):
        mid = await backend.save_learning("a1", "s1", "To be deleted")
        assert await backend.count("a1") == 1

        deleted = await backend.delete(mid)
        assert deleted is True
        assert await backend.count("a1") == 0

    @pytest.mark.asyncio
    async def test_delete_nonexistent(self, backend):
        deleted = await backend.delete("nonexistent-id")
        assert deleted is False

    @pytest.mark.asyncio
    async def test_count(self, backend):
        assert await backend.count("a1") == 0
        await backend.save_learning("a1", "s1", "One")
        await backend.save_learning("a1", "s2", "Two")
        await backend.save_learning("a2", "s3", "Other agent")
        assert await backend.count("a1") == 2
        assert await backend.count("a2") == 1

    @pytest.mark.asyncio
    async def test_save_log(self, backend):
        log_id = await backend.save_log("a1", "s1", "Processing started")
        assert isinstance(log_id, str)

    @pytest.mark.asyncio
    async def test_get_logs(self, backend):
        await backend.save_log("a1", "s1", "Step 1")
        await backend.save_log("a1", "s1", "Step 2")
        await backend.save_log("a2", "s2", "Other agent log")

        logs = await backend.get_logs("a1")
        assert len(logs) == 2
        # Most recent first
        assert logs[0].message == "Step 2"

    @pytest.mark.asyncio
    async def test_get_logs_by_session(self, backend):
        await backend.save_log("a1", "s1", "Session 1 log")
        await backend.save_log("a1", "s2", "Session 2 log")

        logs = await backend.get_logs("a1", session_id="s1")
        assert len(logs) == 1
        assert logs[0].session_id == "s1"

    @pytest.mark.asyncio
    async def test_entry_has_created_at(self, backend):
        await backend.save_learning("a1", "s1", "Test")
        recent = await backend.get_recent("a1", limit=1)
        assert recent[0].created_at != ""

    @pytest.mark.asyncio
    async def test_decisions_and_tags_stored(self, backend):
        await backend.save_learning(
            "a1", "s1", "Test",
            decisions=["d1", "d2"],
            learnings=["l1"],
            tags=["t1", "t2"],
        )
        recent = await backend.get_recent("a1", limit=1)
        entry = recent[0]
        assert entry.decisions == ["d1", "d2"]
        assert entry.learnings == ["l1"]
        assert entry.tags == ["t1", "t2"]


# ---------------------------------------------------------------------------
# SQLiteMemoryBackend
# ---------------------------------------------------------------------------


class TestSQLiteMemoryBackend:
    @pytest.fixture
    def backend(self, tmp_path):
        return SQLiteMemoryBackend(tmp_path / "test_memory.db")

    @pytest.mark.asyncio
    async def test_save_learning_returns_id(self, backend):
        mid = await backend.save_learning("a1", "s1", "Summary here")
        assert isinstance(mid, str)
        assert len(mid) > 0

    @pytest.mark.asyncio
    async def test_get_recent(self, backend):
        await backend.save_learning("a1", "s1", "First")
        await backend.save_learning("a1", "s2", "Second")
        await backend.save_learning("a1", "s3", "Third")

        recent = await backend.get_recent("a1", limit=2)
        assert len(recent) == 2
        assert recent[0].summary == "Third"
        assert recent[1].summary == "Second"

    @pytest.mark.asyncio
    async def test_get_recent_filters_by_agent(self, backend):
        await backend.save_learning("a1", "s1", "Agent 1 learning")
        await backend.save_learning("a2", "s2", "Agent 2 learning")

        recent = await backend.get_recent("a1")
        assert len(recent) == 1
        assert recent[0].agent_id == "a1"

    @pytest.mark.asyncio
    async def test_search_fts(self, backend):
        await backend.save_learning("a1", "s1", "Handle large files carefully")
        await backend.save_learning("a1", "s2", "Use Claude for long prompts")

        results = await backend.search("a1", "files")
        assert len(results) >= 1
        assert "files" in results[0].summary.lower()

    @pytest.mark.asyncio
    async def test_search_by_tag_in_fts(self, backend):
        await backend.save_learning(
            "a1", "s1", "Something generic",
            tags=["transcription", "limits"],
        )
        results = await backend.search("a1", "transcription")
        assert len(results) >= 1

    @pytest.mark.asyncio
    async def test_search_empty(self, backend):
        results = await backend.search("a1", "nonexistent xyz")
        assert results == []

    @pytest.mark.asyncio
    async def test_search_has_score(self, backend):
        await backend.save_learning("a1", "s1", "Files handling is important")
        results = await backend.search("a1", "files")
        assert len(results) >= 1
        assert results[0].score is not None

    @pytest.mark.asyncio
    async def test_delete(self, backend):
        mid = await backend.save_learning("a1", "s1", "To be deleted")
        assert await backend.count("a1") == 1

        deleted = await backend.delete(mid)
        assert deleted is True
        assert await backend.count("a1") == 0

    @pytest.mark.asyncio
    async def test_delete_nonexistent(self, backend):
        deleted = await backend.delete("nonexistent-id")
        assert deleted is False

    @pytest.mark.asyncio
    async def test_count(self, backend):
        assert await backend.count("a1") == 0
        await backend.save_learning("a1", "s1", "One")
        await backend.save_learning("a1", "s2", "Two")
        await backend.save_learning("a2", "s3", "Other agent")
        assert await backend.count("a1") == 2
        assert await backend.count("a2") == 1

    @pytest.mark.asyncio
    async def test_save_log(self, backend):
        log_id = await backend.save_log("a1", "s1", "Processing started")
        assert isinstance(log_id, str)

    @pytest.mark.asyncio
    async def test_get_logs(self, backend):
        await backend.save_log("a1", "s1", "Step 1")
        await backend.save_log("a1", "s1", "Step 2")
        await backend.save_log("a2", "s2", "Other agent log")

        logs = await backend.get_logs("a1")
        assert len(logs) == 2
        assert logs[0].message == "Step 2"

    @pytest.mark.asyncio
    async def test_get_logs_by_session(self, backend):
        await backend.save_log("a1", "s1", "Session 1 log")
        await backend.save_log("a1", "s2", "Session 2 log")

        logs = await backend.get_logs("a1", session_id="s1")
        assert len(logs) == 1
        assert logs[0].session_id == "s1"

    @pytest.mark.asyncio
    async def test_log_metadata(self, backend):
        await backend.save_log(
            "a1", "s1", "With metadata",
            metadata={"key": "value", "count": 42},
        )
        logs = await backend.get_logs("a1")
        assert logs[0].metadata == {"key": "value", "count": 42}

    @pytest.mark.asyncio
    async def test_persistence_across_instances(self, tmp_path):
        """Data survives creating a new backend instance on the same DB."""
        db_path = tmp_path / "persist_test.db"

        backend1 = SQLiteMemoryBackend(db_path)
        await backend1.save_learning("a1", "s1", "Persisted learning")
        await backend1.save_log("a1", "s1", "Persisted log")

        # New instance, same DB
        backend2 = SQLiteMemoryBackend(db_path)
        recent = await backend2.get_recent("a1")
        assert len(recent) == 1
        assert recent[0].summary == "Persisted learning"

        logs = await backend2.get_logs("a1")
        assert len(logs) == 1
        assert logs[0].message == "Persisted log"

    @pytest.mark.asyncio
    async def test_fts_deleted_entry_not_searchable(self, backend):
        """FTS index is cleaned up when a memory is deleted."""
        mid = await backend.save_learning("a1", "s1", "Unique searchable term xylophone")
        results = await backend.search("a1", "xylophone")
        assert len(results) == 1

        await backend.delete(mid)
        results = await backend.search("a1", "xylophone")
        assert len(results) == 0

    @pytest.mark.asyncio
    async def test_decisions_and_tags_roundtrip(self, backend):
        await backend.save_learning(
            "a1", "s1", "Test",
            decisions=["d1", "d2"],
            learnings=["l1"],
            tags=["t1", "t2"],
        )
        recent = await backend.get_recent("a1", limit=1)
        entry = recent[0]
        assert entry.decisions == ["d1", "d2"]
        assert entry.learnings == ["l1"]
        assert entry.tags == ["t1", "t2"]


# ---------------------------------------------------------------------------
# MemoryBackendFactory
# ---------------------------------------------------------------------------


class TestMemoryBackendFactory:
    def test_returns_in_memory_when_no_path(self):
        backend = MemoryBackendFactory.create()
        assert isinstance(backend, InMemoryBackend)

    def test_returns_in_memory_when_none(self):
        backend = MemoryBackendFactory.create(None)
        assert isinstance(backend, InMemoryBackend)

    def test_returns_sqlite_when_path(self, tmp_path):
        db_path = tmp_path / "factory_test.db"
        backend = MemoryBackendFactory.create(db_path)
        assert isinstance(backend, SQLiteMemoryBackend)

    def test_returns_sqlite_when_string_path(self, tmp_path):
        db_path = str(tmp_path / "factory_test2.db")
        backend = MemoryBackendFactory.create(db_path)
        assert isinstance(backend, SQLiteMemoryBackend)


# ---------------------------------------------------------------------------
# LongTermMemory with backend (integration)
# ---------------------------------------------------------------------------


class TestLongTermMemoryWithBackend:
    """LongTermMemory delegates to backend when one is provided."""

    @pytest.fixture
    def mem_with_backend(self):
        backend = InMemoryBackend()
        return LongTermMemory(backend=backend)

    @pytest.fixture
    def mem_no_backend(self):
        """Original behavior, no backend."""
        return LongTermMemory()

    @pytest.mark.asyncio
    async def test_backward_compat_no_backend(self, mem_no_backend):
        """Without backend, behaves exactly like original."""
        await mem_no_backend.save_learning(
            session_id="s1",
            agent_id="a1",
            summary="Test learning",
            decisions=["d1"],
            tags=["t1"],
        )
        recent = await mem_no_backend.get_recent(limit=1)
        assert len(recent) == 1
        assert recent[0]["summary"] == "Test learning"
        assert recent[0]["decisions"] == ["d1"]
        assert recent[0]["tags"] == ["t1"]

    @pytest.mark.asyncio
    async def test_backward_compat_search(self, mem_no_backend):
        await mem_no_backend.save_learning("s1", "a1", summary="Handle files")
        results = await mem_no_backend.search("files")
        assert len(results) == 1

    @pytest.mark.asyncio
    async def test_with_backend_save(self, mem_with_backend):
        result = await mem_with_backend.save_learning(
            session_id="s1",
            agent_id="a1",
            summary="Backend learning",
        )
        assert isinstance(result, str)  # returns memory_id

    @pytest.mark.asyncio
    async def test_with_backend_get_recent(self, mem_with_backend):
        await mem_with_backend.save_learning("s1", "a1", summary="One")
        await mem_with_backend.save_learning("s2", "a1", summary="Two")

        recent = await mem_with_backend.get_recent(limit=2, agent_id="a1")
        assert len(recent) == 2
        assert recent[0]["summary"] == "Two"

    @pytest.mark.asyncio
    async def test_with_backend_search(self, mem_with_backend):
        await mem_with_backend.save_learning("s1", "a1", summary="Handle large files")
        results = await mem_with_backend.search("files", agent_id="a1")
        assert len(results) >= 1

    @pytest.mark.asyncio
    async def test_with_sqlite_backend(self, tmp_path):
        """Full integration: LongTermMemory + SQLiteMemoryBackend."""
        backend = SQLiteMemoryBackend(tmp_path / "ltm_test.db")
        mem = LongTermMemory(backend=backend)

        await mem.save_learning("s1", "a1", summary="SQLite persisted")
        recent = await mem.get_recent(limit=1, agent_id="a1")
        assert len(recent) == 1
        assert recent[0]["summary"] == "SQLite persisted"


# ---------------------------------------------------------------------------
# SharedLog with backend (integration)
# ---------------------------------------------------------------------------


class TestSharedLogWithBackend:
    @pytest.fixture
    def log_no_backend(self):
        return SharedLog()

    @pytest.fixture
    def log_with_backend(self):
        return SharedLog(backend=InMemoryBackend())

    @pytest.mark.asyncio
    async def test_backward_compat(self, log_no_backend):
        await log_no_backend.write("a1", "s1", "Test message")
        entries = await log_no_backend.read(limit=10)
        assert len(entries) == 1
        assert entries[0]["message"] == "Test message"

    @pytest.mark.asyncio
    async def test_backward_compat_filter(self, log_no_backend):
        await log_no_backend.write("a1", "s1", "Agent 1")
        await log_no_backend.write("a2", "s2", "Agent 2")
        entries = await log_no_backend.read(agent_id="a1")
        assert len(entries) == 1

    @pytest.mark.asyncio
    async def test_with_backend_write(self, log_with_backend):
        result = await log_with_backend.write("a1", "s1", "Backend log")
        assert isinstance(result, str)  # returns log_id

    @pytest.mark.asyncio
    async def test_with_backend_read(self, log_with_backend):
        await log_with_backend.write("a1", "s1", "Step 1")
        await log_with_backend.write("a1", "s2", "Step 2")

        entries = await log_with_backend.read(agent_id="a1")
        assert len(entries) == 2
        assert entries[0]["message"] == "Step 2"
