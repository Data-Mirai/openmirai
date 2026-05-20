"""Claude adapter — Anthropic models via HTTP API."""

from __future__ import annotations

from typing import Any

import httpx

from datamirai_engine.llm.adapter import (
    LLMAdapter,
    ModelInfo,
    NormalizedResponse,
)

_CONTEXT_WINDOWS: dict[str, int] = {
    "claude-opus-4-20250514": 200000,
    "claude-sonnet-4-20250514": 200000,
    "claude-haiku-4-5-20251001": 200000,
    "claude-3-5-sonnet-20241022": 200000,
    "claude-3-5-haiku-20241022": 200000,
}


class ClaudeAdapter(LLMAdapter):
    """Adapter for Anthropic's Claude models."""

    provider_name = "claude"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://api.anthropic.com",
        timeout: float = 60.0,
        anthropic_version: str = "2023-06-01",
    ) -> None:
        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self.anthropic_version = anthropic_version

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _headers(self) -> dict[str, str]:
        return {
            "x-api-key": self.api_key,
            "anthropic-version": self.anthropic_version,
            "Content-Type": "application/json",
        }

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
        payload: dict[str, Any] = {
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "messages": [{"role": "user", "content": prompt}],
        }
        if context:
            payload["system"] = context

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(
                f"{self.base_url}/v1/messages",
                headers=self._headers(),
                json=payload,
            )
            resp.raise_for_status()
            data = resp.json()

        text: str = data["content"][0]["text"]

        usage = data.get("usage", {})
        tokens_input = usage.get("input_tokens", 0)
        tokens_output = usage.get("output_tokens", 0)

        return NormalizedResponse(
            response=text,
            tokens_used={"input": tokens_input, "output": tokens_output},
            model=data.get("model", model),
            provider=self.provider_name,
        )

    async def list_models(self) -> list[ModelInfo]:
        return [
            ModelInfo(id=mid, name=mid, context_window=cw)
            for mid, cw in _CONTEXT_WINDOWS.items()
        ]

    def _default_test_model(self) -> str:
        return "claude-sonnet-4-20250514"
