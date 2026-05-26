"""Local Storage Resource — filesystem-based object storage.

Stores files under ~/.datamirai/universes/{slug}/{env}/storage/.
Files are directly accessible in Finder/file manager.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any


class LocalStorageResource:
    """Filesystem-backed storage resource.

    Implements the StorageResource protocol (get, put, delete, presign).
    Files are stored as regular files on disk.
    """

    def __init__(self, base_dir: str | Path) -> None:
        self._base_dir = Path(base_dir)
        self._base_dir.mkdir(parents=True, exist_ok=True)

    @property
    def path(self) -> Path:
        return self._base_dir

    async def get(self, key: str) -> bytes:
        loop = asyncio.get_event_loop()
        return await loop.run_in_executor(None, self._get_sync, key)

    async def put(self, key: str, data: bytes, *, content_type: str | None = None) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._put_sync, key, data)

    async def delete(self, key: str) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._delete_sync, key)

    async def presign(self, key: str, *, expires_in: int = 3600) -> str:
        file_path = self._base_dir / key
        return file_path.as_uri()

    async def list_keys(self, prefix: str = "") -> list[str]:
        loop = asyncio.get_event_loop()
        return await loop.run_in_executor(None, self._list_keys_sync, prefix)

    # --- Sync implementations ---

    def _get_sync(self, key: str) -> bytes:
        file_path = self._base_dir / key
        if not file_path.exists():
            raise FileNotFoundError(f"Object '{key}' not found in {self._base_dir}")
        return file_path.read_bytes()

    def _put_sync(self, key: str, data: bytes) -> None:
        file_path = self._base_dir / key
        file_path.parent.mkdir(parents=True, exist_ok=True)
        file_path.write_bytes(data)

    def _delete_sync(self, key: str) -> None:
        file_path = self._base_dir / key
        if file_path.exists():
            file_path.unlink()

    def _list_keys_sync(self, prefix: str) -> list[str]:
        keys: list[str] = []
        search_dir = self._base_dir / prefix if prefix else self._base_dir
        if not search_dir.exists():
            return keys
        for p in search_dir.rglob("*"):
            if p.is_file():
                keys.append(str(p.relative_to(self._base_dir)))
        return sorted(keys)
