"""git/status — show working tree status."""

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


class GitStatusTool(BaseTool):
    spec = ToolSpec(
        tool_type="git/status",
        version="1.0.0",
        display_name="Git Status",
        description="Shows the working tree status: staged, modified, and untracked files.",
        category="git",
        icon="git-branch",
        intents=[
            "ver el estado del repositorio git",
            "ver que archivos han cambiado",
            "verificar que hay para commitear",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=False, description="Repository path (defaults to cwd)"),
        ],
        outputs=[
            ToolOutput(name="branch", type="string", description="Current branch name"),
            ToolOutput(name="staged", type="array", description="Staged files"),
            ToolOutput(name="modified", type="array", description="Modified but unstaged files"),
            ToolOutput(name="untracked", type="array", description="Untracked files"),
            ToolOutput(name="clean", type="boolean", description="True if working tree is clean"),
            ToolOutput(name="raw", type="string", description="Raw git status output"),
        ],
        config=[
            ConfigField(name="short", type="boolean", default=False, description="Use short format output"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        cwd = os.path.abspath(inputs.get("path") or ".")

        # Get branch
        proc = await asyncio.create_subprocess_exec(
            "git", "branch", "--show-current",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        branch_out, _ = await proc.communicate()
        branch = branch_out.decode().strip() or "HEAD (detached)"

        # Get porcelain status
        proc = await asyncio.create_subprocess_exec(
            "git", "status", "--porcelain=v1",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        status_out, status_err = await proc.communicate()

        if proc.returncode != 0:
            raise RuntimeError(f"git status failed: {status_err.decode().strip()}")

        raw = status_out.decode("utf-8", errors="replace")
        staged: list[str] = []
        modified: list[str] = []
        untracked: list[str] = []

        for line in raw.splitlines():
            if len(line) < 4:
                continue
            index_status = line[0]
            worktree_status = line[1]
            filepath = line[3:]

            if index_status in "MADRC":
                staged.append(filepath)
            if worktree_status in "MD":
                modified.append(filepath)
            if index_status == "?" and worktree_status == "?":
                untracked.append(filepath)

        return {
            "branch": branch,
            "staged": staged,
            "modified": modified,
            "untracked": untracked,
            "clean": len(staged) == 0 and len(modified) == 0 and len(untracked) == 0,
            "raw": raw,
        }
