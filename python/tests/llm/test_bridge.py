"""Tests for AdapterBridge — LLMAdapter to LLMResource protocol translation."""

import pytest

from datamirai_engine.llm.adapter import LLMAdapter, NormalizedResponse
from datamirai_engine.llm.bridge import AdapterBridge


class FakeAdapter(LLMAdapter):
    provider_name = "fake"

    async def call(self, *, model, prompt, context=None, temperature=0.7, max_tokens=1024, **kw):
        return NormalizedResponse(
            response=f"fake: {prompt}",
            tokens_used={"input": 10, "output": 20},
            model=model,
            provider="fake",
        )

    async def embed(self, text, *, model=None):
        return [0.1, 0.2, 0.3]


class FakeNoEmbedAdapter(LLMAdapter):
    provider_name = "no_embed"

    async def call(self, *, model, prompt, context=None, temperature=0.7, max_tokens=1024, **kw):
        return NormalizedResponse(response="ok", model=model, provider="no_embed")


class TestAdapterBridge:
    @pytest.mark.asyncio
    async def test_call_returns_normalized_response(self):
        bridge = AdapterBridge(FakeAdapter())
        result = await bridge.call(model="m", prompt="hello")
        assert isinstance(result, NormalizedResponse)
        assert result.response == "fake: hello"
        assert result.model == "m"
        assert result.provider == "fake"

    @pytest.mark.asyncio
    async def test_call_includes_token_counts(self):
        bridge = AdapterBridge(FakeAdapter())
        result = await bridge.call(model="m", prompt="hi")
        assert result.tokens_used["input"] == 10
        assert result.tokens_used["output"] == 20

    @pytest.mark.asyncio
    async def test_call_passes_kwargs(self):
        received = {}

        class SpyAdapter(LLMAdapter):
            provider_name = "spy"

            async def call(self, *, model, prompt, context=None, temperature=0.7, max_tokens=1024, **kw):
                received.update(model=model, prompt=prompt, context=context, temperature=temperature, max_tokens=max_tokens)
                return NormalizedResponse(response="ok", model=model, provider="spy")

        bridge = AdapterBridge(SpyAdapter())
        await bridge.call(model="gpt", prompt="test", context="ctx", temperature=0.5, max_tokens=512)
        assert received["model"] == "gpt"
        assert received["context"] == "ctx"
        assert received["temperature"] == 0.5
        assert received["max_tokens"] == 512

    @pytest.mark.asyncio
    async def test_embed_delegates(self):
        bridge = AdapterBridge(FakeAdapter())
        vec = await bridge.embed("text")
        assert vec == [0.1, 0.2, 0.3]

    @pytest.mark.asyncio
    async def test_embed_not_supported_propagates(self):
        bridge = AdapterBridge(FakeNoEmbedAdapter())
        with pytest.raises(NotImplementedError, match="no_embed"):
            await bridge.embed("text")

    def test_adapter_property(self):
        adapter = FakeAdapter()
        bridge = AdapterBridge(adapter)
        assert bridge.adapter is adapter

    @pytest.mark.asyncio
    async def test_bridge_satisfies_llm_resource_protocol(self):
        """AdapterBridge duck-types as LLMResource."""
        from datamirai_engine.core.context import LLMResource

        bridge = AdapterBridge(FakeAdapter())
        assert isinstance(bridge, LLMResource)


class TestBridgeWithTools:
    """Integration: verify existing tools work when LLM is an AdapterBridge."""

    @pytest.mark.asyncio
    async def test_llm_call_tool_with_bridge(self):
        from datamirai_engine.resources.context import SimpleExecutionContext
        from datamirai_engine.tools.builtin.ai.llm_call import LLMCallTool

        bridge = AdapterBridge(FakeAdapter())
        ctx = SimpleExecutionContext(
            db=SimpleExecutionContext.default().db,
            vector=SimpleExecutionContext.default().vector,
            storage=SimpleExecutionContext.default().storage,
            llm=bridge,
            auth=SimpleExecutionContext.default().auth,
        )
        tool = LLMCallTool()
        result = await tool.run(
            {"prompt": "summarize", "data": "some data here"},
            {"model": "test-model"},
            ctx,
        )
        assert "response" in result
        assert "fake: summarize" in result["response"]
        assert result["tokens_used"] == 20  # output tokens from FakeAdapter

    @pytest.mark.asyncio
    async def test_embeddings_tool_with_bridge(self):
        from datamirai_engine.resources.context import SimpleExecutionContext
        from datamirai_engine.tools.builtin.ai.embeddings import EmbeddingsTool

        bridge = AdapterBridge(FakeAdapter())
        ctx = SimpleExecutionContext(
            db=SimpleExecutionContext.default().db,
            vector=SimpleExecutionContext.default().vector,
            storage=SimpleExecutionContext.default().storage,
            llm=bridge,
            auth=SimpleExecutionContext.default().auth,
        )
        tool = EmbeddingsTool()
        result = await tool.run({"text": "hello"}, {}, ctx)
        assert result["embedding"] == [0.1, 0.2, 0.3]
        assert result["dimension"] == 3
