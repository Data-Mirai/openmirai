"""filesystem/write_file — create or overwrite a file."""

from __future__ import annotations

import os
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec


class WriteFileTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/write_file",
        version="1.0.0",
        display_name="Write File",
        description="Creates a new file or overwrites an existing one with the provided content.",
        category="filesystem",
        icon="file-plus",
        intents=[
            "crear un archivo nuevo",
            "sobreescribir el contenido de un archivo",
            "guardar texto en un archivo",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Absolute or relative path for the file"),
            ToolInput(name="content", type="string", required=True, description="Content to write"),
        ],
        outputs=[
            ToolOutput(name="path", type="string", description="Resolved absolute path"),
            ToolOutput(name="bytes_written", type="number", description="Number of bytes written"),
            ToolOutput(name="created", type="boolean", description="True if the file was newly created"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])
        content = inputs["content"]
        created = not os.path.exists(path)

        parent = os.path.dirname(path)
        if parent and not os.path.isdir(parent):
            os.makedirs(parent, exist_ok=True)

        with open(path, "w", encoding="utf-8") as f:
            bytes_written = f.write(content)

        return {
            "path": path,
            "bytes_written": bytes_written,
            "created": created,
        }
