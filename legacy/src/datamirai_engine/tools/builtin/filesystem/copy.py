"""filesystem/copy — copy files or directories."""

from __future__ import annotations

import os
import shutil
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec


class CopyTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/copy",
        version="1.0.0",
        display_name="Copy",
        description="Copies a file or directory to a new location.",
        category="filesystem",
        icon="copy",
        intents=[
            "copiar un archivo a otra ubicacion",
            "duplicar un archivo o carpeta",
            "crear una copia de respaldo",
        ],
        inputs=[
            ToolInput(name="source", type="string", required=True, description="Path to copy from"),
            ToolInput(name="destination", type="string", required=True, description="Path to copy to"),
        ],
        outputs=[
            ToolOutput(name="source", type="string", description="Source absolute path"),
            ToolOutput(name="destination", type="string", description="Destination absolute path"),
            ToolOutput(name="success", type="boolean", description="Whether the copy succeeded"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        source = os.path.abspath(inputs["source"])
        destination = os.path.abspath(inputs["destination"])

        if not os.path.exists(source):
            raise FileNotFoundError(f"Source not found: {source}")

        dest_parent = os.path.dirname(destination)
        if dest_parent and not os.path.isdir(dest_parent):
            os.makedirs(dest_parent, exist_ok=True)

        if os.path.isdir(source):
            shutil.copytree(source, destination)
        else:
            shutil.copy2(source, destination)

        return {
            "source": source,
            "destination": destination,
            "success": True,
        }
