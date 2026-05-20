"""filesystem/tree — directory tree view."""

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


class TreeTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/tree",
        version="1.0.0",
        display_name="Directory Tree",
        description="Generates a visual tree representation of a directory structure.",
        category="filesystem",
        icon="list-tree",
        intents=[
            "ver la estructura completa de un proyecto",
            "generar un arbol de directorios",
            "visualizar la jerarquia de carpetas y archivos",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Root directory"),
        ],
        outputs=[
            ToolOutput(name="tree", type="string", description="Visual tree representation"),
            ToolOutput(name="file_count", type="number", description="Total files found"),
            ToolOutput(name="dir_count", type="number", description="Total directories found"),
        ],
        config=[
            ConfigField(name="max_depth", type="number", default=4, description="Maximum depth to traverse"),
            ConfigField(name="show_hidden", type="boolean", default=False, description="Include hidden entries"),
            ConfigField(name="pattern", type="string", default="", description="Filter files by glob pattern"),
        ],
    )

    _SKIP_DIRS = frozenset({
        "node_modules", ".git", "__pycache__", ".venv", "venv",
        "dist", "build", ".next", ".cache", ".tox",
    })

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        root = os.path.abspath(inputs["path"])
        max_depth = int(config.get("max_depth", 4))
        show_hidden = config.get("show_hidden", False)
        pattern = config.get("pattern", "")

        if not os.path.isdir(root):
            raise NotADirectoryError(f"Not a directory: {root}")

        import fnmatch

        lines: list[str] = [os.path.basename(root) + "/"]
        file_count = 0
        dir_count = 0

        def _walk(dir_path: str, prefix: str, depth: int) -> None:
            nonlocal file_count, dir_count
            if depth >= max_depth:
                return

            try:
                entries = sorted(os.listdir(dir_path))
            except PermissionError:
                return

            if not show_hidden:
                entries = [e for e in entries if not e.startswith(".")]

            entries = [e for e in entries if e not in self._SKIP_DIRS or not os.path.isdir(os.path.join(dir_path, e))]

            for i, name in enumerate(entries):
                full = os.path.join(dir_path, name)
                is_last = i == len(entries) - 1
                connector = "└── " if is_last else "├── "
                child_prefix = prefix + ("    " if is_last else "│   ")

                if os.path.isdir(full):
                    if name in self._SKIP_DIRS:
                        continue
                    dir_count += 1
                    lines.append(f"{prefix}{connector}{name}/")
                    _walk(full, child_prefix, depth + 1)
                else:
                    if pattern and not fnmatch.fnmatch(name, pattern):
                        continue
                    file_count += 1
                    lines.append(f"{prefix}{connector}{name}")

        _walk(root, "", 0)

        return {
            "tree": "\n".join(lines),
            "file_count": file_count,
            "dir_count": dir_count,
        }
