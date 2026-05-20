"""Search providers for SQLite backend."""

from __future__ import annotations

import sqlite3
from pathlib import Path


class SQLiteFTSProvider:
    """FTS5 search over the agent_memory table in SQLite."""

    def __init__(
        self,
        db_path: str | Path,
        table: str = "agent_memory",
        fts_table: str = "agent_memory_fts",
        agent_id: str | None = None,
    ):
        self._db_path = str(db_path)
        self._table = table
        self._fts_table = fts_table
        self._agent_id = agent_id

    async def search(self, query: str, limit: int = 10) -> list[dict]:
        """Search using FTS5 MATCH with BM25 ranking.

        Falls back to LIKE if FTS query syntax fails.
        """
        conn = sqlite3.connect(self._db_path)
        conn.row_factory = sqlite3.Row
        try:
            safe_query = self._sanitize_fts_query(query)

            if self._agent_id:
                rows = conn.execute(
                    f"SELECT m.id, m.summary as content, fts.rank "
                    f"FROM {self._fts_table} fts "
                    f"JOIN {self._table} m ON m.rowid = fts.rowid "
                    f"WHERE {self._fts_table} MATCH ? AND m.agent_id = ? "
                    f"ORDER BY fts.rank LIMIT ?",
                    (safe_query, self._agent_id, limit),
                ).fetchall()
            else:
                rows = conn.execute(
                    f"SELECT m.id, m.summary as content, fts.rank "
                    f"FROM {self._fts_table} fts "
                    f"JOIN {self._table} m ON m.rowid = fts.rowid "
                    f"WHERE {self._fts_table} MATCH ? "
                    f"ORDER BY fts.rank LIMIT ?",
                    (safe_query, limit),
                ).fetchall()

            return [
                {
                    "id": r["id"],
                    "content": r["content"],
                    "rank": r["rank"],
                    "metadata": {},
                }
                for r in rows
            ]
        except sqlite3.OperationalError:
            # Fallback to LIKE if FTS match fails
            return await self._fallback_like(conn, query, limit)
        finally:
            conn.close()

    async def _fallback_like(
        self, conn: sqlite3.Connection, query: str, limit: int
    ) -> list[dict]:
        """LIKE-based fallback when FTS5 MATCH syntax fails."""
        like = f"%{query}%"
        if self._agent_id:
            rows = conn.execute(
                f"SELECT id, summary as content FROM {self._table} "
                f"WHERE agent_id = ? AND summary LIKE ? LIMIT ?",
                (self._agent_id, like, limit),
            ).fetchall()
        else:
            rows = conn.execute(
                f"SELECT id, summary as content FROM {self._table} "
                f"WHERE summary LIKE ? LIMIT ?",
                (like, limit),
            ).fetchall()
        return [
            {"id": r["id"], "content": r["content"], "rank": 0.5, "metadata": {}}
            for r in rows
        ]

    @staticmethod
    def _sanitize_fts_query(query: str) -> str:
        """Remove FTS5 special characters that could cause syntax errors.

        Wraps each word in quotes and joins with OR for broad matching.
        """
        words = query.split()
        if not words:
            return '""'
        return " OR ".join(f'"{w}"' for w in words if w.strip())
