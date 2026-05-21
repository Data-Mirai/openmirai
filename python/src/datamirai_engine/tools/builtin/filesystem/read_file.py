"""filesystem/read_file — read file contents with numbered lines."""

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


class ReadFileTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/read_file",
        version="1.0.0",
        display_name="Read File",
        description="Reads a file and returns its contents with numbered lines. Supports offset/limit for large files.",
        category="filesystem",
        icon="file-text",
        intents=[
            "leer el contenido de un archivo",
            "ver el codigo fuente de un archivo",
            "inspeccionar un archivo con numeros de linea",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Absolute or relative path to the file"),
        ],
        outputs=[
            ToolOutput(name="content", type="string", description="File content with numbered lines"),
            ToolOutput(name="lines", type="number", description="Total number of lines in the file"),
            ToolOutput(name="size", type="number", description="File size in bytes"),
            ToolOutput(name="path", type="string", description="Resolved absolute path"),
        ],
        config=[
            ConfigField(name="offset", type="number", default=0, description="Line number to start reading from (0-based)"),
            ConfigField(name="limit", type="number", default=2000, description="Maximum number of lines to read"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])

        if not os.path.isfile(path):
            raise FileNotFoundError(f"File not found: {path}")

        size = os.path.getsize(path)
        offset = int(config.get("offset", 0))
        limit = int(config.get("limit", 2000))

        with open(path, "r", encoding="utf-8", errors="replace") as f:
            all_lines = f.readlines()

        total_lines = len(all_lines)
        selected = all_lines[offset : offset + limit]

        numbered = []
        for i, line in enumerate(selected, start=offset + 1):
            numbered.append(f"{i:>6}\t{line.rstrip()}")

        return {
            "content": "\n".join(numbered),
            "lines": total_lines,
            "size": size,
            "path": path,
        }
