"""ExecutionContext — Resource access abstraction for block execution."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Any, Protocol, runtime_checkable


@dataclass(frozen=True)
class AuthContext:
    """Caller identity and permissions. Provided by external auth provider."""

    user_id: str
    role: str  # OWNER | ADMIN | EDITOR | VIEWER
    universe_id: str | None = None
    environment_id: str | None = None


@runtime_checkable
class DBResource(Protocol):
    """Relational database access (PostgreSQL)."""

    async def execute(self, query: str, params: dict[str, Any] | None = None) -> Any: ...
    async def fetch_one(self, query: str, params: dict[str, Any] | None = None) -> dict | None: ...
    async def fetch_all(self, query: str, params: dict[str, Any] | None = None) -> list[dict]: ...


@runtime_checkable
class VectorResource(Protocol):
    """Vector database access (pgvector)."""

    async def search(
        self, embedding: list[float], *, limit: int = 10, filter: dict | None = None
    ) -> list[dict]: ...
    async def upsert(
        self, id: str, embedding: list[float], metadata: dict | None = None
    ) -> None: ...


@runtime_checkable
class StorageResource(Protocol):
    """Object storage access (S3-compatible)."""

    async def get(self, key: str) -> bytes: ...
    async def put(self, key: str, data: bytes, *, content_type: str | None = None) -> None: ...
    async def delete(self, key: str) -> None: ...
    async def presign(self, key: str, *, expires_in: int = 3600) -> str: ...


@runtime_checkable
class LLMResource(Protocol):
    """LLM access (model-agnostic)."""

    async def call(
        self, *, model: str, prompt: str, context: str | None = None, **kwargs: Any
    ) -> dict: ...
    async def embed(self, text: str, *, model: str | None = None) -> list[float]: ...


@runtime_checkable
class MemoryResource(Protocol):
    """Agent memory access (short-term + long-term)."""

    @property
    def short_term(self) -> Any: ...
    @property
    def long_term(self) -> Any: ...


class ExecutionContext(ABC):
    """Abstract base providing access to all resources during block execution.

    Concrete implementations inject actual resource connections.
    Agnostic to resource origin — works same for self-hosted, cloud, or hybrid.
    """

    @property
    @abstractmethod
    def db(self) -> DBResource: ...

    @property
    @abstractmethod
    def vector(self) -> VectorResource: ...

    @property
    @abstractmethod
    def storage(self) -> StorageResource: ...

    @property
    @abstractmethod
    def llm(self) -> LLMResource: ...

    @property
    @abstractmethod
    def memory(self) -> MemoryResource: ...

    @property
    @abstractmethod
    def auth(self) -> AuthContext: ...

    @property
    def session_id(self) -> str | None:
        """Current session ID (for traceability in db_write)."""
        return None

    @property
    def events(self) -> Any | None:
        """EventEmitter for streaming events (LLM tokens, progress). Optional."""
        return None

    @property
    def vault(self) -> Any | None:
        """Knowledge Vault service (optional). Available when storage is configured."""
        return None

    @property
    def node_id(self) -> str | None:
        """Current executing node ID. Set by runner during execution."""
        return None

    @property
    def fingerprint(self) -> Any | None:
        """SessionFingerprint for stealth scraping. Generated once per session/cycle."""
        return None

    @property
    def system_prompt(self) -> str | None:
        """Agent-level system prompt. Injected into all ai/llm_call nodes."""
        return None
