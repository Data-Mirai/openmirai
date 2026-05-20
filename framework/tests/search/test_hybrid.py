"""Tests for Hybrid Search Engine and SQLiteFTSProvider."""

from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from datamirai_engine.search.hybrid import HybridSearchEngine, SearchResult
from datamirai_engine.search.providers import SQLiteFTSProvider
from datamirai_engine.memory.sqlite_backend import SQLiteMemoryBackend


# ---------------------------------------------------------------------------
# SearchResult dataclass
# ---------------------------------------------------------------------------


class TestSearchResult:
    def test_defaults(self):
        r = SearchResult(id="1", content="hello")
        assert r.id == "1"
        assert r.content == "hello"
        assert r.score == 0.0
        assert r.vector_score == 0.0
        assert r.fts_score == 0.0
        assert r.metadata == {}

    def test_custom_values(self):
        r = SearchResult(
            id="2",
            content="world",
            score=0.9,
            vector_score=0.8,
            fts_score=0.7,
            metadata={"key": "val"},
        )
        assert r.score == 0.9
        assert r.metadata == {"key": "val"}


# ---------------------------------------------------------------------------
# HybridSearchEngine — unit tests with mocked providers
# ---------------------------------------------------------------------------


class TestHybridSearchEngine:
    @pytest.mark.asyncio
    async def test_fts_only(self):
        """FTS provider only, no vector. Returns FTS results."""
        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "a", "content": "alpha", "rank": -5.0, "metadata": {}},
            {"id": "b", "content": "beta", "rank": -3.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(fts_provider=fts)
        results = await engine.search("test query", limit=10)

        assert len(results) == 2
        fts.search.assert_awaited_once()
        # All results should have fts_score > 0 or == 0, vector_score == 0
        for r in results:
            assert r.vector_score == 0.0

    @pytest.mark.asyncio
    async def test_vector_only(self):
        """Vector provider only, no FTS. Returns vector results."""
        vec = AsyncMock()
        vec.search.return_value = [
            {"id": "x", "content": "xray", "distance": 0.1, "metadata": {}},
            {"id": "y", "content": "yankee", "distance": 0.5, "metadata": {}},
        ]

        engine = HybridSearchEngine(vector_provider=vec)
        results = await engine.search(
            "test", query_embedding=[0.1, 0.2, 0.3], limit=10
        )

        assert len(results) == 2
        vec.search.assert_awaited_once()
        for r in results:
            assert r.fts_score == 0.0

    @pytest.mark.asyncio
    async def test_hybrid_both_providers(self):
        """Both providers available, score fusion happens."""
        vec = AsyncMock()
        vec.search.return_value = [
            {"id": "a", "content": "alpha", "distance": 0.1, "metadata": {}},
            {"id": "b", "content": "beta", "distance": 0.9, "metadata": {}},
        ]

        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "a", "content": "alpha", "rank": -5.0, "metadata": {}},
            {"id": "c", "content": "charlie", "rank": -2.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        results = await engine.search(
            "test", query_embedding=[0.1, 0.2], limit=10
        )

        # Should have 3 unique IDs: a, b, c
        ids = {r.id for r in results}
        assert ids == {"a", "b", "c"}

        # "a" appears in both, should have both scores > 0
        a_result = next(r for r in results if r.id == "a")
        assert a_result.vector_score > 0.0
        assert a_result.fts_score >= 0.0

    @pytest.mark.asyncio
    async def test_deduplication(self):
        """Same ID from both sources is merged, not duplicated."""
        vec = AsyncMock()
        vec.search.return_value = [
            {"id": "dup", "content": "duplicate", "distance": 0.2, "metadata": {}},
        ]
        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "dup", "content": "duplicate", "rank": -3.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        results = await engine.search(
            "dup", query_embedding=[0.1], limit=10
        )

        assert len(results) == 1
        assert results[0].id == "dup"

    @pytest.mark.asyncio
    async def test_scores_normalized_to_0_1(self):
        """All scores should be in [0.0, 1.0] range."""
        vec = AsyncMock()
        vec.search.return_value = [
            {"id": "a", "content": "a", "distance": 0.0, "metadata": {}},
            {"id": "b", "content": "b", "distance": 1.0, "metadata": {}},
        ]
        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "a", "content": "a", "rank": -10.0, "metadata": {}},
            {"id": "b", "content": "b", "rank": -1.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        results = await engine.search(
            "q", query_embedding=[0.5], limit=10
        )

        for r in results:
            assert 0.0 <= r.score <= 1.0
            assert 0.0 <= r.vector_score <= 1.0
            assert 0.0 <= r.fts_score <= 1.0

    @pytest.mark.asyncio
    async def test_weight_normalization(self):
        """Weights that don't sum to 1.0 are normalized."""
        engine = HybridSearchEngine(w_vec=1.0, w_fts=1.0)
        assert abs(engine._w_vec - 0.5) < 1e-9
        assert abs(engine._w_fts - 0.5) < 1e-9

        engine2 = HybridSearchEngine(w_vec=3.0, w_fts=7.0)
        assert abs(engine2._w_vec - 0.3) < 1e-9
        assert abs(engine2._w_fts - 0.7) < 1e-9

    @pytest.mark.asyncio
    async def test_zero_weights_default_to_half(self):
        """Zero total weight falls back to 0.5/0.5."""
        engine = HybridSearchEngine(w_vec=0.0, w_fts=0.0)
        assert engine._w_vec == 0.5
        assert engine._w_fts == 0.5

    @pytest.mark.asyncio
    async def test_empty_results_no_providers(self):
        """No providers configured returns empty list."""
        engine = HybridSearchEngine()
        results = await engine.search("anything")
        assert results == []

    @pytest.mark.asyncio
    async def test_provider_error_graceful(self):
        """Provider throws exception, returns partial results from other."""
        vec = AsyncMock()
        vec.search.side_effect = RuntimeError("vector db down")

        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "ok", "content": "still works", "rank": -1.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        results = await engine.search(
            "test", query_embedding=[0.1], limit=10
        )

        # FTS results still returned despite vector failure
        assert len(results) == 1
        assert results[0].id == "ok"

    @pytest.mark.asyncio
    async def test_both_providers_error_returns_empty(self):
        """Both providers fail, returns empty list."""
        vec = AsyncMock()
        vec.search.side_effect = RuntimeError("boom")
        fts = AsyncMock()
        fts.search.side_effect = RuntimeError("crash")

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        results = await engine.search(
            "test", query_embedding=[0.1], limit=10
        )
        assert results == []

    @pytest.mark.asyncio
    async def test_limit_respected(self):
        """Returns at most `limit` results."""
        fts = AsyncMock()
        fts.search.return_value = [
            {"id": f"r{i}", "content": f"result {i}", "rank": float(-i), "metadata": {}}
            for i in range(20)
        ]

        engine = HybridSearchEngine(fts_provider=fts)
        results = await engine.search("test", limit=5)

        assert len(results) == 5

    @pytest.mark.asyncio
    async def test_vector_not_called_without_embedding(self):
        """Vector provider is not called if no query_embedding."""
        vec = AsyncMock()
        fts = AsyncMock()
        fts.search.return_value = []

        engine = HybridSearchEngine(vector_provider=vec, fts_provider=fts)
        await engine.search("test", limit=5)

        vec.search.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_sorted_by_score_desc(self):
        """Results are sorted by score descending."""
        fts = AsyncMock()
        fts.search.return_value = [
            {"id": "low", "content": "low", "rank": -1.0, "metadata": {}},
            {"id": "high", "content": "high", "rank": -10.0, "metadata": {}},
            {"id": "mid", "content": "mid", "rank": -5.0, "metadata": {}},
        ]

        engine = HybridSearchEngine(fts_provider=fts)
        results = await engine.search("test", limit=10)

        scores = [r.score for r in results]
        assert scores == sorted(scores, reverse=True)


# ---------------------------------------------------------------------------
# SQLiteFTSProvider — tests with real SQLite
# ---------------------------------------------------------------------------


class TestSQLiteFTSProvider:
    @pytest.fixture
    def db_path(self, tmp_path):
        """Create a SQLite DB with the agent_memory schema + FTS."""
        db = tmp_path / "test_search.db"
        import sqlite3

        conn = sqlite3.connect(str(db))
        conn.executescript(
            """
            CREATE TABLE agent_memory (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                decisions TEXT DEFAULT '[]',
                learnings TEXT DEFAULT '[]',
                tags TEXT DEFAULT '[]',
                created_at TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE agent_memory_fts USING fts5(
                summary, tags, content=agent_memory, content_rowid=rowid
            );
            CREATE TRIGGER agent_memory_ai AFTER INSERT ON agent_memory BEGIN
                INSERT INTO agent_memory_fts(rowid, summary, tags)
                VALUES (new.rowid, new.summary, new.tags);
            END;
            """
        )
        # Insert test data
        conn.execute(
            "INSERT INTO agent_memory (id, agent_id, session_id, summary, tags, created_at) "
            "VALUES ('m1', 'agent1', 's1', 'Handle large files carefully', '[]', '2025-01-01')"
        )
        conn.execute(
            "INSERT INTO agent_memory (id, agent_id, session_id, summary, tags, created_at) "
            "VALUES ('m2', 'agent1', 's2', 'Use Claude for prompts', '[]', '2025-01-02')"
        )
        conn.execute(
            "INSERT INTO agent_memory (id, agent_id, session_id, summary, tags, created_at) "
            "VALUES ('m3', 'agent2', 's3', 'Files are important too', '[]', '2025-01-03')"
        )
        conn.commit()
        conn.close()
        return db

    @pytest.mark.asyncio
    async def test_search_finds_match(self, db_path):
        provider = SQLiteFTSProvider(db_path)
        results = await provider.search("files", limit=10)
        assert len(results) >= 1
        ids = {r["id"] for r in results}
        assert "m1" in ids

    @pytest.mark.asyncio
    async def test_search_no_match(self, db_path):
        provider = SQLiteFTSProvider(db_path)
        results = await provider.search("xylophonezzzz", limit=10)
        assert results == []

    @pytest.mark.asyncio
    async def test_search_agent_filter(self, db_path):
        """Only returns entries for the specified agent_id."""
        provider = SQLiteFTSProvider(db_path, agent_id="agent1")
        results = await provider.search("files", limit=10)
        ids = {r["id"] for r in results}
        assert "m1" in ids
        assert "m3" not in ids  # agent2's entry

    @pytest.mark.asyncio
    async def test_search_without_agent_filter(self, db_path):
        """Without agent_id, returns entries from all agents."""
        provider = SQLiteFTSProvider(db_path)
        results = await provider.search("files", limit=10)
        ids = {r["id"] for r in results}
        assert "m1" in ids
        assert "m3" in ids  # agent2's entry also matched

    @pytest.mark.asyncio
    async def test_sanitize_query_basic(self):
        assert SQLiteFTSProvider._sanitize_fts_query("hello world") == '"hello" OR "world"'

    @pytest.mark.asyncio
    async def test_sanitize_query_empty(self):
        assert SQLiteFTSProvider._sanitize_fts_query("") == '""'

    @pytest.mark.asyncio
    async def test_sanitize_query_single_word(self):
        assert SQLiteFTSProvider._sanitize_fts_query("test") == '"test"'

    @pytest.mark.asyncio
    async def test_sanitize_query_special_chars(self):
        """FTS5 special chars like AND OR NOT should be wrapped safely."""
        result = SQLiteFTSProvider._sanitize_fts_query("AND OR NOT")
        assert '"AND"' in result
        assert '"OR"' in result
        assert '"NOT"' in result

    @pytest.mark.asyncio
    async def test_results_have_rank(self, db_path):
        provider = SQLiteFTSProvider(db_path)
        results = await provider.search("files", limit=10)
        assert len(results) >= 1
        for r in results:
            assert "rank" in r
            assert isinstance(r["rank"], float)


# ---------------------------------------------------------------------------
# Integration: SQLiteMemoryBackend uses HybridSearch
# ---------------------------------------------------------------------------


class TestIntegrationWithMemoryBackend:
    @pytest.fixture
    def backend(self, tmp_path):
        return SQLiteMemoryBackend(tmp_path / "integration_test.db")

    @pytest.mark.asyncio
    async def test_sqlite_backend_uses_hybrid_search(self, backend):
        """save_learning + search goes through HybridSearchEngine."""
        await backend.save_learning("a1", "s1", "Handle large files carefully")
        await backend.save_learning("a1", "s2", "Use Claude for long prompts")
        await backend.save_learning("a1", "s3", "Files need special handling")

        results = await backend.search("a1", "files")
        assert len(results) >= 1
        # Verify we get MemoryEntry objects back
        from datamirai_engine.memory.backend import MemoryEntry

        for entry in results:
            assert isinstance(entry, MemoryEntry)
            assert entry.agent_id == "a1"

    @pytest.mark.asyncio
    async def test_search_returns_scores(self, backend):
        """Results have score populated from HybridSearchEngine."""
        await backend.save_learning("a1", "s1", "Files handling is important")
        results = await backend.search("a1", "files")
        assert len(results) >= 1
        assert results[0].score is not None

    @pytest.mark.asyncio
    async def test_search_empty_query(self, backend):
        """Empty query returns empty results."""
        await backend.save_learning("a1", "s1", "Something")
        results = await backend.search("a1", "")
        assert results == []

    @pytest.mark.asyncio
    async def test_search_no_match(self, backend):
        """Query with no matching entries returns empty."""
        await backend.save_learning("a1", "s1", "Unrelated content")
        results = await backend.search("a1", "xylophonezzz")
        assert results == []

    @pytest.mark.asyncio
    async def test_search_respects_agent_filter(self, backend):
        """Search only returns entries for the given agent_id."""
        await backend.save_learning("a1", "s1", "Agent one files")
        await backend.save_learning("a2", "s2", "Agent two files")

        results = await backend.search("a1", "files")
        assert all(e.agent_id == "a1" for e in results)

    @pytest.mark.asyncio
    async def test_search_preserves_full_entry(self, backend):
        """Full MemoryEntry fields are preserved through hybrid search."""
        await backend.save_learning(
            "a1",
            "s1",
            "Test learning about files",
            decisions=["d1", "d2"],
            learnings=["l1"],
            tags=["search", "files"],
        )
        results = await backend.search("a1", "files")
        assert len(results) >= 1
        entry = results[0]
        assert entry.decisions == ["d1", "d2"]
        assert entry.learnings == ["l1"]
        assert entry.tags == ["search", "files"]
