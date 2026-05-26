"""filesystem/glob — find files by glob pattern."""

from __future__ import annotations

import glob as _glob
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


class GlobFilesTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/glob",
        version="1.0.0",
        display_name="Glob (Find Files)",
        description="Finds files matching a glob pattern (e.g. '**/*.py', 'src/**/*.ts'). Returns paths sorted by modification time (newest first).",
        category="filesystem",
        icon="folder-search",
        intents=[
            "buscar archivos por patron de nombre",
            "encontrar todos los archivos de un tipo",
            "listar archivos que coincidan con un glob",
        ],
        inputs=[
            ToolInput(name="pattern", type="string", required=True, description="Glob pattern (e.g. '**/*.py')"),
        ],
        outputs=[
            ToolOutput(name="files", type="array", description="List of matching file paths"),
            ToolOutput(name="count", type="number", description="Number of matches"),
        ],
        config=[
            ConfigField(name="path", type="string", default=".", description="Base directory to search in"),
            ConfigField(name="max_results", type="number", default=200, description="Maximum number of results"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        pattern = inputs["pattern"]
        base = os.path.abspath(config.get("path", "."))
        max_results = int(config.get("max_results", 200))

        full_pattern = os.path.join(base, pattern)
        matches = _glob.glob(full_pattern, recursive=True)

        # Filter to files only, sort by mtime descending
        files = [f for f in matches if os.path.isfile(f)]
        files.sort(key=lambda f: os.path.getmtime(f), reverse=True)
        files = files[:max_results]

        return {
            "files": files,
            "count": len(files),
        }
