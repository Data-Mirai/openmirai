"""Schema definitions for agent data tables.

Provides type mapping between abstract schema types and concrete SQL types
for SQLite and PostgreSQL backends.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class ColumnDef:
    """Definition of a single column in a table schema."""
    name: str
    type: str = "text"  # text, integer, float, boolean, json, timestamp
    nullable: bool = True
    default: str | None = None


@dataclass
class TableSchema:
    """Schema for a table created by db_write nodes.

    System columns (id, created_at, session_id, node_id) are always
    added automatically and should not be included in `columns`.
    """
    table: str
    columns: list[ColumnDef] = field(default_factory=list)

    SYSTEM_COLUMNS = ("id", "created_at", "session_id", "node_id")

    @classmethod
    def from_config(cls, config: dict[str, Any]) -> TableSchema | None:
        """Build TableSchema from a db_write node config dict.

        Returns None if no schema is defined.
        """
        table = config.get("table", "")
        if not table:
            return None
        raw_schema = config.get("schema")
        if not raw_schema:
            return cls(table=table)
        columns = []
        for col in raw_schema:
            if isinstance(col, dict):
                columns.append(ColumnDef(
                    name=col.get("name", ""),
                    type=col.get("type", "text"),
                    nullable=col.get("nullable", True),
                    default=col.get("default"),
                ))
        return cls(table=table, columns=columns)

    def to_columns_list(self) -> list[dict[str, Any]]:
        """Convert to the format expected by ensure_table()."""
        return [
            {"name": col.name, "type": col.type, "nullable": col.nullable}
            for col in self.columns
            if col.name not in self.SYSTEM_COLUMNS
        ]


# Type mapping: abstract type -> {sqlite: sql_type, postgres: sql_type}
SCHEMA_TYPE_MAP: dict[str, dict[str, str]] = {
    "text":      {"sqlite": "TEXT",    "postgres": "TEXT"},
    "string":    {"sqlite": "TEXT",    "postgres": "TEXT"},
    "integer":   {"sqlite": "INTEGER", "postgres": "INTEGER"},
    "int":       {"sqlite": "INTEGER", "postgres": "INTEGER"},
    "float":     {"sqlite": "REAL",    "postgres": "DOUBLE PRECISION"},
    "number":    {"sqlite": "REAL",    "postgres": "DOUBLE PRECISION"},
    "boolean":   {"sqlite": "INTEGER", "postgres": "BOOLEAN"},
    "bool":      {"sqlite": "INTEGER", "postgres": "BOOLEAN"},
    "json":      {"sqlite": "TEXT",    "postgres": "JSONB"},
    "object":    {"sqlite": "TEXT",    "postgres": "JSONB"},
    "timestamp": {"sqlite": "TEXT",    "postgres": "TIMESTAMPTZ"},
    "datetime":  {"sqlite": "TEXT",    "postgres": "TIMESTAMPTZ"},
}


def map_type(abstract_type: str, backend: str = "sqlite") -> str:
    """Map an abstract schema type to a concrete SQL type.

    Args:
        abstract_type: One of the keys in SCHEMA_TYPE_MAP.
        backend: "sqlite" or "postgres".

    Returns:
        SQL type string (e.g., "TEXT", "INTEGER", "JSONB").
    """
    entry = SCHEMA_TYPE_MAP.get(abstract_type.lower())
    if entry:
        return entry.get(backend, "TEXT")
    return "TEXT"
