"""LLM Adapter — abstract interface and normalized data models.

Every LLM provider adapter returns NormalizedResponse. The engine never sees
raw provider-specific formats.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any, AsyncIterator


@dataclass(frozen=True)
class ToolCall:
    """A tool/function call requested by the LLM."""

    id: str
    name: str
    arguments: str  # JSON string of arguments


@dataclass(frozen=True)
class NormalizedResponse:
    """Unified response from any LLM provider."""

    response: str
    tokens_used: dict[str, int] = field(default_factory=lambda: {"input": 0, "output": 0})
    model: str = ""
    provider: str = ""
    tool_calls: list[ToolCall] = field(default_factory=list)


@dataclass(frozen=True)
class NormalizedChunk:
    """Single chunk from a streaming response."""

    delta: str
    done: bool = False
    tokens_used: dict[str, int] | None = None


@dataclass(frozen=True)
class ModelInfo:
    """Metadata about a model available in a provider."""

    id: str
    name: str
    context_window: int | None = None
    supports_streaming: bool = True
    supports_tools: bool = False


@dataclass(frozen=True)
class ConnectionTestResult:
    """Result of a provider connection test."""

    success: bool
    latency_ms: int = 0
    error: str | None = None
    model_used: str | None = None


class LLMAdapter(ABC):
    """Abstract base class for LLM provider adapters.

    Each concrete adapter normalizes input/output for a specific provider.
    The engine only interacts via this interface.
    """

    provider_name: str = ""

    @abstractmethod
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
        """Send prompt to LLM and return normalized response."""

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
        """Send a full conversation (messages array) with optional tool definitions.

        This is the method used by the agentic loop. Unlike ``call()``, it
        accepts a complete messages array and tool schemas, and returns
        ``NormalizedResponse`` with ``tool_calls`` populated when the LLM
        decides to invoke a tool.
        """
        raise NotImplementedError(
            f"{self.provider_name} adapter does not implement call_with_messages. "
            "Override this method to enable agentic tool-calling."
        )

    async def stream(
        self,
        *,
        model: str,
        prompt: str,
        context: str | None = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs: Any,
    ) -> AsyncIterator[NormalizedChunk]:
        """Stream response from LLM. Default falls back to non-streaming call."""
        result = await self.call(
            model=model,
            prompt=prompt,
            context=context,
            temperature=temperature,
            max_tokens=max_tokens,
            **kwargs,
        )
        yield NormalizedChunk(delta=result.response, done=True, tokens_used=result.tokens_used)

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        """Generate embedding vector. Not all providers support this."""
        raise NotImplementedError(
            f"{self.provider_name} adapter does not support embeddings. "
            "Configure a provider with embedding support as fallback."
        )

    async def list_models(self) -> list[ModelInfo]:
        """List models available from this provider."""
        return []

    async def test_connection(self) -> ConnectionTestResult:
        """Verify provider connectivity with a minimal request."""
        import time

        start = time.monotonic()
        try:
            result = await self.call(
                model=self._default_test_model(),
                prompt="Reply with exactly: ok",
                max_tokens=10,
                temperature=0.0,
            )
            elapsed = int((time.monotonic() - start) * 1000)
            return ConnectionTestResult(
                success=True,
                latency_ms=elapsed,
                model_used=result.model,
            )
        except Exception as exc:
            elapsed = int((time.monotonic() - start) * 1000)
            return ConnectionTestResult(
                success=False,
                latency_ms=elapsed,
                error=str(exc),
            )

    def _default_test_model(self) -> str:
        """Model to use for connection tests. Override per adapter."""
        return ""
