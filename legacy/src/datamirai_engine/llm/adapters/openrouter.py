"""OpenRouter adapter — multi-provider routing via OpenAI-compatible API."""

from __future__ import annotations

from typing import Any

from datamirai_engine.llm.adapter import ModelInfo, NormalizedResponse
from datamirai_engine.llm.adapters.openai_adapter import OpenAIAdapter

_CONTEXT_WINDOWS: dict[str, int] = {
    "anthropic/claude-sonnet-4": 200000,
    "openai/gpt-4o": 128000,
    "google/gemini-2.5-flash": 1048576,
    "meta-llama/llama-3.3-70b-instruct": 131072,
}


class OpenRouterAdapter(OpenAIAdapter):
    """Adapter for OpenRouter. Inherits OpenAI format with extra headers."""

    provider_name = "openrouter"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://openrouter.ai/api",
        timeout: float = 60.0,
        site_url: str = "",
        site_name: str = "Data Mirai Engine",
    ) -> None:
        super().__init__(api_key=api_key, base_url=base_url, timeout=timeout)
        self.site_url = site_url
        self.site_name = site_name

    def _headers(self) -> dict[str, str]:
        headers = super()._headers()
        if self.site_url:
            headers["HTTP-Referer"] = self.site_url
        if self.site_name:
            headers["X-Title"] = self.site_name
        return headers

    async def call(
        self,
        *,
        model: str,
        prompt: str,
        context: str | None = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs: Any,
    ) -> NormalizedResponse:
        result = await super().call(
            model=model,
            prompt=prompt,
            context=context,
            temperature=temperature,
            max_tokens=max_tokens,
            **kwargs,
        )
        return NormalizedResponse(
            response=result.response,
            tokens_used=result.tokens_used,
            model=result.model,
            provider=self.provider_name,
        )

    async def list_models(self) -> list[ModelInfo]:
        models = await super().list_models()
        return [
            ModelInfo(
                id=m.id,
                name=m.name,
                context_window=_CONTEXT_WINDOWS.get(m.id) or m.context_window,
                supports_streaming=m.supports_streaming,
                supports_tools=m.supports_tools,
            )
            for m in models
        ]

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        raise NotImplementedError(
            f"{self.provider_name} adapter does not support embeddings. "
            "Configure a provider with embedding support as fallback."
        )

    def _default_test_model(self) -> str:
        return "openai/gpt-4o-mini"
