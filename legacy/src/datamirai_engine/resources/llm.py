"""LLM Resource implementations — Mock for dev/testing."""

from __future__ import annotations

import hashlib
from typing import Any

from datamirai_engine.llm.adapter import NormalizedResponse


class MockLLMResource:
    """Mock LLM that returns configurable responses. For dev/testing.

    Can be initialized with a response map: {prompt: NormalizedResponse or dict}.
    Falls back to a generic response if prompt not in map.
    Embeddings are deterministic hashes of the input text.
    """

    def __init__(
        self,
        responses: dict[str, dict[str, Any]] | None = None,
        embedding_dim: int = 8,
    ) -> None:
        self._responses = responses or {}
        self._embedding_dim = embedding_dim

    async def call(
        self,
        *,
        model: str,
        prompt: str,
        context: str | None = None,
        **kwargs: Any,
    ) -> NormalizedResponse:
        if prompt in self._responses:
            resp = self._responses[prompt]
            if isinstance(resp, NormalizedResponse):
                return resp
            # Legacy dict format — convert
            return NormalizedResponse(
                response=resp.get("text", ""),
                tokens_used={"input": 0, "output": resp.get("tokens", 0)},
                model=resp.get("model", model),
            )
        token_count = len(prompt.split())
        return NormalizedResponse(
            response=f"[mock response to: {prompt[:50]}]",
            tokens_used={"input": 0, "output": token_count},
            model=model,
        )

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        h = hashlib.sha256(text.encode()).digest()
        return [
            (b / 255.0) * 2 - 1  # normalize to [-1, 1]
            for b in h[: self._embedding_dim]
        ]
