"""Vector Resource implementations — InMemory for dev/testing."""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any


@dataclass
class _VectorEntry:
    id: str
    embedding: list[float]
    metadata: dict[str, Any] = field(default_factory=dict)


def _cosine_similarity(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b, strict=False))
    norm_a = math.sqrt(sum(x * x for x in a))
    norm_b = math.sqrt(sum(x * x for x in b))
    if norm_a == 0 or norm_b == 0:
        return 0.0
    return dot / (norm_a * norm_b)


class InMemoryVectorResource:
    """In-memory vector store with cosine similarity search."""

    def __init__(self) -> None:
        self._entries: dict[str, _VectorEntry] = {}

    async def search(
        self,
        embedding: list[float],
        *,
        limit: int = 10,
        filter: dict | None = None,
    ) -> list[dict]:
        if not self._entries:
            return []

        scored = []
        for entry in self._entries.values():
            if filter and not all(
                entry.metadata.get(k) == v for k, v in filter.items()
            ):
                continue
            score = _cosine_similarity(embedding, entry.embedding)
            scored.append((score, entry))

        scored.sort(key=lambda x: x[0], reverse=True)

        return [
            {"id": entry.id, "score": score, "metadata": dict(entry.metadata)}
            for score, entry in scored[:limit]
        ]

    async def upsert(
        self,
        id: str,
        embedding: list[float],
        metadata: dict | None = None,
    ) -> None:
        self._entries[id] = _VectorEntry(
            id=id, embedding=list(embedding), metadata=metadata or {}
        )
