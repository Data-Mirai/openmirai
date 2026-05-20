"""git/log — show commit history."""

from __future__ import annotations

import asyncio
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


class GitLogTool(BaseTool):
    spec = ToolSpec(
        tool_type="git/log",
        version="1.0.0",
        display_name="Git Log",
        description="Shows the commit history with hash, author, date, and message.",
        category="git",
        icon="git-commit",
        intents=[
            "ver el historial de commits",
            "revisar los cambios recientes",
            "buscar un commit especifico",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=False, description="Repository path (defaults to cwd)"),
        ],
        outputs=[
            ToolOutput(name="commits", type="array", description="List of {hash, author, date, message} entries"),
            ToolOutput(name="count", type="number", description="Number of commits returned"),
            ToolOutput(name="raw", type="string", description="Raw log output"),
        ],
        config=[
            ConfigField(name="limit", type="number", default=20, description="Maximum commits to return"),
            ConfigField(name="oneline", type="boolean", default=False, description="Compact one-line format"),
            ConfigField(name="author", type="string", default="", description="Filter by author name"),
            ConfigField(name="file", type="string", default="", description="Show history for specific file"),
        ],
    )

    _SEP = "---COMMIT_SEP---"

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        cwd = os.path.abspath(inputs.get("path") or ".")
        limit = int(config.get("limit", 20))
        author = config.get("author", "")
        file_filter = config.get("file", "")

        fmt = f"%H{self._SEP}%an{self._SEP}%ai{self._SEP}%s"
        cmd = ["git", "log", f"-{limit}", f"--format={fmt}"]

        if author:
            cmd.append(f"--author={author}")
        if file_filter:
            cmd.extend(["--", file_filter])

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        stdout, stderr = await proc.communicate()

        if proc.returncode != 0:
            raise RuntimeError(f"git log failed: {stderr.decode().strip()}")

        raw = stdout.decode("utf-8", errors="replace")
        commits: list[dict[str, str]] = []

        for line in raw.strip().splitlines():
            parts = line.split(self._SEP, 3)
            if len(parts) == 4:
                commits.append({
                    "hash": parts[0],
                    "author": parts[1],
                    "date": parts[2],
                    "message": parts[3],
                })

        return {
            "commits": commits,
            "count": len(commits),
            "raw": raw,
        }
