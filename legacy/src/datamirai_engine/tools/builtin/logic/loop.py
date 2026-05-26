"""Loop tool — iterates over items, outputs current index/item and done flag."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec
from datamirai_engine.core.context import ExecutionContext


class LoopTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/loop",
        version="1.0.0",
        display_name="Loop",
        description="Iterates over items. Outputs current item and done flag",
        category="logic",
        icon="repeat",
        intents=[
            "procesar una lista de items uno por uno",
            "iterar sobre resultados para aplicar una accion a cada uno",
            "batch processing: repetir un paso para cada elemento",
        ],
        inputs=[
            ToolInput(name="items", type="array", required=True),
            ToolInput(name="current_index", type="number", required=False, default=0),
        ],
        outputs=[
            ToolOutput(name="current_item", type="any"),
            ToolOutput(name="current_index", type="number"),
            ToolOutput(name="done", type="boolean"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        items = inputs["items"]
        idx = inputs.get("current_index", 0)
        next_idx = idx + 1

        if next_idx >= len(items):
            return {
                "current_item": items[idx] if idx < len(items) else None,
                "current_index": next_idx,
                "done": True,
            }

        return {
            "current_item": items[next_idx],
            "current_index": next_idx,
            "done": False,
        }
