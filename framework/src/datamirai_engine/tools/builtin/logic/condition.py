"""Condition (IF-ELSE) tool — evaluates condition, outputs {result: bool}."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

_OPERATORS = {
    "eq": lambda a, b: a == b,
    "neq": lambda a, b: a != b,
    "gt": lambda a, b: a > b,
    "lt": lambda a, b: a < b,
    "gte": lambda a, b: a >= b,
    "lte": lambda a, b: a <= b,
    "in": lambda a, b: a in b,
    "contains": lambda a, b: b in a,
}


class ConditionTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/condition",
        version="1.0.0",
        display_name="Condition (IF-ELSE)",
        description="Evaluates condition against value. Outputs {result: true/false}",
        category="logic",
        icon="git-branch",
        intents=[
            "tomar caminos diferentes segun una condicion (if/else)",
            "filtrar o clasificar datos antes de procesarlos",
            "decidir entre dos acciones basado en un valor",
        ],
        inputs=[
            ToolInput(name="value", type="any", required=True, description="Value to evaluate"),
        ],
        outputs=[
            ToolOutput(name="result", type="boolean", description="Condition result"),
        ],
        config=[
            ConfigField(
                name="operator",
                type="select",
                default="eq",
                options=list(_OPERATORS.keys()),
            ),
            ConfigField(name="compare_to", type="any", default=None),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        value = inputs["value"]
        operator = config.get("operator", "eq")
        compare_to = config.get("compare_to")

        op_fn = _OPERATORS.get(operator)
        if op_fn is None:
            raise ValueError(f"Unknown operator: {operator}")

        return {"result": op_fn(value, compare_to)}
