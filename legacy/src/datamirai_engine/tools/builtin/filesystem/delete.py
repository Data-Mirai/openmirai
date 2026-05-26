"""filesystem/delete — delete files or directories with safety checks."""

from __future__ import annotations

import os
import shutil
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class DeleteTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/delete",
        version="1.0.0",
        display_name="Delete",
        description="Deletes a file or directory. Directories require force=true. Protected paths are blocked.",
        category="filesystem",
        icon="trash",
        intents=[
            "eliminar un archivo",
            "borrar una carpeta",
            "limpiar archivos temporales",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Path to delete"),
        ],
        outputs=[
            ToolOutput(name="path", type="string", description="Deleted absolute path"),
            ToolOutput(name="success", type="boolean", description="Whether deletion succeeded"),
            ToolOutput(name="type", type="string", description="'file' or 'directory'"),
        ],
        config=[
            ConfigField(name="force", type="boolean", default=False, description="Required for deleting directories"),
        ],
    )

    _PROTECTED = frozenset({"/", "/bin", "/usr", "/etc", "/var", "/tmp", "/home", "/root", "/System", "/Library"})

    _SAFE_DIR_NAMES = frozenset({
        "node_modules", "dist", "build", ".next", ".cache",
        "tmp", "test-results", "__pycache__", ".tox", "egg-info",
    })

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])
        force = config.get("force", False)

        if path in self._PROTECTED or os.path.dirname(path) == path:
            raise PermissionError(f"Refusing to delete protected path: {path}")

        if not os.path.exists(path):
            raise FileNotFoundError(f"Path not found: {path}")

        if os.path.isdir(path):
            basename = os.path.basename(path)
            if not force:
                raise PermissionError(
                    f"Cannot delete directory '{basename}' without force=true. "
                    "This is a safety measure."
                )
            if basename not in self._SAFE_DIR_NAMES and not force:
                raise PermissionError(
                    f"Directory '{basename}' is not in the safe-delete list. "
                    "Set force=true to confirm deletion."
                )
            shutil.rmtree(path)
            return {"path": path, "success": True, "type": "directory"}

        os.remove(path)
        return {"path": path, "success": True, "type": "file"}
