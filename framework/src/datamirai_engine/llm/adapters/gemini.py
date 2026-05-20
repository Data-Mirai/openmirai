"""Gemini adapter — Google AI models via HTTP API."""

from __future__ import annotations

from typing import Any

import httpx

from datamirai_engine.llm.adapter import (
    LLMAdapter,
    ModelInfo,
    NormalizedResponse,
)


_CONTEXT_WINDOWS: dict[str, int] = {
    "gemini-2.5-pro": 1048576,
    "gemini-2.5-flash": 1048576,
    "gemini-2.0-flash": 1048576,
    "gemini-1.5-pro": 2097152,
    "gemini-1.5-flash": 1048576,
}


class GeminiAdapter(LLMAdapter):
    """Adapter for Google Gemini models. API key sent as query param."""

    provider_name = "gemini"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://generativelanguage.googleapis.com",
        timeout: float = 60.0,
    ) -> None:
        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # ------------------------------------------------------------------
    # Core interface
    # ------------------------------------------------------------------

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
        contents: list[dict[str, Any]] = []
        if context:
            contents.append({"role": "user", "parts": [{"text": context}]})
            contents.append({"role": "model", "parts": [{"text": "Understood."}]})
        contents.append({"role": "user", "parts": [{"text": prompt}]})

        payload: dict[str, Any] = {
            "contents": contents,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": max_tokens,
            },
        }

        url = (
            f"{self.base_url}/v1beta/models/{model}:generateContent"
            f"?key={self.api_key}"
        )

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(url, json=payload)
            resp.raise_for_status()
            data = resp.json()

        text: str = data["candidates"][0]["content"]["parts"][0]["text"]

        usage = data.get("usageMetadata", {})
        tokens_input = usage.get("promptTokenCount", 0)
        tokens_output = usage.get("candidatesTokenCount", 0)

        return NormalizedResponse(
            response=text,
            tokens_used={"input": tokens_input, "output": tokens_output},
            model=model,
            provider=self.provider_name,
        )

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        embed_model = model or "text-embedding-004"
        url = (
            f"{self.base_url}/v1beta/models/{embed_model}:embedContent"
            f"?key={self.api_key}"
        )
        payload = {"content": {"parts": [{"text": text}]}}

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(url, json=payload)
            resp.raise_for_status()
            data = resp.json()

        return data["embedding"]["values"]

    async def list_models(self) -> list[ModelInfo]:
        url = f"{self.base_url}/v1beta/models?key={self.api_key}"

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.get(url)
            resp.raise_for_status()
            data = resp.json()

        models: list[ModelInfo] = []
        for m in data.get("models", []):
            model_id = m.get("name", "")
            # Prefer API-reported value; fall back to static table
            cw = m.get("inputTokenLimit") or _CONTEXT_WINDOWS.get(model_id)
            models.append(
                ModelInfo(
                    id=model_id,
                    name=m.get("displayName", ""),
                    context_window=cw,
                )
            )
        return models

    def _default_test_model(self) -> str:
        return "gemini-2.0-flash"
