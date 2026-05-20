"""Bridge between LLMAdapter (new) and LLMResource protocol (existing).

AdapterBridge wraps any LLMAdapter and presents it as LLMResource,
so existing tools (llm_call, embeddings) work without changes.
"""

from __future__ import annotations

from typing import Any

from datamirai_engine.llm.adapter import LLMAdapter, NormalizedResponse


class AdapterBridge:
    """Wraps an LLMAdapter to satisfy the LLMResource protocol.

    call() returns NormalizedResponse (dataclass with .response, .tokens_used).
    embed() returns list[float].
    """

    def __init__(self, adapter: LLMAdapter) -> None:
        self._adapter = adapter

    @property
    def adapter(self) -> LLMAdapter:
        return self._adapter

    async def call(
        self,
        *,
        model: str,
        prompt: str,
        context: str | None = None,
        **kwargs: Any,
    ) -> NormalizedResponse:
        return await self._adapter.call(
            model=model,
            prompt=prompt,
            context=context,
            **kwargs,
        )

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        return await self._adapter.embed(text, model=model)
