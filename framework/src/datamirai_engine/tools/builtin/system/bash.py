"""system/bash — execute shell commands with safety checks."""

from __future__ import annotations

import asyncio
import os
import re
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class BashTool(BaseTool):
    spec = ToolSpec(
        tool_type="system/bash",
        version="1.0.0",
        display_name="Bash (Shell)",
        description="Executes a shell command and returns stdout, stderr, and exit code. Dangerous commands are blocked.",
        category="system",
        icon="terminal",
        intents=[
            "ejecutar un comando en la terminal",
            "correr tests o scripts",
            "instalar dependencias",
            "compilar o construir el proyecto",
        ],
        inputs=[
            ToolInput(name="command", type="string", required=True, description="Shell command to execute"),
        ],
        outputs=[
            ToolOutput(name="stdout", type="string", description="Standard output"),
            ToolOutput(name="stderr", type="string", description="Standard error"),
            ToolOutput(name="exit_code", type="number", description="Process exit code (0 = success)"),
            ToolOutput(name="timed_out", type="boolean", description="Whether the command timed out"),
        ],
        config=[
            ConfigField(name="timeout", type="number", default=120, description="Timeout in seconds (max 600)"),
            ConfigField(name="cwd", type="string", default="", description="Working directory (defaults to current)"),
        ],
    )

    _BLOCKED_PATTERNS: list[re.Pattern[str]] = [
        re.compile(r"\brm\s+-rf\s+/\s*$"),         # rm -rf /
        re.compile(r"\brm\s+-rf\s+/[a-z]+\s*$"),   # rm -rf /usr etc.
        re.compile(r":\(\)\s*\{\s*:\|:\s*&\s*\}"),  # fork bomb
        re.compile(r"\bmkfs\b"),                     # format disk
        re.compile(r"\bdd\s+.*of=/dev/"),            # overwrite device
        re.compile(r">\s*/dev/sd[a-z]"),             # write to disk device
        re.compile(r"\bcurl\b.*\|\s*(ba)?sh"),       # curl | bash
        re.compile(r"\bwget\b.*\|\s*(ba)?sh"),       # wget | bash
    ]

    _MAX_OUTPUT = 30_000  # chars

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        command = inputs["command"]
        timeout = min(int(config.get("timeout", 120)), 600)
        cwd = config.get("cwd", "") or None

        # Safety: block dangerous commands
        for pat in self._BLOCKED_PATTERNS:
            if pat.search(command):
                raise PermissionError(f"Blocked dangerous command: {command}")

        if cwd:
            cwd = os.path.abspath(cwd)
            if not os.path.isdir(cwd):
                raise NotADirectoryError(f"Working directory not found: {cwd}")

        timed_out = False
        try:
            proc = await asyncio.create_subprocess_shell(
                command,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=cwd,
            )
            stdout_bytes, stderr_bytes = await asyncio.wait_for(
                proc.communicate(), timeout=timeout
            )
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
            timed_out = True
            stdout_bytes = b""
            stderr_bytes = f"Command timed out after {timeout}s".encode()

        stdout = stdout_bytes.decode("utf-8", errors="replace")
        stderr = stderr_bytes.decode("utf-8", errors="replace")

        # Truncate large outputs
        if len(stdout) > self._MAX_OUTPUT:
            stdout = stdout[: self._MAX_OUTPUT] + f"\n... (truncated, {len(stdout)} total chars)"
        if len(stderr) > self._MAX_OUTPUT:
            stderr = stderr[: self._MAX_OUTPUT] + f"\n... (truncated, {len(stderr)} total chars)"

        return {
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": proc.returncode or 0,
            "timed_out": timed_out,
        }
