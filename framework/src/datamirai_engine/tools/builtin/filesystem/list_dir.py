"""filesystem/list_dir — list directory contents."""

from __future__ import annotations

import os
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class ListDirTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/list_dir",
        version="1.0.0",
        display_name="List Directory",
        description="Lists the contents of a directory with file type, size, and modification time.",
        category="filesystem",
        icon="folder-open",
        intents=[
            "ver que archivos hay en un directorio",
            "listar el contenido de una carpeta",
            "explorar la estructura de un proyecto",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Directory path to list"),
        ],
        outputs=[
            ToolOutput(name="entries", type="array", description="List of {name, type, size, modified} entries"),
            ToolOutput(name="count", type="number", description="Number of entries"),
        ],
        config=[
            ConfigField(name="show_hidden", type="boolean", default=False, description="Include hidden files (starting with '.')"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])
        show_hidden = config.get("show_hidden", False)

        if not os.path.isdir(path):
            raise NotADirectoryError(f"Not a directory: {path}")

        entries: list[dict[str, Any]] = []
        for name in sorted(os.listdir(path)):
            if not show_hidden and name.startswith("."):
                continue
            full = os.path.join(path, name)
            try:
                stat = os.stat(full)
                entries.append({
                    "name": name,
                    "type": "directory" if os.path.isdir(full) else "file",
                    "size": stat.st_size,
                    "modified": stat.st_mtime,
                })
            except OSError:
                entries.append({"name": name, "type": "unknown", "size": 0, "modified": 0})

        return {
            "entries": entries,
            "count": len(entries),
        }
