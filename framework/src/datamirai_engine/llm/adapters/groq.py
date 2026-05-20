"""Groq adapter — fast inference via OpenAI-compatible API."""

from __future__ import annotations

from typing import Any

from datamirai_engine.llm.adapter import ModelInfo, NormalizedResponse
from datamirai_engine.llm.adapters.openai_adapter import OpenAIAdapter

_CONTEXT_WINDOWS: dict[str, int] = {
    "llama-3.3-70b-versatile": 128000,
    "llama-3.1-8b-instant": 131072,
    "mixtral-8x7b-32768": 32768,
    "gemma2-9b-it": 8192,
}


class GroqAdapter(OpenAIAdapter):
    """Adapter for Groq. Inherits OpenAI format with a different base_url."""

    provider_name = "groq"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://api.groq.com/openai",
        timeout: float = 60.0,
    ) -> None:
        super().__init__(api_key=api_key, base_url=base_url, timeout=timeout)

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
        return "llama-3.3-70b-versatile"
