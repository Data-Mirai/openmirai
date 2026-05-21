"""Tests for LLMCallTool — output_schema validation + context truncation."""

import json
from dataclasses import dataclass, field
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from datamirai_engine.tools.builtin.ai.llm_call import LLMCallTool


@dataclass(frozen=True)
class _FakeResponse:
    """Mimics NormalizedResponse for tests."""
    response: str
    tokens_used: dict = field(default_factory=lambda: {"input": 0, "output": 0})


def _resp(text: str, output_tokens: int = 10) -> _FakeResponse:
    return _FakeResponse(response=text, tokens_used={"input": 0, "output": output_tokens})


@pytest.fixture
def tool():
    return LLMCallTool()


@pytest.fixture
def mock_context():
    ctx = MagicMock()
    ctx.llm = MagicMock()
    ctx.llm.call = AsyncMock(return_value=_resp("hello"))
    ctx.llm.stream = None  # disable streaming for tests
    ctx.events = None
    ctx.session_id = None
    ctx.node_id = None
    ctx.system_prompt = ""
    return ctx


TRADE_SCHEMA = {
    "type": "object",
    "properties": {
        "action": {"type": "string", "enum": ["BUY", "SELL", "HOLD"]},
        "confidence": {"type": "number"},
        "reasoning": {"type": "string"},
    },
    "required": ["action", "confidence", "reasoning"],
}


class TestOutputSchemaConfig:
    def test_spec_has_output_schema_field(self, tool):
        names = [c.name for c in tool.spec.config]
        assert "output_schema" in names
        assert "schema_retries" in names

    def test_spec_has_structured_output(self, tool):
        names = [o.name for o in tool.spec.outputs]
        assert "structured_output" in names
        assert "schema_valid" in names


class TestParseOutputSchema:
    def test_empty_string_returns_none(self, tool):
        assert tool._parse_output_schema("") is None
        assert tool._parse_output_schema(None) is None

    def test_valid_json_string(self, tool):
        result = tool._parse_output_schema('{"type": "object"}')
        assert result == {"type": "object"}

    def test_dict_passthrough(self, tool):
        schema = {"type": "object", "properties": {"x": {"type": "number"}}}
        assert tool._parse_output_schema(schema) is schema

    def test_invalid_json_returns_none(self, tool):
        assert tool._parse_output_schema("not json") is None


class TestExtractJson:
    def test_direct_json(self, tool):
        text = '{"action": "BUY", "confidence": 85}'
        result = tool._extract_json_from_response(text)
        assert result == text

    def test_markdown_code_block(self, tool):
        text = 'Here is the result:\n```json\n{"action": "HOLD"}\n```\nDone.'
        result = tool._extract_json_from_response(text)
        assert result == '{"action": "HOLD"}'

    def test_embedded_json(self, tool):
        text = 'The analysis shows: {"action": "SELL", "confidence": 70} based on data.'
        result = tool._extract_json_from_response(text)
        parsed = json.loads(result)
        assert parsed["action"] == "SELL"

    def test_no_json(self, tool):
        text = "This is just plain text with no JSON."
        assert tool._extract_json_from_response(text) is None


class TestValidateResponse:
    def test_valid_response(self, tool):
        response = json.dumps({
            "action": "BUY",
            "confidence": 85,
            "reasoning": "Strong bullish signal",
        })
        parsed, errors = tool._validate_response(response, TRADE_SCHEMA)
        assert errors == []
        assert parsed["action"] == "BUY"
        assert parsed["confidence"] == 85

    def test_missing_required_field(self, tool):
        response = json.dumps({"action": "BUY", "confidence": 85})
        parsed, errors = tool._validate_response(response, TRADE_SCHEMA)
        assert any("reasoning" in e for e in errors)

    def test_wrong_type(self, tool):
        response = json.dumps({
            "action": "BUY",
            "confidence": "high",  # should be number
            "reasoning": "test",
        })
        parsed, errors = tool._validate_response(response, TRADE_SCHEMA)
        assert any("confidence" in e for e in errors)

    def test_invalid_enum(self, tool):
        response = json.dumps({
            "action": "MAYBE",
            "confidence": 50,
            "reasoning": "unsure",
        })
        parsed, errors = tool._validate_response(response, TRADE_SCHEMA)
        assert any("action" in e and "one of" in e for e in errors)

    def test_not_json(self, tool):
        parsed, errors = tool._validate_response("not json at all", TRADE_SCHEMA)
        assert parsed is None
        assert len(errors) > 0


