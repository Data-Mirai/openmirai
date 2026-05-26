"""SQLite DB Resource — local persistent database for agent data.

Uses per-operation connections. WAL mode + busy_timeout handle concurrency
at the SQLite level without needing application-level locks.
"""

from __future__ import annotations

import asyncio
import json as _json
import re
import sqlite3
import uuid
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


# Safe identifier pattern: only allow alphanumeric + underscore
_SAFE_IDENT = re.compile(r"^[a-zA-Z_][a-zA-Z0-9_]*$")


def _quote_ident(name: str) -> str:
    """Quote a SQL identifier safely. Raises on invalid names."""
    if not _SAFE_IDENT.match(name):
        raise ValueError(f"Invalid SQL identifier: {name!r}")
    return f'"{name}"'


class SQLiteDBResource:
    """SQLite-backed DB resource for local persistent storage.

    Each operation opens a fresh connection, does the work, and closes it.
    WAL mode + busy_timeout handle concurrent access safely.
    """

    def __init__(self, db_path: str | Path) -> None:
        self._db_path = Path(db_path)
        self._db_path.parent.mkdir(parents=True, exist_ok=True)
        # Initialize WAL mode (persists in the database file)
        conn = sqlite3.connect(str(self._db_path), timeout=30)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.close()

    @contextmanager
    def _open(self):
        """Short-lived connection with auto-commit/rollback."""
        conn = sqlite3.connect(str(self._db_path), timeout=30)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA busy_timeout=10000")
        conn.execute("PRAGMA foreign_keys=ON")
        try:
            yield conn
            conn.commit()
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    @property
    def path(self) -> Path:
        return self._db_path

    # --- Public async API ---

    async def execute(self, query: str, params: dict[str, Any] | None = None) -> Any:
        if params is not None and "__table__" in params:
            return await self._legacy_execute(query, params)
        return await asyncio.to_thread(self._exec_sql, query, params)

    async def fetch_one(self, query: str, params: dict[str, Any] | None = None) -> dict | None:
        if params is not None and "__table__" in params:
            return await self._legacy_fetch_one(params)
        return await asyncio.to_thread(self._fetch_one_sql, query, params)

    async def fetch_all(self, query: str, params: dict[str, Any] | None = None) -> list[dict]:
        if params is not None and "__table__" in params:
            return await self._legacy_fetch_all(params)
        return await asyncio.to_thread(self._fetch_all_sql, query, params)

    async def ensure_table(self, table: str, columns: list[dict[str, Any]]) -> None:
        await asyncio.to_thread(self._ensure_table_sync, table, columns)

    async def insert(self, table: str, data: dict[str, Any], meta: dict[str, Any] | None = None) -> dict[str, Any]:
        return await asyncio.to_thread(self._insert_sync, table, data, meta)

    async def query(
        self,
        table: str,
        *,
        where: dict[str, Any] | None = None,
        limit: int | None = None,
        order_by: str | None = None,
    ) -> list[dict[str, Any]]:
        return await asyncio.to_thread(self._query_sync, table, where, limit, order_by)

    # --- Sync implementations (each opens its own connection) ---

    def _exec_sql(self, query: str, params: Any | None) -> None:
        with self._open() as conn:
            conn.execute(query, params or ())

    def _fetch_one_sql(self, query: str, params: Any | None) -> dict | None:
        with self._open() as conn:
            row = conn.execute(query, params or ()).fetchone()
            return dict(row) if row else None

    def _fetch_all_sql(self, query: str, params: Any | None) -> list[dict]:
        with self._open() as conn:
            return [dict(r) for r in conn.execute(query, params or ()).fetchall()]

    def _ensure_table_sync(self, table: str, columns: list[dict[str, Any]]) -> None:
        tbl = _quote_ident(table)
        col_defs = [
            '"id" TEXT PRIMARY KEY',
            '"created_at" TEXT NOT NULL',
            '"session_id" TEXT',
            '"node_id" TEXT',
        ]
        system_cols = {"id", "created_at", "session_id", "node_id"}
        for col in columns:
            name = col["name"]
            if name in system_cols:
                continue
            sql_type = _map_type_sqlite(col.get("type", "text"))
            nullable = col.get("nullable", True)
            col_def = f'{_quote_ident(name)} {sql_type}'
            if not nullable:
                col_def += " NOT NULL"
            col_defs.append(col_def)

        with self._open() as conn:
            conn.execute(f"CREATE TABLE IF NOT EXISTS {tbl} ({', '.join(col_defs)})")
            existing = {r["name"] for r in conn.execute(f"PRAGMA table_info({tbl})").fetchall()}
            for col in columns:
                name = col["name"]
                if name not in existing and name not in system_cols:
                    sql_type = _map_type_sqlite(col.get("type", "text"))
                    conn.execute(f"ALTER TABLE {tbl} ADD COLUMN {_quote_ident(name)} {sql_type}")

    def _insert_sync(self, table: str, data: dict[str, Any], meta: dict[str, Any] | None) -> dict[str, Any]:
        meta = meta or {}
        row = {**data}
        if "id" not in row:
            row["id"] = str(uuid.uuid4())
        if "created_at" not in row:
            row["created_at"] = datetime.now(timezone.utc).isoformat()
        row["session_id"] = meta.get("session_id", "")
        row["node_id"] = meta.get("node_id", "")

        serialized_values = []
        for v in row.values():
            if isinstance(v, (list, dict)):
                serialized_values.append(_json.dumps(v, ensure_ascii=False, default=str))
            elif isinstance(v, bool):
                serialized_values.append(1 if v else 0)
            else:
                serialized_values.append(v)

        tbl = _quote_ident(table)
        col_names = [_quote_ident(k) for k in row.keys()]
        placeholders = ["?" for _ in row]

        with self._open() as conn:
            # Auto-create table if it doesn't exist
            tables = {r[0] for r in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()}
            if table not in tables:
                col_defs = [
                    '"id" TEXT PRIMARY KEY', '"created_at" TEXT NOT NULL',
                    '"session_id" TEXT', '"node_id" TEXT',
                ]
                for k in row.keys():
                    if k not in {"id", "created_at", "session_id", "node_id"}:
                        col_defs.append(f'{_quote_ident(k)} TEXT')
                conn.execute(f"CREATE TABLE IF NOT EXISTS {tbl} ({', '.join(col_defs)})")
            else:
                # Auto-add missing columns
                existing_cols = {r["name"] for r in conn.execute(f"PRAGMA table_info({tbl})").fetchall()}
                for k in row.keys():
                    if k not in existing_cols:
                        conn.execute(f"ALTER TABLE {tbl} ADD COLUMN {_quote_ident(k)} TEXT")

            conn.execute(
                f"INSERT INTO {tbl} ({', '.join(col_names)}) VALUES ({', '.join(placeholders)})",
                serialized_values,
            )
        return dict(row)

    def _query_sync(
        self,
        table: str,
        where: dict[str, Any] | None,
        limit: int | None,
        order_by: str | None,
    ) -> list[dict[str, Any]]:
        tbl = _quote_ident(table)
        sql = f"SELECT * FROM {tbl}"
        values: list[Any] = []

        if where:
            clauses = []
            for k, v in where.items():
                clauses.append(f"{_quote_ident(k)} = ?")
                values.append(v)
            sql += " WHERE " + " AND ".join(clauses)

        if order_by:
            parts = order_by.split()
            col = parts[0]
            direction = parts[1].upper() if len(parts) > 1 else "ASC"
            if direction not in ("ASC", "DESC"):
                direction = "ASC"
            sql += f" ORDER BY {_quote_ident(col)} {direction}"

        if limit is not None:
            sql += f" LIMIT {int(limit)}"

        with self._open() as conn:
            rows = conn.execute(sql, values).fetchall()
            return [dict(r) for r in rows]

    # --- Legacy shim (compat with __table__ convention) ---

    async def _legacy_execute(self, query: str, params: dict[str, Any]) -> None:
        table = params.get("__table__", "_default")
        row = {k: v for k, v in params.items() if k != "__table__"}
        query_upper = query.upper()

        if "DELETE" in query_upper:
            row_id = row.get("id")
            if row_id:
                tbl = _quote_ident(table)
                await asyncio.to_thread(
                    self._exec_sql, f"DELETE FROM {tbl} WHERE id = ?", (row_id,)
                )
        else:
            await asyncio.to_thread(self._legacy_upsert_sync, table, row)

    def _legacy_upsert_sync(self, table: str, row: dict[str, Any]) -> None:
        tbl = _quote_ident(table)
        with self._open() as conn:
            tables = {r[0] for r in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()}
            if table not in tables:
                col_defs = []
                for k in row.keys():
                    if k == "id":
                        col_defs.append('"id" TEXT PRIMARY KEY')
                    else:
                        col_defs.append(f'{_quote_ident(k)} TEXT')
                if not any(k == "id" for k in row.keys()):
                    col_defs.insert(0, '"id" TEXT PRIMARY KEY')
                conn.execute(f"CREATE TABLE IF NOT EXISTS {tbl} ({', '.join(col_defs)})")
            else:
                existing_cols = {r["name"] for r in conn.execute(f"PRAGMA table_info({tbl})").fetchall()}
                for k in row.keys():
                    if k not in existing_cols:
                        conn.execute(f"ALTER TABLE {tbl} ADD COLUMN {_quote_ident(k)} TEXT")

            col_names = [_quote_ident(k) for k in row.keys()]
            placeholders = ["?" for _ in row]
            conn.execute(
                f"INSERT OR REPLACE INTO {tbl} ({', '.join(col_names)}) VALUES ({', '.join(placeholders)})",
                list(row.values()),
            )

    async def _legacy_fetch_one(self, params: dict[str, Any]) -> dict | None:
        table = params.get("__table__", "_default")
        row_id = params.get("id")
        if not row_id:
            return None
        tbl = _quote_ident(table)
        return await asyncio.to_thread(
            self._fetch_one_sql, f"SELECT * FROM {tbl} WHERE id = ?", (row_id,)
        )

    async def _legacy_fetch_all(self, params: dict[str, Any]) -> list[dict]:
        table = params.get("__table__", "_default")
        tbl = _quote_ident(table)
        return await asyncio.to_thread(
            self._fetch_all_sql, f"SELECT * FROM {tbl}", None
        )


def _map_type_sqlite(type_name: str) -> str:
    """Map schema type names to SQLite types."""
    mapping = {
        "text": "TEXT",
        "string": "TEXT",
        "integer": "INTEGER",
        "int": "INTEGER",
        "float": "REAL",
        "number": "REAL",
        "boolean": "INTEGER",
        "bool": "INTEGER",
        "json": "TEXT",
        "object": "TEXT",
        "timestamp": "TEXT",
        "datetime": "TEXT",
    }
    return mapping.get(type_name.lower(), "TEXT")
