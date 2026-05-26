"""SimpleExecutionContext — Concrete wiring of all resources."""

from __future__ import annotations

from typing import Any

from datamirai_engine.core.context import (
    AuthContext,
    DBResource,
    ExecutionContext,
    LLMResource,
    MemoryResource,
    StorageResource,
    VectorResource,
)


class _SimpleMemory:
    """Minimal memory holder — replaced by real memory in BLOCK-009/010."""

    def __init__(self) -> None:
        self._short_term: Any = None
        self._long_term: Any = None

    @property
    def short_term(self) -> Any:
        return self._short_term

    @property
    def long_term(self) -> Any:
        return self._long_term


class SimpleExecutionContext(ExecutionContext):
    """Ready-to-use ExecutionContext. Accepts any resource implementations."""

    def __init__(
        self,
        *,
        db: DBResource | None = None,
        vector: VectorResource | None = None,
        storage: StorageResource | None = None,
        llm: LLMResource | None = None,
        auth: AuthContext | None = None,
        memory: MemoryResource | None = None,
        event_emitter: Any = None,
        session_id: str | None = None,
        vault: Any = None,
        system_prompt: str | None = None,
        environment_id: str | None = None,
        api_base: str | None = None,
    ) -> None:
        from datamirai_engine.resources.db import InMemoryDBResource
        from datamirai_engine.resources.llm import MockLLMResource
        from datamirai_engine.resources.storage import InMemoryStorageResource
        from datamirai_engine.resources.vector import InMemoryVectorResource

        self._db = db or InMemoryDBResource()
        self._vector = vector or InMemoryVectorResource()
        self._storage = storage or InMemoryStorageResource()
        self._llm = llm or MockLLMResource()
        self._auth = auth or AuthContext(user_id="dev", role="OWNER")
        self._memory = memory or _SimpleMemory()
        self._event_emitter = event_emitter
        self._session_id = session_id
        self._vault = vault
        self._node_id: str | None = None
        self._system_prompt = system_prompt
        self._environment_id = environment_id
        self._api_base = api_base or "http://localhost:8000"
        self._fingerprint: Any = None

    @property
    def db(self) -> DBResource:
        return self._db

    @property
    def vector(self) -> VectorResource:
        return self._vector

    @property
    def storage(self) -> StorageResource:
        return self._storage

    @property
    def llm(self) -> LLMResource:
        return self._llm

    @property
    def memory(self) -> MemoryResource:
        return self._memory

    @property
    def auth(self) -> AuthContext:
        return self._auth

    @property
    def session_id(self) -> str | None:
        return self._session_id

    @property
    def vault(self) -> Any | None:
        return self._vault

    @property
    def events(self) -> Any | None:
        return self._event_emitter

    @property
    def node_id(self) -> str | None:
        return self._node_id

    @node_id.setter
    def node_id(self, value: str | None) -> None:
        self._node_id = value

    @property
    def fingerprint(self) -> Any | None:
        if self._fingerprint is None:
            from datamirai_engine.tools.builtin.data.stealth import SessionFingerprint
            self._fingerprint = SessionFingerprint.generate()
        return self._fingerprint

    @fingerprint.setter
    def fingerprint(self, value: Any) -> None:
        self._fingerprint = value

    @property
    def system_prompt(self) -> str | None:
        return self._system_prompt

    @property
    def environment_id(self) -> str | None:
        return self._environment_id

    @property
    def api_base(self) -> str:
        return self._api_base

    @classmethod
    def default(cls) -> SimpleExecutionContext:
        """Create a fully wired dev/testing context with in-memory resources."""
        from datamirai_engine.resources.db import InMemoryDBResource
        from datamirai_engine.resources.llm import MockLLMResource
        from datamirai_engine.resources.storage import InMemoryStorageResource
        from datamirai_engine.resources.vector import InMemoryVectorResource

        return cls(
            db=InMemoryDBResource(),
            vector=InMemoryVectorResource(),
            storage=InMemoryStorageResource(),
            llm=MockLLMResource(),
            auth=AuthContext(user_id="dev", role="OWNER"),
        )