class TestExecuteWithSchema:
    @pytest.mark.asyncio
    async def test_valid_schema_response(self, tool, mock_context):
        """LLM returns valid JSON matching schema on first try."""
        valid_json = json.dumps({
            "action": "HOLD",
            "confidence": 45,
            "reasoning": "No clear signal",
        })
        mock_context.llm.call = AsyncMock(return_value=_resp(valid_json, 20))

        result = await tool.execute(
            inputs={"market_data": "gold is stable"},
            config={
                "prompt": "Analyze the market",
                "output_schema": json.dumps(TRADE_SCHEMA),
                "stream": False,
            },
            context=mock_context,
        )

        assert result["schema_valid"] is True
        assert result["structured_output"]["action"] == "HOLD"
        assert result["structured_output"]["confidence"] == 45

    @pytest.mark.asyncio
    async def test_retry_on_invalid_then_success(self, tool, mock_context):
        """LLM fails first, succeeds on retry."""
        bad_response = _resp("I think we should buy gold")
        good_response = _resp(json.dumps({
            "action": "BUY", "confidence": 80, "reasoning": "strong signal"
        }), 15)

        mock_context.llm.call = AsyncMock(side_effect=[bad_response, good_response])

        result = await tool.execute(
            inputs={"data": "some data"},
            config={
                "prompt": "Analyze",
                "output_schema": json.dumps(TRADE_SCHEMA),
                "schema_retries": 2,
                "stream": False,
            },
            context=mock_context,
        )

        assert result["schema_valid"] is True
        assert result["structured_output"]["action"] == "BUY"
        assert mock_context.llm.call.call_count == 2

    @pytest.mark.asyncio
    async def test_all_retries_exhausted(self, tool, mock_context):
        """All retries fail — returns best effort."""
        bad = _resp("just text", 5)
        mock_context.llm.call = AsyncMock(return_value=bad)

        result = await tool.execute(
            inputs={"data": "some data"},
            config={
                "prompt": "Analyze",
                "output_schema": json.dumps(TRADE_SCHEMA),
                "schema_retries": 1,
                "stream": False,
            },
            context=mock_context,
        )

        assert result["schema_valid"] is False
        assert result["structured_output"] is None
        # 1 initial + 1 retry = 2 calls
        assert mock_context.llm.call.call_count == 2

    @pytest.mark.asyncio
    async def test_no_schema_returns_raw(self, tool, mock_context):
        """Without output_schema, returns raw response with structured_output=None."""
        mock_context.llm.call = AsyncMock(return_value=_resp("Gold is going up", 8))

        result = await tool.execute(
            inputs={"news": "gold news here"},
            config={"prompt": "Summarize", "stream": False},
            context=mock_context,
        )

        assert result["response"] == "Gold is going up"
        assert result["structured_output"] is None
        assert result["schema_valid"] is False

    @pytest.mark.asyncio
    async def test_prompt_enrichment(self, tool, mock_context):
        """When output_schema is set, prompt is enriched with format instructions."""
        valid_json = json.dumps({"action": "HOLD", "confidence": 50, "reasoning": "stable"})
        mock_context.llm.call = AsyncMock(return_value=_resp(valid_json))

        await tool.execute(
            inputs={"data": "test"},
            config={
                "prompt": "Analyze market",
                "output_schema": json.dumps(TRADE_SCHEMA),
                "stream": False,
            },
            context=mock_context,
        )

        # Check that the prompt sent to LLM contains schema instructions
        call_args = mock_context.llm.call.call_args
        sent_prompt = call_args.kwargs.get("prompt", call_args[1].get("prompt", ""))
        assert "OUTPUT FORMAT" in sent_prompt
        assert "MANDATORY" in sent_prompt


