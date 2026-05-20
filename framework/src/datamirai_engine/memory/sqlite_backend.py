"""SQLiteMemoryBackend -- persistent backend with FTS5 full-text search.

Uses the same SQLite database as the app. FTS5 enables keyword search
over summaries and tags. Suitable for production single-node deployments.
"""

from __future__ import annotations

import json
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .backend import LogEntry, MemoryBackend, MemoryEntry

_SCHEMA = """\
CREATE TABLE IF NOT EXISTS agent_memory (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    decisions TEXT DEFAULT '[]',
    learnings TEXT DEFAULT '[]',
    tags TEXT DEFAULT '[]',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_memory_agent ON agent_memory(agent_id);

CREATE VIRTUAL TABLE IF NOT EXISTS agent_memory_fts USING fts5(
    summary, tags, content=agent_memory, content_rowid=rowid
);

CREATE TRIGGER IF NOT EXISTS agent_memory_ai AFTER INSERT ON agent_memory BEGIN
    INSERT INTO agent_memory_fts(rowid, summary, tags)
    VALUES (new.rowid, new.summary, new.tags);
END;

CREATE TRIGGER IF NOT EXISTS agent_memory_ad AFTER DELETE ON agent_memory BEGIN
    INSERT INTO agent_memory_fts(agent_memory_fts, rowid, summary, tags)
    VALUES('delete', old.rowid, old.summary, old.tags);
END;

CREATE TABLE IF NOT EXISTS agent_log (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata TEXT DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_log_agent ON agent_log(agent_id);
CREATE INDEX IF NOT EXISTS idx_agent_log_session ON agent_log(agent_id, session_id);
"""


