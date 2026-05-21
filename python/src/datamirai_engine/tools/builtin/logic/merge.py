"""Merge tool — convergence point for multiple paths."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolOutput, ToolSpec
from datamirai_engine.core.context import ExecutionContext


class MergeTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/merge",
        version="1.0.0",
        display_name="Merge",
        description="Convergence point. Passes through data from the path that arrived",
        category="logic",
        icon="git-merge",
        intents=[
            "reunir datos de multiples caminos paralelos en un solo punto",
            "convergencia despues de un branch condicional",
        ],
        outputs=[
            ToolOutput(name="data", type="any"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        return {"data": inputs.get("data")}