class TestEnrichPrompt:
    def test_adds_schema_instructions(self, tool):
        schema = {"type": "object", "properties": {"x": {"type": "number"}}}
        enriched = tool._enrich_prompt_with_schema("Do analysis", schema)
        assert "OUTPUT FORMAT (MANDATORY)" in enriched
        assert '"x"' in enriched
        assert "Do analysis" in enriched


class TestContextTruncation:
    """Tests for max_context_length truncation in _format_session_context."""

    def test_no_truncation_when_under_limit(self, tool):
        data = {"news": "short text"}
        result = tool._format_session_context(data, max_length=5000)
        assert "truncated" not in result
        assert "short text" in result

    def test_no_truncation_when_zero(self, tool):
        """max_length=0 means unlimited."""
        big = "x" * 50000
        result = tool._format_session_context({"data": big}, max_length=0)
        assert len(result) > 50000
        assert "truncated" not in result

    def test_truncates_large_context(self, tool):
        data = {"scrape_data": "A" * 20000}
        result = tool._format_session_context(data, max_length=5000)
        assert len(result) <= 6000  # some overhead for truncation marker
        assert "truncated" in result
        assert "20000" in result  # original size mentioned

    def test_proportional_truncation_multiple_keys(self, tool):
        """Larger values get truncated more than smaller ones."""
        data = {
            "small": "B" * 500,
            "large": "A" * 30000,
        }
        result = tool._format_session_context(data, max_length=5000)
        assert "truncated" in result
        # small value should be preserved (under 200 char minimum)
        assert "B" * 200 in result

    def test_preserves_structure(self, tool):
        data = {"key1": "x" * 10000, "key2": "y" * 10000}
        result = tool._format_session_context(data, max_length=3000)
        assert "=== Session Context ===" in result
        assert "=== End Session Context ===" in result
        assert "[key1]:" in result
        assert "[key2]:" in result

    def test_minimum_200_chars_per_value(self, tool):
        """Each value keeps at least 200 chars even with aggressive truncation."""
        data = {"a": "Z" * 10000, "b": "W" * 10000}
        result = tool._format_session_context(data, max_length=500)
        # Both should have at least 200 chars of their original content
        assert "Z" * 200 in result
        assert "W" * 200 in result

    @pytest.mark.asyncio
    async def test_execute_applies_max_context_length(self, tool, mock_context):
        """Config max_context_length flows through to _format_session_context."""
        mock_context.llm.call = AsyncMock(return_value=_resp("summary"))

        result = await tool.execute(
            inputs={"big_data": "D" * 30000},
            config={"prompt": "Summarize", "stream": False, "max_context_length": 5000},
            context=mock_context,
        )

        # Verify the context sent to LLM was truncated
        call_args = mock_context.llm.call.call_args
        sent_context = call_args.kwargs.get("context", "")
        assert len(sent_context) < 30000
        assert result["response"] == "summary"

    @pytest.mark.asyncio
    async def test_default_max_context_length(self, tool, mock_context):
        """Default 12000 char limit is applied when not configured."""
        mock_context.llm.call = AsyncMock(return_value=_resp("ok"))

        await tool.execute(
            inputs={"huge": "X" * 50000},
            config={"prompt": "Process", "stream": False},
            context=mock_context,
        )

        call_args = mock_context.llm.call.call_args
        sent_context = call_args.kwargs.get("context", "")
        assert len(sent_context) < 15000  # 12000 + overhead


class TestRetryPrompt:
    def test_includes_errors_and_original(self, tool):
        prompt = tool._build_retry_prompt(
            "Original prompt here",
            '{"bad": "response"}',
            ["Missing required field: 'action'"]
        )
        assert "RETRY" in prompt
        assert "Missing required field" in prompt
        assert "Original prompt here" in prompt
