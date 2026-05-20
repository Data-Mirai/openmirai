"""DB Resource implementations — InMemory for dev/testing."""

from __future__ import annotations

import uuid
from datetime import datetime, timezone
from typing import Any


class InMemoryDBResource:
    """In-memory database for dev/testing. Stores rows in dicts by table name.

    Supports both legacy __table__ convention (backward compat) and
    new schema-aware methods (ensure_table, insert, query).
    """

    def __init__(self) -> None:
        self._tables: dict[str, dict[str, dict[str, Any]]] = {}
        self._schemas: dict[str, list[dict[str, Any]]] = {}

    # --- Legacy DBResource protocol ---

    async def execute(self, query: str, params: dict[str, Any] | None = None) -> Any:
        if params is None:
            return None
        table = params.get("__table__", "_default")
        if table not in self._tables:
            self._tables[table] = {}

        row_id = params.get("id")
        query_upper = query.upper()

        if "DELETE" in query_upper and row_id:
            self._tables[table].pop(row_id, None)
        elif row_id:
            row = {k: v for k, v in params.items() if k != "__table__"}
            self._tables[table][row_id] = row

        return None

    async def fetch_one(
        self, query: str, params: dict[str, Any] | None = None
    ) -> dict | None:
        if params is None:
            return None
        table = params.get("__table__", "_default")
        row_id = params.get("id")
        if table in self._tables and row_id and row_id in self._tables[table]:
            return dict(self._tables[table][row_id])
        return None

    async def fetch_all(
        self, query: str, params: dict[str, Any] | None = None
    ) -> list[dict]:
        if params is None:
            return []
        table = params.get("__table__", "_default")
        if table not in self._tables:
            return []
        return [dict(row) for row in self._tables[table].values()]

    # --- Schema-aware methods (FEAT-019) ---

    async def ensure_table(self, table: str, columns: list[dict[str, Any]]) -> None:
        """Register table schema. Creates internal storage if needed."""
        if table not in self._tables:
            self._tables[table] = {}
        self._schemas[table] = columns

    async def insert(self, table: str, data: dict[str, Any], meta: dict[str, Any] | None = None) -> dict[str, Any]:
        """Insert a row with auto-generated system columns."""
        meta = meta or {}
        if table not in self._tables:
            self._tables[table] = {}

        row = {**data}
        if "id" not in row:
            row["id"] = str(uuid.uuid4())
        if "created_at" not in row:
            row["created_at"] = datetime.now(timezone.utc).isoformat()
        row["session_id"] = meta.get("session_id", "")
        row["node_id"] = meta.get("node_id", "")

        self._tables[table][row["id"]] = row
        return dict(row)

    async def query(
        self,
        table: str,
        *,
        where: dict[str, Any] | None = None,
        limit: int | None = None,
        order_by: str | None = None,
    ) -> list[dict[str, Any]]:
        """Query rows with optional filtering."""
        if table not in self._tables:
            return []

        rows = list(self._tables[table].values())

        if where:
            filtered = []
            for row in rows:
                match = all(row.get(k) == v for k, v in where.items())
                if match:
                    filtered.append(row)
            rows = filtered

        if order_by:
            parts = order_by.split()
            col = parts[0]
            reverse = len(parts) > 1 and parts[1].upper() == "DESC"
            rows.sort(key=lambda r: r.get(col, ""), reverse=reverse)

        if limit is not None:
            rows = rows[:limit]

        return [dict(r) for r in rows]
