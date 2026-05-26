"""system/process_list — list running processes."""

from __future__ import annotations

import asyncio
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolOutput,
    ToolSpec,
)


class ProcessListTool(BaseTool):
    spec = ToolSpec(
        tool_type="system/process_list",
        version="1.0.0",
        display_name="Process List",
        description="Lists running processes with PID, name, CPU%, and memory usage.",
        category="system",
        icon="activity",
        intents=[
            "ver que procesos estan corriendo",
            "identificar procesos que consumen mucho recurso",
            "verificar si un servicio esta activo",
        ],
        outputs=[
            ToolOutput(name="processes", type="array", description="List of {pid, name, cpu, memory} entries"),
            ToolOutput(name="count", type="number", description="Number of processes"),
        ],
        config=[
            ConfigField(name="filter", type="string", default="", description="Filter by process name (substring match)"),
            ConfigField(name="limit", type="number", default=50, description="Maximum processes to return"),
            ConfigField(name="sort_by", type="select", default="cpu", options=["cpu", "memory", "pid", "name"], description="Sort field"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        name_filter = config.get("filter", "").lower()
        limit = int(config.get("limit", 50))
        sort_by = config.get("sort_by", "cpu")

        # Use ps command for cross-platform compatibility
        proc = await asyncio.create_subprocess_shell(
            "ps -eo pid,pcpu,pmem,comm --no-headers 2>/dev/null || ps -eo pid,pcpu,pmem,comm",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, _ = await proc.communicate()
        output = stdout.decode("utf-8", errors="replace")

        processes: list[dict[str, Any]] = []
        for line in output.strip().splitlines():
            parts = line.split(None, 3)
            if len(parts) < 4:
                continue
            try:
                pid = int(parts[0])
                cpu = float(parts[1])
                memory = float(parts[2])
                name = parts[3].strip()
            except (ValueError, IndexError):
                continue

            if name_filter and name_filter not in name.lower():
                continue

            processes.append({
                "pid": pid,
                "name": name,
                "cpu": cpu,
                "memory": memory,
            })

        sort_keys = {
            "cpu": lambda p: p["cpu"],
            "memory": lambda p: p["memory"],
            "pid": lambda p: p["pid"],
            "name": lambda p: p["name"].lower(),
        }
        processes.sort(key=sort_keys.get(sort_by, sort_keys["cpu"]), reverse=sort_by in ("cpu", "memory"))
        processes = processes[:limit]

        return {
            "processes": processes,
            "count": len(processes),
        }
