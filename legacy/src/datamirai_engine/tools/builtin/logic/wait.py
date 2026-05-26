"""Wait/Delay tool — pauses execution for configured time."""

from __future__ import annotations

import asyncio
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class WaitTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/wait",
        version="1.0.0",
        display_name="Wait / Delay",
        description="Pauses execution for configured time",
        category="logic",
        icon="clock",
        intents=[
            "esperar antes de continuar (rate limiting, cooldown)",
            "pausar entre pasos para evitar sobrecarga",
        ],
        outputs=[
            ToolOutput(name="waited_seconds", type="number"),
        ],
        config=[
            ConfigField(name="delay_seconds", type="number", default=0),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        delay = config.get("delay_seconds", 0)
        if delay > 0:
            await asyncio.sleep(delay)
        return {"waited_seconds": delay}
