"""OpenAI adapter — GPT models via HTTP API."""

from __future__ import annotations

from typing import Any

import httpx

from datamirai_engine.llm.adapter import (
    LLMAdapter,
    ModelInfo,
    NormalizedResponse,
    ToolCall,
)


_CONTEXT_WINDOWS: dict[str, int] = {
    "gpt-4o": 128000,
    "gpt-4o-mini": 128000,
    "gpt-4-turbo": 128000,
    "gpt-4": 8192,
    "gpt-3.5-turbo": 16385,
    "o1": 200000,
    "o1-mini": 128000,
    "o3-mini": 200000,
}


class OpenAIAdapter(LLMAdapter):
    """Adapter for OpenAI's chat completions and embeddings API."""

    provider_name = "openai"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://api.openai.com",
        timeout: float = 60.0,
    ) -> None:
        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _headers(self) -> dict[str, str]:
        return {
            "Authorization": f"Bearer {self.api_key}",
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
        messages: list[dict[str, str]] = []
        if context:
            messages.append({"role": "system", "content": context})
        messages.append({"role": "user", "content": prompt})

        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        }

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(
                f"{self.base_url}/v1/chat/completions",
                headers=self._headers(),
                json=payload,
            )
            resp.raise_for_status()
            data = resp.json()

        choice = data["choices"][0]
        text: str = choice["message"]["content"] or ""

        usage = data.get("usage", {})
        tokens_input = usage.get("prompt_tokens", 0)
        tokens_output = usage.get("completion_tokens", 0)

        return NormalizedResponse(
            response=text,
            tokens_used={"input": tokens_input, "output": tokens_output},
            model=data.get("model", model),
            provider=self.provider_name,
        )

    async def call_with_messages(
        self,
        *,
        model: str,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        temperature: float = 0.7,
        max_tokens: int = 4096,
        **kwargs: Any,
    ) -> NormalizedResponse:
        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        }
        if tools:
            payload["tools"] = tools
            payload["tool_choice"] = kwargs.get("tool_choice", "auto")

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(
                f"{self.base_url}/v1/chat/completions",
                headers=self._headers(),
                json=payload,
            )
            resp.raise_for_status()
            data = resp.json()

        choice = data["choices"][0]
        message = choice["message"]
        text: str = message.get("content") or ""

        tool_calls: list[ToolCall] = []
        for tc in message.get("tool_calls", []):
            tool_calls.append(
                ToolCall(
                    id=tc.get("id", ""),
                    name=tc["function"]["name"],
                    arguments=tc["function"]["arguments"],
                )
            )

        usage = data.get("usage", {})

        return NormalizedResponse(
            response=text,
            tokens_used={
                "input": usage.get("prompt_tokens", 0),
                "output": usage.get("completion_tokens", 0),
            },
            model=data.get("model", model),
            provider=self.provider_name,
            tool_calls=tool_calls,
        )

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        embed_model = model or "text-embedding-3-small"
        payload = {
            "model": embed_model,
            "input": text,
        }

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(
                f"{self.base_url}/v1/embeddings",
                headers=self._headers(),
                json=payload,
            )
            resp.raise_for_status()
            data = resp.json()

        return data["data"][0]["embedding"]

    async def list_models(self) -> list[ModelInfo]:
        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.get(
                f"{self.base_url}/v1/models",
                headers=self._headers(),
            )
            resp.raise_for_status()
            data = resp.json()

        models: list[ModelInfo] = []
        for m in data.get("data", []):
            model_id = m.get("id", "")
            models.append(
                ModelInfo(
                    id=model_id,
                    name=model_id,
                    context_window=_CONTEXT_WINDOWS.get(model_id),
                )
            )
        return models

    def _default_test_model(self) -> str:
        return "gpt-4o-mini"
