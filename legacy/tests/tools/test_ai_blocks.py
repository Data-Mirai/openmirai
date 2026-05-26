"""Tests for AI blocks — llm_call, transcribe, embeddings."""

from __future__ import annotations

import json

import pytest

from datamirai_engine.tools.builtin.ai.embeddings import EmbeddingsTool
from datamirai_engine.tools.builtin.ai.llm_call import LLMCallTool
from datamirai_engine.tools.builtin.ai.transcribe import TranscribeTool
from datamirai_engine.resources.context import SimpleExecutionContext


@pytest.fixture
def ctx():
    return SimpleExecutionContext.default()


class TestLLMCallTool:
    """LLM block acts as a data processor: prompt + session data → grounded response."""

    @pytest.mark.asyncio
    async def test_basic_call_with_session_data(self, ctx):
        """LLM receives prompt + data from previous nodes via data_map."""
        tool = LLMCallTool()
        result = await tool.run(
            {
                "prompt": "Summarize the following news",
                "news_data": "Breaking: AI advances in 2026...",
            },
            {"model": "claude"},
            ctx,
        )
        assert "response" in result
        assert "tokens_used" in result

    @pytest.mark.asyncio
    async def test_session_context_formatted_in_llm_call(self, ctx):
        """All non-prompt inputs are formatted as structured session context."""
        # Track what the mock LLM receives
        received = {}
        original_call = ctx.llm.call

        async def spy_call(*, model, prompt, context=None, **kwargs):
            received["prompt"] = prompt
            received["context"] = context
            return await original_call(model=model, prompt=prompt, context=context, **kwargs)

        ctx.llm.call = spy_call

        tool = LLMCallTool()
        await tool.run(
            {
                "prompt": "Analyze this data",
                "web_scrape_results": "Article about tech...",
                "db_records": [{"id": 1, "title": "Record A"}],
            },
            {"model": "claude"},
            ctx,
        )
        # Session context must contain both data sources
        assert received["context"] is not None
        assert "web_scrape_results" in received["context"]
        assert "db_records" in received["context"]
        assert "Article about tech" in received["context"]
        assert "Record A" in received["context"]

    @pytest.mark.asyncio
    async def test_missing_session_data_raises(self, ctx):
        """LLM block MUST have data to process — prompt alone is not enough."""
        tool = LLMCallTool()
        with pytest.raises(ValueError, match=r"session context.*empty"):
            await tool.run(
                {"prompt": "Generate something"},
                {"model": "claude"},
                ctx,
            )

    @pytest.mark.asyncio
    async def test_missing_prompt_raises(self, ctx):
        tool = LLMCallTool()
        with pytest.raises(ValueError, match=r"prompt is required"):
            await tool.run({"news_data": "some data"}, {"model": "claude"}, ctx)

    @pytest.mark.asyncio
    async def test_multiple_data_sources_in_context(self, ctx):
        """Multiple nodes feeding data into LLM — all appear in session context."""
        received = {}
        original_call = ctx.llm.call

        async def spy_call(*, model, prompt, context=None, **kwargs):
            received["context"] = context
            return await original_call(model=model, prompt=prompt, context=context, **kwargs)

        ctx.llm.call = spy_call

        tool = LLMCallTool()
        await tool.run(
            {
                "prompt": "Cross-reference these sources",
                "source_a": "Data from web scrape",
                "source_b": {"records": [1, 2, 3]},
                "source_c": ["item1", "item2"],
            },
            {"model": "claude"},
            ctx,
        )
        ctx_text = received["context"]
        assert "source_a" in ctx_text
        assert "source_b" in ctx_text
        assert "source_c" in ctx_text

    @pytest.mark.asyncio
    async def test_non_string_data_serialized(self, ctx):
        """Dicts, lists, numbers in session data are JSON-serialized."""
        received = {}
        original_call = ctx.llm.call

        async def spy_call(*, model, prompt, context=None, **kwargs):
            received["context"] = context
            return await original_call(model=model, prompt=prompt, context=context, **kwargs)

        ctx.llm.call = spy_call

        tool = LLMCallTool()
        await tool.run(
            {
                "prompt": "Process this",
                "structured_data": {"key": "value", "count": 42},
            },
            {"model": "claude"},
            ctx,
        )
        assert '"key": "value"' in received["context"]
        assert '"count": 42' in received["context"]

    @pytest.mark.asyncio
    async def test_prompt_from_config_with_session_data(self, ctx):
        """Static prompt from config works with session data from inputs."""
        tool = LLMCallTool()
        result = await tool.run(
            {"research_data": "Important findings..."},
            {"model": "claude", "prompt": "Summarize the research"},
            ctx,
        )
        assert "response" in result

    def test_spec_metadata(self):
        assert LLMCallTool.spec.tool_type == "ai/llm_call"
        assert LLMCallTool.spec.category == "ai"


class TestTranscribeTool:
    @pytest.mark.asyncio
    async def test_transcribe(self, ctx):
        tool = TranscribeTool()
        result = await tool.run(
            {"audio_key": "recordings/meeting.mp3"},
            {"model": "whisper-1"},
            ctx,
        )
        assert "text" in result
        assert "duration_seconds" in result

    @pytest.mark.asyncio
    async def test_missing_audio_raises(self, ctx):
        tool = TranscribeTool()
        with pytest.raises(ValueError, match=r"Missing required input"):
            await tool.run({}, {}, ctx)

    def test_spec_metadata(self):
        assert TranscribeTool.spec.tool_type == "ai/transcribe"


class TestEmbeddingsTool:
    @pytest.mark.asyncio
    async def test_embed_text(self, ctx):
        tool = EmbeddingsTool()
        result = await tool.run(
            {"text": "hello world"},
            {},
            ctx,
        )
        assert "embedding" in result
        assert isinstance(result["embedding"], list)
        assert len(result["embedding"]) > 0

    @pytest.mark.asyncio
    async def test_embed_dimension(self, ctx):
        tool = EmbeddingsTool()
        result = await tool.run({"text": "test"}, {}, ctx)
        assert "dimension" in result
        assert result["dimension"] == len(result["embedding"])

    def test_spec_metadata(self):
        assert EmbeddingsTool.spec.tool_type == "ai/embeddings"
