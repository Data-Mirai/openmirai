"""Database connection pool — asyncpg wrapper.

Usage:
    pool = await create_pool("postgresql://user:pass@localhost/datamirai")
    async with pool.acquire() as conn:
        rows = await conn.fetch("SELECT * FROM agents")
    await close_pool(pool)

The pool is optional — if DATABASE_URL is not set, the engine
runs fully in-memory (backwards compatible).
"""

from __future__ import annotations

import logging
import os
from typing import Any

logger = logging.getLogger(__name__)

Pool = Any  # asyncpg.Pool at runtime


def get_database_url() -> str | None:
    """Read DATABASE_URL from environment. Returns None if not set."""
    return os.environ.get("DATABASE_URL")


async def create_pool(dsn: str | None = None, **kwargs: Any) -> Pool:
    """Create an asyncpg connection pool.

    Args:
        dsn: PostgreSQL connection string. Falls back to DATABASE_URL env var.
        **kwargs: Extra args passed to asyncpg.create_pool().

    Returns:
        asyncpg.Pool instance.

    Raises:
        RuntimeError: If asyncpg is not installed.
        ConnectionError: If cannot connect to database.
    """
    try:
        import asyncpg
    except ImportError as e:
        raise RuntimeError(
            "asyncpg is required for Postgres persistence. "
            "Install with: pip install datamirai-engine[postgres]"
        ) from e

    url = dsn or get_database_url()
    if not url:
        raise ValueError(
            "No database URL provided. Set DATABASE_URL or pass dsn= argument."
        )

    try:
        pool = await asyncpg.create_pool(
            url,
            min_size=kwargs.pop("min_size", 2),
            max_size=kwargs.pop("max_size", 10),
            **kwargs,
        )
        logger.info("Database pool created (%s)", url.split("@")[-1] if "@" in url else url)
        return pool
    except Exception as e:
        raise ConnectionError(f"Cannot connect to database: {e}") from e


async def close_pool(pool: Pool) -> None:
    """Close the connection pool gracefully."""
    if pool is not None:
        await pool.close()
        logger.info("Database pool closed")
