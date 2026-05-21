"""Tests for LLM Adapter ABC and data models."""

import pytest

from datamirai_engine.llm.adapter import (
    ConnectionTestResult,
    LLMAdapter,
    ModelInfo,
    NormalizedChunk,
    NormalizedResponse,
)


# ── Data models ─────────────────────────────────────────────────────────


class TestNormalizedResponse:
    def test_defaults(self):
        r = NormalizedResponse(response="hello")
        assert r.response == "hello"
        assert r.tokens_used == {"input": 0, "output": 0}
        assert r.model == ""
        assert r.provider == ""

    def test_with_values(self):
        r = NormalizedResponse(
            response="hi",
            tokens_used={"input": 10, "output": 5},
            model="gpt-4",
            provider="openai",
        )
        assert r.tokens_used["input"] == 10
        assert r.provider == "openai"

    def test_frozen(self):
        r = NormalizedResponse(response="x")
        with pytest.raises(AttributeError):
            r.response = "y"


class TestNormalizedChunk:
    def test_defaults(self):
        c = NormalizedChunk(delta="tok")
        assert c.delta == "tok"
        assert c.done is False
        assert c.tokens_used is None

    def test_done(self):
        c = NormalizedChunk(delta="", done=True, tokens_used={"input": 5, "output": 3})
        assert c.done is True
        assert c.tokens_used["output"] == 3


class TestModelInfo:
    def test_defaults(self):
        m = ModelInfo(id="gpt-4", name="GPT-4")
        assert m.context_window is None
        assert m.supports_streaming is True

    def test_full(self):
        m = ModelInfo(id="x", name="X", context_window=128000, supports_tools=True)
        assert m.context_window == 128000
        assert m.supports_tools is True


class TestConnectionTestResult:
    def test_success(self):
        r = ConnectionTestResult(success=True, latency_ms=120, model_used="gpt-4")
        assert r.success is True
        assert r.error is None

    def test_failure(self):
        r = ConnectionTestResult(success=False, error="timeout")
        assert r.success is False
        assert r.error == "timeout"


# ── Adapter ABC ─────────────────────────────────────────────────────────


class DummyAdapter(LLMAdapter):
    """Concrete adapter for testing the ABC."""

    provider_name = "dummy"

    def __init__(self, *, fail: bool = False):
        self._fail = fail

    async def call(self, *, model, prompt, context=None, temperature=0.7, max_tokens=1024, **kw):
        if self._fail:
            raise RuntimeError("boom")
        return NormalizedResponse(
            response=f"echo: {prompt}",
            tokens_used={"input": len(prompt), "output": 5},
            model=model,
            provider="dummy",
        )

    def _default_test_model(self):
        return "dummy-model"


class TestLLMAdapterABC:
    def test_cannot_instantiate_abc(self):
        with pytest.raises(TypeError):
            LLMAdapter()

    @pytest.mark.asyncio
    async def test_call(self):
        adapter = DummyAdapter()
        r = await adapter.call(model="m", prompt="hi")
        assert r.response == "echo: hi"
        assert r.provider == "dummy"

    @pytest.mark.asyncio
    async def test_stream_fallback(self):
        adapter = DummyAdapter()
        chunks = []
        async for chunk in adapter.stream(model="m", prompt="hi"):
            chunks.append(chunk)
        assert len(chunks) == 1
        assert chunks[0].done is True
        assert "echo: hi" in chunks[0].delta

    @pytest.mark.asyncio
    async def test_embed_not_implemented(self):
        adapter = DummyAdapter()
        with pytest.raises(NotImplementedError, match="dummy"):
            await adapter.embed("text")

    @pytest.mark.asyncio
    async def test_list_models_default_empty(self):
        adapter = DummyAdapter()
        models = await adapter.list_models()
        assert models == []

    @pytest.mark.asyncio
    async def test_test_connection_success(self):
        adapter = DummyAdapter()
        result = await adapter.test_connection()
        assert result.success is True
        assert result.latency_ms >= 0
        assert result.model_used == "dummy-model"

    @pytest.mark.asyncio
    async def test_test_connection_failure(self):
        adapter = DummyAdapter(fail=True)
        result = await adapter.test_connection()
        assert result.success is False
        assert "boom" in result.error
