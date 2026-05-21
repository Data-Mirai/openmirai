"""NVIDIA NIM adapter — 100+ models via OpenAI-compatible API."""

from __future__ import annotations

from typing import Any

from datamirai_engine.llm.adapter import ModelInfo, NormalizedResponse
from datamirai_engine.llm.adapters.openai_adapter import OpenAIAdapter

# Baseline models — fallback when API catalog is unavailable.
_BASELINE_MODELS: list[dict[str, Any]] = [
    {"id": "moonshotai/kimi-k2-instruct", "name": "Kimi K2 Instruct", "context_window": 128000},
    {"id": "moonshotai/kimi-k2.5", "name": "Kimi K2.5", "context_window": 200000},
    {"id": "minimaxai/minimax-m2.7", "name": "MiniMax M2.7", "context_window": 204800},
    {"id": "deepseek-ai/deepseek-v4-flash", "name": "DeepSeek V4 Flash", "context_window": 1048576},
    {"id": "meta/llama-3.3-70b-instruct", "name": "Llama 3.3 70B", "context_window": 131072},
    {"id": "meta/llama-4-maverick-17b-128e-instruct", "name": "Llama 4 Maverick", "context_window": 131072},
    {"id": "qwen/qwen3-235b-a22b", "name": "Qwen 3 235B", "context_window": 131072},
    {"id": "mistralai/mistral-large-2-instruct", "name": "Mistral Large 2", "context_window": 131072},
    {"id": "nvidia/llama-3.1-nemotron-ultra-253b-v1", "name": "Nemotron Ultra 253B", "context_window": 131072},
    {"id": "google/gemma-3-27b-it", "name": "Gemma 3 27B", "context_window": 131072},
]

_CONTEXT_WINDOWS: dict[str, int] = {m["id"]: m["context_window"] for m in _BASELINE_MODELS}


class NvidiaAdapter(OpenAIAdapter):
    """Adapter for NVIDIA NIM. Inherits OpenAI format with NIM base_url."""

    provider_name = "nvidia_nim"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://integrate.api.nvidia.com/v1",
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
        # Try dynamic catalog from NIM API first
        try:
            models = await super().list_models()
            if models:
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
        except Exception:
            pass

        # Fallback: baseline hardcoded models
        return [
            ModelInfo(
                id=m["id"],
                name=m["name"],
                context_window=m["context_window"],
                supports_streaming=True,
                supports_tools=True,
            )
            for m in _BASELINE_MODELS
        ]

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        embed_model = model or "nvidia/nv-embedqa-e5-v5"
        return await super().embed(text, model=embed_model)

    def _default_test_model(self) -> str:
        return "moonshotai/kimi-k2-instruct"
