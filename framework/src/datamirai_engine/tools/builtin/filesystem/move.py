"""filesystem/move — move or rename files and directories."""

from __future__ import annotations

import os
import shutil
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec


class MoveTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/move",
        version="1.0.0",
        display_name="Move / Rename",
        description="Moves or renames a file or directory.",
        category="filesystem",
        icon="file-symlink",
        intents=[
            "mover un archivo a otra ubicacion",
            "renombrar un archivo o carpeta",
            "reorganizar archivos del proyecto",
        ],
        inputs=[
            ToolInput(name="source", type="string", required=True, description="Current path"),
            ToolInput(name="destination", type="string", required=True, description="New path"),
        ],
        outputs=[
            ToolOutput(name="source", type="string", description="Original absolute path"),
            ToolOutput(name="destination", type="string", description="New absolute path"),
            ToolOutput(name="success", type="boolean", description="Whether the operation succeeded"),
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

        if os.path.exists(destination):
            raise FileExistsError(f"Destination already exists: {destination}")

        dest_parent = os.path.dirname(destination)
        if dest_parent and not os.path.isdir(dest_parent):
            os.makedirs(dest_parent, exist_ok=True)

        shutil.move(source, destination)

        return {
            "source": source,
            "destination": destination,
            "success": True,
        }