class SQLiteMemoryBackend(MemoryBackend):
    """SQLite persistent backend with FTS5 full-text search."""

    def __init__(self, db_path: str | Path) -> None:
        self._db_path = Path(db_path)
        self._db_path.parent.mkdir(parents=True, exist_ok=True)
        self._ensure_tables()

    def _get_conn(self) -> sqlite3.Connection:
        conn = sqlite3.connect(str(self._db_path))
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA foreign_keys=ON")
        return conn

    def _ensure_tables(self) -> None:
        conn = self._get_conn()
        try:
            conn.executescript(_SCHEMA)
            conn.commit()
        finally:
            conn.close()

    async def save_learning(
        self,
        agent_id: str,
        session_id: str,
        summary: str,
        decisions: list[str] | None = None,
        learnings: list[str] | None = None,
        tags: list[str] | None = None,
    ) -> str:
        memory_id = str(uuid.uuid4())
        created_at = datetime.now(timezone.utc).isoformat()
        decisions_json = json.dumps(decisions or [])
        learnings_json = json.dumps(learnings or [])
        tags_json = json.dumps(tags or [])

        conn = self._get_conn()
        try:
            conn.execute(
                """INSERT INTO agent_memory
                   (id, agent_id, session_id, summary, decisions, learnings, tags, created_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                (memory_id, agent_id, session_id, summary,
                 decisions_json, learnings_json, tags_json, created_at),
            )
            conn.commit()
        finally:
            conn.close()
        return memory_id

    async def search(
        self, agent_id: str, query: str, limit: int = 10
    ) -> list[MemoryEntry]:
        from datamirai_engine.search.hybrid import HybridSearchEngine
        from datamirai_engine.search.providers import SQLiteFTSProvider

        fts = SQLiteFTSProvider(self._db_path, agent_id=agent_id)
        engine = HybridSearchEngine(fts_provider=fts)

        results = await engine.search(query, limit=limit)

        # Convert SearchResults to MemoryEntries
        entries: list[MemoryEntry] = []
        for r in results:
            full = await self._get_by_id(r.id)
            if full:
                full.score = r.score
                entries.append(full)
        return entries

    async def _get_by_id(self, memory_id: str) -> MemoryEntry | None:
        """Fetch a single memory entry by ID."""
        conn = self._get_conn()
        try:
            row = conn.execute(
                "SELECT * FROM agent_memory WHERE id = ?", (memory_id,)
            ).fetchone()
            if row is None:
                return None
            return self._row_to_entry(row)
        finally:
            conn.close()

    async def _search_fallback(
        self,
        conn: sqlite3.Connection,
        agent_id: str,
        query: str,
        limit: int,
    ) -> list[MemoryEntry]:
        """LIKE-based fallback when FTS match fails."""
        pattern = f"%{query}%"
        rows = conn.execute(
            """SELECT * FROM agent_memory
               WHERE agent_id = ?
                 AND (summary LIKE ? OR tags LIKE ?)
               ORDER BY created_at DESC
               LIMIT ?""",
            (agent_id, pattern, pattern, limit),
        ).fetchall()
        return [self._row_to_entry(row) for row in rows]

    @staticmethod
    def _prepare_fts_query(query: str) -> str:
        """Prepare query for FTS5 MATCH. Adds * for prefix matching."""
        # Strip FTS special chars to avoid syntax errors
        cleaned = query.strip()
        if not cleaned:
            return ""
        # Split into tokens, add * for prefix matching on each
        tokens = cleaned.split()
        # Wrap each token in quotes to handle special chars, add *
        parts = [f'"{token}"*' for token in tokens if token]
        return " ".join(parts)

    async def get_recent(
        self, agent_id: str, limit: int = 10
    ) -> list[MemoryEntry]:
        conn = self._get_conn()
        try:
            rows = conn.execute(
                """SELECT * FROM agent_memory
                   WHERE agent_id = ?
                   ORDER BY created_at DESC
                   LIMIT ?""",
                (agent_id, limit),
            ).fetchall()
            return [self._row_to_entry(row) for row in rows]
        finally:
            conn.close()

    async def delete(self, memory_id: str) -> bool:
        conn = self._get_conn()
        try:
            cursor = conn.execute(
                "DELETE FROM agent_memory WHERE id = ?", (memory_id,)
            )
            conn.commit()
            return cursor.rowcount > 0
        finally:
            conn.close()

    async def save_log(
        self,
        agent_id: str,
        session_id: str,
        message: str,
        metadata: dict[str, Any] | None = None,
    ) -> str:
        log_id = str(uuid.uuid4())
        created_at = datetime.now(timezone.utc).isoformat()
        metadata_json = json.dumps(metadata or {})

        conn = self._get_conn()
        try:
            conn.execute(
                """INSERT INTO agent_log
                   (id, agent_id, session_id, message, metadata, created_at)
                   VALUES (?, ?, ?, ?, ?, ?)""",
                (log_id, agent_id, session_id, message, metadata_json, created_at),
            )
            conn.commit()
        finally:
            conn.close()
        return log_id

    async def get_logs(
        self,
        agent_id: str,
        limit: int = 50,
        session_id: str | None = None,
    ) -> list[LogEntry]:
        conn = self._get_conn()
        try:
            if session_id:
                rows = conn.execute(
                    """SELECT * FROM agent_log
                       WHERE agent_id = ? AND session_id = ?
                       ORDER BY created_at DESC
                       LIMIT ?""",
                    (agent_id, session_id, limit),
                ).fetchall()
            else:
                rows = conn.execute(
                    """SELECT * FROM agent_log
                       WHERE agent_id = ?
                       ORDER BY created_at DESC
                       LIMIT ?""",
                    (agent_id, limit),
                ).fetchall()
            return [self._row_to_log_entry(row) for row in rows]
        finally:
            conn.close()

    async def count(self, agent_id: str) -> int:
        conn = self._get_conn()
        try:
            row = conn.execute(
                "SELECT COUNT(*) as cnt FROM agent_memory WHERE agent_id = ?",
                (agent_id,),
            ).fetchone()
            return row["cnt"]
        finally:
            conn.close()

    @staticmethod
    def _row_to_entry(row: sqlite3.Row, score: float | None = None) -> MemoryEntry:
        return MemoryEntry(
            id=row["id"],
            agent_id=row["agent_id"],
            session_id=row["session_id"],
            summary=row["summary"],
            decisions=json.loads(row["decisions"]),
            learnings=json.loads(row["learnings"]),
            tags=json.loads(row["tags"]),
            score=score,
            created_at=row["created_at"],
        )

    @staticmethod
    def _row_to_log_entry(row: sqlite3.Row) -> LogEntry:
        return LogEntry(
            id=row["id"],
            agent_id=row["agent_id"],
            session_id=row["session_id"],
            message=row["message"],
            metadata=json.loads(row["metadata"]),
            created_at=row["created_at"],
        )
