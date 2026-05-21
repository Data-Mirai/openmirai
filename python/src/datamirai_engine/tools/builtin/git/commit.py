"""git/commit — create a git commit with safeguards."""

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


class GitCommitTool(BaseTool):
    spec = ToolSpec(
        tool_type="git/commit",
        version="1.0.0",
        display_name="Git Commit",
        description="Stages specified files and creates a commit. Never amends or force-pushes.",
        category="git",
        icon="git-commit",
        intents=[
            "crear un commit con los cambios actuales",
            "guardar el progreso en git",
            "commitear archivos especificos",
        ],
        inputs=[
            ToolInput(name="message", type="string", required=True, description="Commit message"),
            ToolInput(name="files", type="array", required=False, description="Files to stage (if empty, commits already-staged files)"),
        ],
        outputs=[
            ToolOutput(name="hash", type="string", description="Commit hash"),
            ToolOutput(name="message", type="string", description="Commit message used"),
            ToolOutput(name="files_committed", type="number", description="Number of files in the commit"),
            ToolOutput(name="success", type="boolean", description="Whether commit succeeded"),
        ],
        config=[
            ConfigField(name="path", type="string", default="", description="Repository path (defaults to cwd)"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        cwd = os.path.abspath(config.get("path") or ".")
        message = inputs["message"]
        files = inputs.get("files") or []

        if not message.strip():
            raise ValueError("Commit message cannot be empty")

        # Stage specific files if provided
        if files:
            proc = await asyncio.create_subprocess_exec(
                "git", "add", *files,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=cwd,
            )
            _, stderr = await proc.communicate()
            if proc.returncode != 0:
                raise RuntimeError(f"git add failed: {stderr.decode().strip()}")

        # Check there's something to commit
        proc = await asyncio.create_subprocess_exec(
            "git", "diff", "--cached", "--name-only",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        staged_out, _ = await proc.communicate()
        staged_files = [f for f in staged_out.decode().strip().splitlines() if f]

        if not staged_files:
            raise RuntimeError("Nothing to commit — no staged changes")

        # Create commit (NEVER amend, NEVER skip hooks)
        proc = await asyncio.create_subprocess_exec(
            "git", "commit", "-m", message,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        stdout, stderr = await proc.communicate()

        if proc.returncode != 0:
            raise RuntimeError(f"git commit failed: {stderr.decode().strip()}")

        # Get the commit hash
        proc = await asyncio.create_subprocess_exec(
            "git", "rev-parse", "HEAD",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        hash_out, _ = await proc.communicate()
        commit_hash = hash_out.decode().strip()

        return {
            "hash": commit_hash,
            "message": message,
            "files_committed": len(staged_files),
            "success": True,
        }
