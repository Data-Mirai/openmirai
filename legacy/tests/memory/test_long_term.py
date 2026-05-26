"""Tests for LongTermMemory and shared log (Bitácora)."""

from __future__ import annotations

import pytest

from datamirai_engine.memory.long_term import LongTermMemory, SharedLog


class TestLongTermMemory:
    @pytest.fixture
    def mem(self):
        return LongTermMemory()

    @pytest.mark.asyncio
    async def test_save_learning(self, mem):
        await mem.save_learning(
            session_id="sess-001",
            agent_id="agent-001",
            summary="Large MP3 files >50MB fail transcription",
            decisions=["Split files before transcribing"],
            tags=["transcription", "limits"],
        )
        recent = await mem.get_recent(limit=1)
        assert len(recent) == 1
        assert recent[0]["summary"] == "Large MP3 files >50MB fail transcription"

    @pytest.mark.asyncio
    async def test_get_recent_ordering(self, mem):
        for i in range(5):
            await mem.save_learning(
                session_id=f"sess-{i}",
                agent_id="a1",
                summary=f"Learning {i}",
            )
        recent = await mem.get_recent(limit=3)
        assert len(recent) == 3
        # Most recent first
        assert recent[0]["summary"] == "Learning 4"

    @pytest.mark.asyncio
    async def test_search_by_text(self, mem):
        await mem.save_learning("s1", "a1", summary="Handle large files carefully")
        await mem.save_learning("s2", "a1", summary="Use Claude for long prompts")
        results = await mem.search("files")
        assert len(results) >= 1
        assert "files" in results[0]["summary"].lower()

    @pytest.mark.asyncio
    async def test_search_empty(self, mem):
        results = await mem.search("nonexistent topic xyz")
        assert results == []

    @pytest.mark.asyncio
    async def test_tags_stored(self, mem):
        await mem.save_learning("s1", "a1", summary="Test", tags=["tag1", "tag2"])
        recent = await mem.get_recent(limit=1)
        assert set(recent[0]["tags"]) == {"tag1", "tag2"}


class TestSharedLog:
    @pytest.fixture
    def log(self):
        return SharedLog()

    @pytest.mark.asyncio
    async def test_write_entry(self, log):
        await log.write(
            agent_id="agent-001",
            session_id="sess-001",
            message="Processed 15 meetings successfully",
        )
        entries = await log.read(limit=10)
        assert len(entries) == 1
        assert entries[0]["agent_id"] == "agent-001"

    @pytest.mark.asyncio
    async def test_multiple_agents(self, log):
        await log.write("agent-001", "s1", "Agent 1 did thing A")
        await log.write("agent-002", "s2", "Agent 2 did thing B")
        await log.write("agent-001", "s3", "Agent 1 did thing C")
        entries = await log.read(limit=10)
        assert len(entries) == 3

    @pytest.mark.asyncio
    async def test_read_by_agent(self, log):
        await log.write("a1", "s1", "msg1")
        await log.write("a2", "s2", "msg2")
        await log.write("a1", "s3", "msg3")
        entries = await log.read(agent_id="a1")
        assert len(entries) == 2
        assert all(e["agent_id"] == "a1" for e in entries)

    @pytest.mark.asyncio
    async def test_entries_have_timestamp(self, log):
        await log.write("a1", "s1", "test")
        entries = await log.read()
        assert "timestamp" in entries[0]

    @pytest.mark.asyncio
    async def test_ordering_most_recent_first(self, log):
        await log.write("a1", "s1", "first")
        await log.write("a1", "s2", "second")
        entries = await log.read()
        assert entries[0]["message"] == "second"
