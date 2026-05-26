"""git/diff — show changes between commits, working tree, etc."""

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


class GitDiffTool(BaseTool):
    spec = ToolSpec(
        tool_type="git/diff",
        version="1.0.0",
        display_name="Git Diff",
        description="Shows differences in the working tree, staged changes, or between commits.",
        category="git",
        icon="git-compare",
        intents=[
            "ver los cambios en el codigo",
            "comparar diferencias entre versiones",
            "revisar que se va a commitear",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=False, description="Repository path (defaults to cwd)"),
        ],
        outputs=[
            ToolOutput(name="diff", type="string", description="Diff output"),
            ToolOutput(name="files_changed", type="number", description="Number of files changed"),
            ToolOutput(name="insertions", type="number", description="Lines added"),
            ToolOutput(name="deletions", type="number", description="Lines removed"),
        ],
        config=[
            ConfigField(name="staged", type="boolean", default=False, description="Show staged changes (--cached)"),
            ConfigField(name="ref", type="string", default="", description="Compare against ref (e.g. 'HEAD~3', 'main')"),
            ConfigField(name="file", type="string", default="", description="Limit diff to specific file"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        cwd = os.path.abspath(inputs.get("path") or ".")
        staged = config.get("staged", False)
        ref = config.get("ref", "")
        file_filter = config.get("file", "")

        cmd = ["git", "diff"]
        if staged:
            cmd.append("--cached")
        if ref:
            cmd.append(ref)
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
            raise RuntimeError(f"git diff failed: {stderr.decode().strip()}")

        diff_text = stdout.decode("utf-8", errors="replace")

        # Get stat summary
        stat_cmd = cmd + ["--stat"]
        proc2 = await asyncio.create_subprocess_exec(
            *stat_cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=cwd,
        )
        stat_out, _ = await proc2.communicate()
        stat_text = stat_out.decode("utf-8", errors="replace")

        files_changed = 0
        insertions = 0
        deletions = 0
        for line in stat_text.splitlines():
            if "file" in line and "changed" in line:
                parts = line.split(",")
                for part in parts:
                    part = part.strip()
                    if "file" in part:
                        try:
                            files_changed = int(part.split()[0])
                        except (ValueError, IndexError):
                            pass
                    elif "insertion" in part:
                        try:
                            insertions = int(part.split()[0])
                        except (ValueError, IndexError):
                            pass
                    elif "deletion" in part:
                        try:
                            deletions = int(part.split()[0])
                        except (ValueError, IndexError):
                            pass

        # Truncate if too large
        max_len = 50_000
        if len(diff_text) > max_len:
            diff_text = diff_text[:max_len] + f"\n... (truncated, {len(diff_text)} total chars)"

        return {
            "diff": diff_text,
            "files_changed": files_changed,
            "insertions": insertions,
            "deletions": deletions,
        }
