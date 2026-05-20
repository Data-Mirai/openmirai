"""Hybrid Search Engine -- combines vector search + full-text search.

Formula: score_total = (w_vec * vector_score) + (w_fts * fts_score)
Fallback: if no embeddings -> FTS only. If no FTS -> vector only.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol


@dataclass
class SearchResult:
    """Single result from hybrid search with individual score components."""

    id: str
    content: str
    score: float = 0.0
    vector_score: float = 0.0
    fts_score: float = 0.0
    metadata: dict[str, Any] = field(default_factory=dict)


class VectorSearchProvider(Protocol):
    """Abstract vector search. Implemented per backend (SQLite/Postgres)."""

    async def search(self, embedding: list[float], limit: int = 10) -> list[dict]:
        """Returns list of {id, content, distance, metadata}."""
        ...


class FTSProvider(Protocol):
    """Abstract full-text search. Implemented per backend."""

    async def search(self, query: str, limit: int = 10) -> list[dict]:
        """Returns list of {id, content, rank, metadata}."""
        ...


class HybridSearchEngine:
    """Merges vector + FTS results using weighted score fusion.

    Weight normalization is applied at init so w_vec + w_fts always == 1.0.
    Provider errors are swallowed -- partial results are always returned.
    """

    def __init__(
        self,
        vector_provider: VectorSearchProvider | None = None,
        fts_provider: FTSProvider | None = None,
        w_vec: float = 0.7,
        w_fts: float = 0.3,
    ):
        self._vector = vector_provider
        self._fts = fts_provider
        # Normalize weights so they sum to 1.0
        total = w_vec + w_fts
        self._w_vec = w_vec / total if total > 0 else 0.5
        self._w_fts = w_fts / total if total > 0 else 0.5

    async def search(
        self,
        query: str,
        *,
        query_embedding: list[float] | None = None,
        limit: int = 10,
    ) -> list[SearchResult]:
        """Execute hybrid search.

        If query_embedding provided + vector provider available: hybrid search.
        If only FTS available: FTS-only search.
        If nothing available: empty results.
        """
        vector_results: dict[str, dict] = {}
        fts_results: dict[str, dict] = {}

        # Vector search
        if query_embedding and self._vector:
            try:
                raw = await self._vector.search(query_embedding, limit=limit * 2)
                for r in raw:
                    vector_results[r["id"]] = r
            except Exception:
                pass  # never fail -- return partial

        # FTS search
        if self._fts:
            try:
                raw = await self._fts.search(query, limit=limit * 2)
                for r in raw:
                    fts_results[r["id"]] = r
            except Exception:
                pass  # never fail -- return partial

        # Merge + score fusion
        all_ids = set(vector_results.keys()) | set(fts_results.keys())

        if not all_ids:
            return []

        # Normalize vector scores: convert distance to similarity (1 - d/max)
        vec_scores: dict[str, float] = {}
        if vector_results:
            raw_distances = {
                id: r.get("distance", 0.0) for id, r in vector_results.items()
            }
            max_vec = max(raw_distances.values()) if raw_distances else 1.0
            max_vec = max_vec if max_vec > 0 else 1.0
            vec_scores = {
                id: 1.0 - (d / max_vec) for id, d in raw_distances.items()
            }

        # Normalize FTS scores: rank is negative in BM25, lower = better
        fts_scores: dict[str, float] = {}
        if fts_results:
            raw_ranks = {
                id: r.get("rank", 0.0) for id, r in fts_results.items()
            }
            rank_values = list(raw_ranks.values())
            min_rank = min(rank_values)
            max_rank = max(rank_values)
            range_rank = max_rank - min_rank if max_rank != min_rank else 1.0
            fts_scores = {
                id: (r - min_rank) / range_rank for id, r in raw_ranks.items()
            }

        # Fuse scores
        results: list[SearchResult] = []
        for id in all_ids:
            v_score = vec_scores.get(id, 0.0)
            f_score = fts_scores.get(id, 0.0)

            # Weight based on what sources are available
            if vector_results and fts_results:
                total_score = (self._w_vec * v_score) + (self._w_fts * f_score)
            elif vector_results:
                total_score = v_score
            else:
                total_score = f_score

            # Get content from whichever source has it
            source = vector_results.get(id, fts_results.get(id, {}))

            results.append(
                SearchResult(
                    id=id,
                    content=source.get("content", ""),
                    score=total_score,
                    vector_score=v_score,
                    fts_score=f_score,
                    metadata=source.get("metadata", {}),
                )
            )

        # Sort by score desc, return top N
        results.sort(key=lambda r: r.score, reverse=True)
        return results[:limit]
