"""SharedState — Global dict where each block saves output indexed by node ID."""

from __future__ import annotations

import copy
import threading
from typing import Any

_SENTINEL = object()


class SharedState:
    """Write-once, read-many state container for graph execution.

    Each node writes its output exactly once. Subsequent writes raise ValueError.
    Thread-safe via lock (prepares for future concurrent use).
    """

    def __init__(self) -> None:
        self._data: dict[str, dict[str, Any]] = {}
        self._lock = threading.Lock()

    def set(self, node_id: str, output: dict[str, Any], *, overwrite: bool = False) -> None:
        """Store output for a node. Raises ValueError if already set unless overwrite=True."""
        with self._lock:
            if node_id in self._data and not overwrite:
                raise ValueError(f"Output for node '{node_id}' is already set (immutable)")
            self._data[node_id] = copy.deepcopy(output)

    def get(self, node_id: str) -> dict[str, Any]:
        """Get output for a node. Raises KeyError if not found."""
        with self._lock:
            if node_id not in self._data:
                raise KeyError(f"No output for node '{node_id}'")
            return copy.deepcopy(self._data[node_id])

    def get_field(self, node_id: str, field: str, *, default: Any = _SENTINEL) -> Any:
        """Get specific field from a node's output."""
        output = self.get(node_id)
        if field not in output:
            if default is not _SENTINEL:
                return default
            raise KeyError(f"Field '{field}' not found in output of node '{node_id}'")
        return output[field]

    def __contains__(self, node_id: str) -> bool:
        with self._lock:
            return node_id in self._data

    def __len__(self) -> int:
        with self._lock:
            return len(self._data)

    def keys(self) -> list[str]:
        """Return list of node IDs that have outputs."""
        with self._lock:
            return list(self._data.keys())

    def snapshot(self) -> dict[str, dict[str, Any]]:
        """Return deep copy of entire state."""
        with self._lock:
            return copy.deepcopy(self._data)
