"""Switch (Router) tool — routes to one of N cases based on value."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class SwitchTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/switch",
        version="1.0.0",
        display_name="Switch (Router)",
        description="Evaluates value against N cases. Routes to matched case",
        category="logic",
        icon="shuffle",
        intents=[
            "enrutar a diferentes caminos segun multiples opciones",
            "clasificacion con mas de 2 categorias",
            "dispatch a diferentes acciones segun tipo o categoria",
        ],
        inputs=[
            ToolInput(name="value", type="any", required=True),
        ],
        outputs=[
            ToolOutput(name="matched_case", type="any"),
            ToolOutput(name="case_index", type="number"),
        ],
        config=[
            ConfigField(name="cases", type="array", default=[]),
            ConfigField(name="default_case", type="any", default=None),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        value = inputs["value"]
        cases = config.get("cases", [])
        default = config.get("default_case")

        for i, case in enumerate(cases):
            if value == case:
                return {"matched_case": case, "case_index": i}

        return {"matched_case": default, "case_index": -1}
