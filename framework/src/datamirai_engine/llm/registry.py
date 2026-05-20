"""LLM Adapter Registry and Factory.

Registry maps provider names to adapter classes.
Factory instantiates adapters with concrete config and caches them.
"""

from __future__ import annotations

from typing import Any

from datamirai_engine.llm.adapter import LLMAdapter


class LLMAdapterRegistry:
    """Maps provider names to adapter classes. Singleton-style."""

    _adapters: dict[str, type[LLMAdapter]] = {}

    @classmethod
    def register(cls, provider_name: str, adapter_class: type[LLMAdapter]) -> None:
        cls._adapters[provider_name] = adapter_class

    @classmethod
    def get(cls, provider_name: str) -> type[LLMAdapter]:
        if provider_name not in cls._adapters:
            available = ", ".join(sorted(cls._adapters.keys())) or "(none)"
            raise KeyError(
                f"No adapter registered for provider '{provider_name}'. "
                f"Available: {available}"
            )
        return cls._adapters[provider_name]

    @classmethod
    def list_providers(cls) -> list[str]:
        return sorted(cls._adapters.keys())

    @classmethod
    def is_registered(cls, provider_name: str) -> bool:
        return provider_name in cls._adapters

    @classmethod
    def clear(cls) -> None:
        """Clear all registrations. For testing only."""
        cls._adapters.clear()


class LLMAdapterFactory:
    """Creates and caches adapter instances per provider config."""

    def __init__(self) -> None:
        self._cache: dict[str, LLMAdapter] = {}

    def get_adapter(
        self,
        provider_name: str,
        *,
        config: dict[str, Any] | None = None,
        cache_key: str | None = None,
    ) -> LLMAdapter:
        """Get or create adapter instance.

        Args:
            provider_name: registered provider name (e.g. 'ollama', 'openai')
            config: provider-specific config (api_key, base_url, etc.)
            cache_key: unique key for caching. Defaults to provider_name.
        """
        key = cache_key or provider_name
        if key not in self._cache:
            adapter_class = LLMAdapterRegistry.get(provider_name)
            self._cache[key] = adapter_class(**(config or {}))
        return self._cache[key]

    def get_embedding_adapter(
        self,
        preferred_provider: str | None = None,
        *,
        config: dict[str, Any] | None = None,
        fallback_providers: list[str] | None = None,
    ) -> LLMAdapter:
        """Get an adapter that supports embeddings, with fallback chain.

        Tries preferred_provider first, then each fallback in order.
        Raises NotImplementedError if none support embeddings.
        """
        providers_to_try = []
        if preferred_provider:
            providers_to_try.append(preferred_provider)
        if fallback_providers:
            providers_to_try.extend(
                p for p in fallback_providers if p != preferred_provider
            )

        for provider in providers_to_try:
            if LLMAdapterRegistry.is_registered(provider):
                return self.get_adapter(provider, config=config)

        if providers_to_try:
            raise NotImplementedError(
                f"No embedding-capable provider found. Tried: {providers_to_try}"
            )
        raise NotImplementedError("No embedding provider configured.")

    def clear_cache(self) -> None:
        self._cache.clear()
