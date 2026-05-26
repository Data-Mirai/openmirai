"""filesystem/mkdir — create directories."""

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


class MkdirTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/mkdir",
        version="1.0.0",
        display_name="Make Directory",
        description="Creates a new directory. Optionally creates parent directories.",
        category="filesystem",
        icon="folder-plus",
        intents=[
            "crear un directorio nuevo",
            "crear una carpeta con subdirectorios",
            "preparar la estructura de un proyecto",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Directory path to create"),
        ],
        outputs=[
            ToolOutput(name="path", type="string", description="Created absolute path"),
            ToolOutput(name="created", type="boolean", description="True if newly created, False if already existed"),
        ],
        config=[
            ConfigField(name="parents", type="boolean", default=True, description="Create parent directories if needed"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])
        parents = config.get("parents", True)
        existed = os.path.isdir(path)

        if parents:
            os.makedirs(path, exist_ok=True)
        else:
            os.mkdir(path)

        return {
            "path": path,
            "created": not existed,
        }
