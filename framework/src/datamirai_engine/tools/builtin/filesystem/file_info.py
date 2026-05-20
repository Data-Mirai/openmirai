"""filesystem/file_info — get file or directory metadata."""

from __future__ import annotations

import os
import stat as _stat
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec


class FileInfoTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/file_info",
        version="1.0.0",
        display_name="File Info",
        description="Returns metadata about a file or directory: size, permissions, modification time, type.",
        category="filesystem",
        icon="info",
        intents=[
            "obtener informacion de un archivo",
            "verificar si un archivo existe",
            "ver permisos y tamano de un archivo",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Path to inspect"),
        ],
        outputs=[
            ToolOutput(name="exists", type="boolean", description="Whether the path exists"),
            ToolOutput(name="type", type="string", description="'file', 'directory', 'symlink', or 'other'"),
            ToolOutput(name="size", type="number", description="Size in bytes"),
            ToolOutput(name="modified", type="number", description="Last modification timestamp"),
            ToolOutput(name="created", type="number", description="Creation timestamp"),
            ToolOutput(name="permissions", type="string", description="Unix permission string (e.g. 'rwxr-xr-x')"),
            ToolOutput(name="path", type="string", description="Resolved absolute path"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])

        if not os.path.exists(path):
            return {
                "exists": False,
                "type": "unknown",
                "size": 0,
                "modified": 0,
                "created": 0,
                "permissions": "",
                "path": path,
            }

        st = os.stat(path)
        mode = st.st_mode

        if os.path.islink(path):
            ftype = "symlink"
        elif os.path.isdir(path):
            ftype = "directory"
        elif os.path.isfile(path):
            ftype = "file"
        else:
            ftype = "other"

        perm = _stat.filemode(mode)[1:]  # strip leading 'd' or '-'

        return {
            "exists": True,
            "type": ftype,
            "size": st.st_size,
            "modified": st.st_mtime,
            "created": getattr(st, "st_birthtime", st.st_ctime),
            "permissions": perm,
            "path": path,
        }
