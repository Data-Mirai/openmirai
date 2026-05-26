"""Tests for all concrete LLM adapters — fully mocked, no real API calls."""

from __future__ import annotations

from unittest.mock import AsyncMock, patch

import httpx
import pytest

from datamirai_engine.llm.adapter import LLMAdapter, NormalizedResponse
from datamirai_engine.llm.adapters.claude import ClaudeAdapter
from datamirai_engine.llm.adapters.gemini import GeminiAdapter
from datamirai_engine.llm.adapters.groq import GroqAdapter
from datamirai_engine.llm.adapters.ollama import OllamaAdapter
from datamirai_engine.llm.adapters.openai_adapter import OpenAIAdapter
from datamirai_engine.llm.adapters.openrouter import OpenRouterAdapter
from datamirai_engine.llm.registry import LLMAdapterFactory, LLMAdapterRegistry


# ── Helpers ──────────────────────────────────────────────────────────────


def _mock_response(status_code: int = 200, json_data: dict | None = None) -> httpx.Response:
    """Build a fake httpx.Response with JSON body."""
    return httpx.Response(
        status_code=status_code,
        json=json_data or {},
        request=httpx.Request("POST", "http://fake"),
    )


# ═══════════════════════════════════════════════════════════════════════
# OllamaAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestOllamaAdapter:
    def _adapter(self, **kw) -> OllamaAdapter:
        return OllamaAdapter(**kw)

    @pytest.mark.asyncio
    async def test_call_success(self, monkeypatch):
        resp_json = {
            "message": {"role": "assistant", "content": "Hello world"},
            "model": "llama3.2",
            "prompt_eval_count": 10,
            "eval_count": 5,
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        adapter = self._adapter()
        result = await adapter.call(model="llama3.2", prompt="hi")

        assert isinstance(result, NormalizedResponse)
        assert result.response == "Hello world"
        assert result.model == "llama3.2"
        assert result.provider == "ollama"
        assert result.tokens_used == {"input": 10, "output": 5}

    @pytest.mark.asyncio
    async def test_call_strips_think_tags(self, monkeypatch):
        resp_json = {
            "message": {"content": "<think>reasoning here</think>The answer is 42"},
            "model": "llama3.2",
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="llama3.2", prompt="q")
        assert "<think>" not in result.response
        assert result.response == "The answer is 42"

    @pytest.mark.asyncio
    async def test_call_strips_nested_think(self, monkeypatch):
        resp_json = {
            "message": {"content": "<think>outer<think>inner</think>more</think>Final"},
            "model": "llama3.2",
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="llama3.2", prompt="q")
        assert "<think>" not in result.response
        assert "Final" in result.response

    @pytest.mark.asyncio
    async def test_call_strips_unclosed_think(self, monkeypatch):
        resp_json = {
            "message": {"content": "<think>reasoning without close tag"},
            "model": "llama3.2",
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="llama3.2", prompt="q")
        assert "<think>" not in result.response
        assert result.response == ""

    @pytest.mark.asyncio
    async def test_embed_success(self, monkeypatch):
        resp_json = {"embedding": [0.1, 0.2, 0.3]}
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        embedding = await self._adapter().embed("hello")
        assert embedding == [0.1, 0.2, 0.3]

    @pytest.mark.asyncio
    async def test_list_models(self, monkeypatch):
        resp_json = {
            "models": [
                {"name": "llama3.2", "details": {"parameter_size": "8B"}},
                {"name": "codellama", "details": {}},
            ]
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "get", mock)

        models = await self._adapter().list_models()
        assert len(models) == 2
        assert models[0].id == "llama3.2"
        assert models[1].id == "codellama"

    @pytest.mark.asyncio
    async def test_connection_error(self, monkeypatch):
        mock = AsyncMock(side_effect=httpx.ConnectError("Connection refused"))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        adapter = self._adapter()
        result = await adapter.test_connection()
        assert result.success is False
        assert "Connection refused" in result.error


# ═══════════════════════════════════════════════════════════════════════
# OpenAIAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestOpenAIAdapter:
    def _adapter(self, **kw) -> OpenAIAdapter:
        return OpenAIAdapter(api_key="sk-test-key", **kw)

    @pytest.mark.asyncio
    async def test_call_success(self, monkeypatch):
        resp_json = {
            "choices": [{"message": {"content": "Hello from GPT"}}],
            "model": "gpt-4o",
            "usage": {"prompt_tokens": 12, "completion_tokens": 8},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="gpt-4o", prompt="hi")
        assert result.response == "Hello from GPT"
        assert result.model == "gpt-4o"
        assert result.provider == "openai"

    @pytest.mark.asyncio
    async def test_call_with_usage(self, monkeypatch):
        resp_json = {
            "choices": [{"message": {"content": "ok"}}],
            "model": "gpt-4o",
            "usage": {"prompt_tokens": 50, "completion_tokens": 25},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="gpt-4o", prompt="test")
        assert result.tokens_used["input"] == 50
        assert result.tokens_used["output"] == 25

    @pytest.mark.asyncio
    async def test_embed_success(self, monkeypatch):
        resp_json = {"data": [{"embedding": [0.5, 0.6, 0.7]}]}
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        embedding = await self._adapter().embed("hello")
        assert embedding == [0.5, 0.6, 0.7]

    @pytest.mark.asyncio
    async def test_list_models(self, monkeypatch):
        resp_json = {
            "data": [
                {"id": "gpt-4o"},
                {"id": "gpt-4o-mini"},
            ]
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "get", mock)

        models = await self._adapter().list_models()
        assert len(models) == 2
        assert models[0].id == "gpt-4o"
        assert models[1].id == "gpt-4o-mini"

    def test_requires_api_key(self):
        with pytest.raises(TypeError):
            OpenAIAdapter()  # type: ignore[call-arg]

    def test_headers_include_bearer(self):
        adapter = self._adapter()
        headers = adapter._headers()
        assert headers["Authorization"] == "Bearer sk-test-key"
        assert headers["Content-Type"] == "application/json"


# ═══════════════════════════════════════════════════════════════════════
# ClaudeAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestClaudeAdapter:
    def _adapter(self, **kw) -> ClaudeAdapter:
        return ClaudeAdapter(api_key="sk-ant-test", **kw)

    @pytest.mark.asyncio
    async def test_call_success(self, monkeypatch):
        resp_json = {
            "content": [{"type": "text", "text": "Hello from Claude"}],
            "model": "claude-sonnet-4-20250514",
            "usage": {"input_tokens": 15, "output_tokens": 10},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="claude-sonnet-4-20250514", prompt="hi")
        assert result.response == "Hello from Claude"
        assert result.model == "claude-sonnet-4-20250514"
        assert result.provider == "claude"

    @pytest.mark.asyncio
    async def test_call_with_usage(self, monkeypatch):
        resp_json = {
            "content": [{"type": "text", "text": "ok"}],
            "model": "claude-sonnet-4-20250514",
            "usage": {"input_tokens": 40, "output_tokens": 20},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="claude-sonnet-4-20250514", prompt="test")
        assert result.tokens_used["input"] == 40
        assert result.tokens_used["output"] == 20

    def test_headers(self):
        adapter = self._adapter()
        headers = adapter._headers()
        assert headers["x-api-key"] == "sk-ant-test"
        assert headers["anthropic-version"] == "2023-06-01"
        assert headers["Content-Type"] == "application/json"

    def test_custom_anthropic_version(self):
        adapter = self._adapter(anthropic_version="2024-01-01")
        headers = adapter._headers()
        assert headers["anthropic-version"] == "2024-01-01"

    @pytest.mark.asyncio
    async def test_embed_not_supported(self):
        adapter = self._adapter()
        with pytest.raises(NotImplementedError, match="claude"):
            await adapter.embed("text")


# ═══════════════════════════════════════════════════════════════════════
# GeminiAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestGeminiAdapter:
    def _adapter(self, **kw) -> GeminiAdapter:
        return GeminiAdapter(api_key="AIza-test-key", **kw)

    @pytest.mark.asyncio
    async def test_call_success(self, monkeypatch):
        resp_json = {
            "candidates": [
                {"content": {"parts": [{"text": "Hello from Gemini"}]}}
            ],
            "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 6},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="gemini-2.0-flash", prompt="hi")
        assert result.response == "Hello from Gemini"
        assert result.provider == "gemini"
        assert result.tokens_used == {"input": 8, "output": 6}

    @pytest.mark.asyncio
    async def test_embed_success(self, monkeypatch):
        resp_json = {"embedding": {"values": [0.11, 0.22, 0.33]}}
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        embedding = await self._adapter().embed("hello")
        assert embedding == [0.11, 0.22, 0.33]

    @pytest.mark.asyncio
    async def test_api_key_as_param(self, monkeypatch):
        """Gemini sends api_key as query parameter, not as header."""
        resp_json = {
            "candidates": [
                {"content": {"parts": [{"text": "ok"}]}}
            ],
        }
        captured_url = None

        async def capture_post(self_client, url, **kwargs):
            nonlocal captured_url
            captured_url = str(url)
            return _mock_response(200, resp_json)

        monkeypatch.setattr(httpx.AsyncClient, "post", capture_post)

        await self._adapter().call(model="gemini-2.0-flash", prompt="hi")
        assert captured_url is not None
        assert "key=AIza-test-key" in captured_url

    @pytest.mark.asyncio
    async def test_list_models(self, monkeypatch):
        resp_json = {
            "models": [
                {"name": "models/gemini-2.0-flash", "displayName": "Gemini 2.0 Flash", "inputTokenLimit": 1000000},
                {"name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro"},
            ]
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "get", mock)

        models = await self._adapter().list_models()
        assert len(models) == 2
        assert models[0].name == "Gemini 2.0 Flash"
        assert models[0].context_window == 1000000


# ═══════════════════════════════════════════════════════════════════════
# GroqAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestGroqAdapter:
    def _adapter(self, **kw) -> GroqAdapter:
        return GroqAdapter(api_key="gsk-test-key", **kw)

    @pytest.mark.asyncio
    async def test_inherits_openai_format(self, monkeypatch):
        """Groq uses OpenAI-compatible response format."""
        resp_json = {
            "choices": [{"message": {"content": "Hello from Groq"}}],
            "model": "llama-3.3-70b-versatile",
            "usage": {"prompt_tokens": 10, "completion_tokens": 7},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="llama-3.3-70b-versatile", prompt="hi")
        assert result.response == "Hello from Groq"
        assert result.provider == "groq"
        assert result.tokens_used["input"] == 10

    def test_custom_base_url(self):
        adapter = self._adapter()
        assert "api.groq.com" in adapter.base_url

    def test_is_subclass_of_openai(self):
        assert issubclass(GroqAdapter, OpenAIAdapter)

    @pytest.mark.asyncio
    async def test_embed_not_supported(self):
        adapter = self._adapter()
        with pytest.raises(NotImplementedError, match="groq"):
            await adapter.embed("text")


# ═══════════════════════════════════════════════════════════════════════
# OpenRouterAdapter
# ═══════════════════════════════════════════════════════════════════════


class TestOpenRouterAdapter:
    def _adapter(self, **kw) -> OpenRouterAdapter:
        return OpenRouterAdapter(api_key="sk-or-test", **kw)

    @pytest.mark.asyncio
    async def test_inherits_openai_format(self, monkeypatch):
        """OpenRouter uses OpenAI-compatible response format."""
        resp_json = {
            "choices": [{"message": {"content": "Hello from OpenRouter"}}],
            "model": "openai/gpt-4o",
            "usage": {"prompt_tokens": 11, "completion_tokens": 9},
        }
        mock = AsyncMock(return_value=_mock_response(200, resp_json))
        monkeypatch.setattr(httpx.AsyncClient, "post", mock)

        result = await self._adapter().call(model="openai/gpt-4o", prompt="hi")
        assert result.response == "Hello from OpenRouter"
        assert result.provider == "openrouter"

    def test_extra_headers(self):
        adapter = self._adapter(site_url="https://example.com", site_name="MyApp")
        headers = adapter._headers()
        assert headers["HTTP-Referer"] == "https://example.com"
        assert headers["X-Title"] == "MyApp"
        assert "Bearer sk-or-test" in headers["Authorization"]

    def test_extra_headers_default_site_name(self):
        adapter = self._adapter()
        headers = adapter._headers()
        assert headers["X-Title"] == "Data Mirai Engine"
        # No site_url set, so HTTP-Referer should not be present
        assert "HTTP-Referer" not in headers

    def test_is_subclass_of_openai(self):
        assert issubclass(OpenRouterAdapter, OpenAIAdapter)

    @pytest.mark.asyncio
    async def test_embed_not_supported(self):
        adapter = self._adapter()
        with pytest.raises(NotImplementedError, match="openrouter"):
            await adapter.embed("text")


# ═══════════════════════════════════════════════════════════════════════
# Registry + Factory
# ═══════════════════════════════════════════════════════════════════════


class TestLLMAdapterRegistry:
    def setup_method(self):
        """Clean registry before each test."""
        LLMAdapterRegistry.clear()

    def teardown_method(self):
        """Clean registry after each test."""
        LLMAdapterRegistry.clear()

    def test_register_and_get(self):
        LLMAdapterRegistry.register("ollama", OllamaAdapter)
        cls = LLMAdapterRegistry.get("ollama")
        assert cls is OllamaAdapter

    def test_get_unknown_raises(self):
        with pytest.raises(KeyError, match="No adapter registered for provider 'unknown'"):
            LLMAdapterRegistry.get("unknown")

    def test_get_unknown_shows_available(self):
        LLMAdapterRegistry.register("ollama", OllamaAdapter)
        with pytest.raises(KeyError, match="Available: ollama"):
            LLMAdapterRegistry.get("nope")

    def test_list_providers(self):
        LLMAdapterRegistry.register("ollama", OllamaAdapter)
        LLMAdapterRegistry.register("openai", OpenAIAdapter)
        providers = LLMAdapterRegistry.list_providers()
        assert providers == ["ollama", "openai"]

    def test_is_registered(self):
        assert LLMAdapterRegistry.is_registered("ollama") is False
        LLMAdapterRegistry.register("ollama", OllamaAdapter)
        assert LLMAdapterRegistry.is_registered("ollama") is True


class TestLLMAdapterFactory:
    def setup_method(self):
        LLMAdapterRegistry.clear()
        LLMAdapterRegistry.register("ollama", OllamaAdapter)
        LLMAdapterRegistry.register("openai", OpenAIAdapter)

    def teardown_method(self):
        LLMAdapterRegistry.clear()

    def test_factory_creates_adapter(self):
        factory = LLMAdapterFactory()
        adapter = factory.get_adapter("ollama")
        assert isinstance(adapter, OllamaAdapter)
        assert isinstance(adapter, LLMAdapter)

    def test_factory_caches(self):
        factory = LLMAdapterFactory()
        a1 = factory.get_adapter("ollama")
        a2 = factory.get_adapter("ollama")
        assert a1 is a2

    def test_factory_different_providers(self):
        factory = LLMAdapterFactory()
        ollama = factory.get_adapter("ollama")
        openai = factory.get_adapter("openai", config={"api_key": "sk-test"})
        assert type(ollama) is not type(openai)

    def test_factory_with_config(self):
        factory = LLMAdapterFactory()
        adapter = factory.get_adapter(
            "openai", config={"api_key": "sk-custom"}
        )
        assert adapter.api_key == "sk-custom"

    def test_factory_embedding_fallback(self):
        factory = LLMAdapterFactory()
        # ollama supports embeddings, try it as fallback
        adapter = factory.get_embedding_adapter(
            preferred_provider="ollama",
            fallback_providers=["openai"],
        )
        assert isinstance(adapter, OllamaAdapter)

    def test_factory_embedding_no_provider_raises(self):
        factory = LLMAdapterFactory()
        with pytest.raises(NotImplementedError, match="No embedding provider configured"):
            factory.get_embedding_adapter()

    def test_factory_embedding_unknown_provider_raises(self):
        factory = LLMAdapterFactory()
        with pytest.raises(NotImplementedError, match="No embedding-capable provider found"):
            factory.get_embedding_adapter(
                preferred_provider="nonexistent",
                fallback_providers=["also_nonexistent"],
            )

    def test_factory_clear_cache(self):
        factory = LLMAdapterFactory()
        a1 = factory.get_adapter("ollama")
        factory.clear_cache()
        a2 = factory.get_adapter("ollama")
        assert a1 is not a2
