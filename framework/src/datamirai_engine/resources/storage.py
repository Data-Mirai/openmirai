"""Storage Resource implementations — InMemory for dev/testing."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class _StoredObject:
    data: bytes
    content_type: str | None = None


class InMemoryStorageResource:
    """In-memory S3-compatible object storage."""

    def __init__(self) -> None:
        self._objects: dict[str, _StoredObject] = {}

    async def get(self, key: str) -> bytes:
        if key not in self._objects:
            raise FileNotFoundError(f"Object '{key}' not found")
        return self._objects[key].data

    async def put(
        self, key: str, data: bytes, *, content_type: str | None = None
    ) -> None:
        self._objects[key] = _StoredObject(data=data, content_type=content_type)

    async def delete(self, key: str) -> None:
        self._objects.pop(key, None)

    async def presign(self, key: str, *, expires_in: int = 3600) -> str:
        return f"mem://{key}?expires={expires_in}"

    async def list_keys(self, prefix: str = "") -> list[str]:
        return [k for k in self._objects if k.startswith(prefix)]
