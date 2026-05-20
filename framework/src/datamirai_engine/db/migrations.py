"""Database migrations — standalone engine schema.

This is the simplified schema for the standalone engine.
No universe/environment references — just graphs, agents, sessions.
The full schema (schema.sql) is for the Cloud version.
"""

from __future__ import annotations

import logging
from typing import Any

logger = logging.getLogger(__name__)

SCHEMA_VERSION = 1

SCHEMA_SQL = """\
-- Data Mirai Engine — Standalone Schema v1
-- Simplified for self-hosted / library use

-- ── Graphs ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS graphs (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    version     TEXT NOT NULL DEFAULT '1.0',
    nodes       JSONB NOT NULL DEFAULT '[]',
    edges       JSONB NOT NULL DEFAULT '[]',
    metadata    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── Agents ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS agents (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    graph_id    TEXT NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'disabled',
    triggers    JSONB NOT NULL DEFAULT '[]',
    metadata    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);

-- ── Sessions ─────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    agent_name  TEXT NOT NULL,
    graph_id    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running',
    trace       JSONB NOT NULL DEFAULT '[]',
    transcript  JSONB NOT NULL DEFAULT '[]',
    state       JSONB NOT NULL DEFAULT '{}',
    error       TEXT,
    started_at  DOUBLE PRECISION NOT NULL,
    finished_at DOUBLE PRECISION,
    duration_ms DOUBLE PRECISION
);

CREATE INDEX IF NOT EXISTS idx_sessions_agent ON sessions(agent_id);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

-- ── Schema version tracking ─────────────────────────────────

CREATE TABLE IF NOT EXISTS _schema_version (
    version     INT PRIMARY KEY,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO _schema_version (version)
VALUES (1)
ON CONFLICT (version) DO NOTHING;
"""


async def run_migrations(pool: Any) -> None:
    """Apply schema migrations. Idempotent (uses IF NOT EXISTS)."""
    async with pool.acquire() as conn:
        await conn.execute(SCHEMA_SQL)
        logger.info("Schema v%d applied", SCHEMA_VERSION)


async def get_schema_version(pool: Any) -> int | None:
    """Check current schema version. Returns None if no schema."""
    async with pool.acquire() as conn:
        try:
            row = await conn.fetchrow(
                "SELECT MAX(version) as v FROM _schema_version"
            )
            return row["v"] if row else None
        except Exception:
            return None


async def get_table_counts(pool: Any) -> dict[str, int]:
    """Get row counts for main tables. Useful for `datamirai db status`."""
    tables = ["graphs", "agents", "sessions"]
    counts: dict[str, int] = {}
    async with pool.acquire() as conn:
        for table in tables:
            try:
                row = await conn.fetchrow(f"SELECT COUNT(*) as c FROM {table}")
                counts[table] = row["c"] if row else 0
            except Exception:
                counts[table] = -1
    return counts
