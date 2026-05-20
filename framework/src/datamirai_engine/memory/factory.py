"""MemoryBackendFactory -- creates the right backend based on config."""

from __future__ import annotations

from pathlib import Path

from .backend import MemoryBackend
from .in_memory_backend import InMemoryBackend
from .sqlite_backend import SQLiteMemoryBackend


class MemoryBackendFactory:
    """Factory for creating memory backends.

    If db_path is provided, returns SQLiteMemoryBackend (persistent).
    Otherwise returns InMemoryBackend (volatile, for dev/testing).
    """

    @staticmethod
    def create(db_path: str | Path | None = None) -> MemoryBackend:
        """Create a memory backend.

        Args:
            db_path: Path to SQLite database. If None, uses in-memory backend.

        Returns:
            MemoryBackend instance.
        """
        if db_path:
            return SQLiteMemoryBackend(db_path)
        return InMemoryBackend()
