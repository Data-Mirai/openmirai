"""Heartbeat Trigger — periodic condition-based trigger for graph execution.

Evaluates a condition at a configurable interval. If the condition is met,
the graph runs. Supports quiet hours and pluggable condition evaluators.
"""

from __future__ import annotations

import ast
import operator
import re
from datetime import datetime, timezone
from typing import Any, Callable

from datamirai_engine.tools.base import BaseTool, ConfigField, ToolInput, ToolOutput, ToolSpec


# ---------------------------------------------------------------------------
# Condition evaluator registry
# ---------------------------------------------------------------------------

_EVALUATORS: dict[str, Callable[[str, dict[str, Any]], bool]] = {}


def register_evaluator(name: str):
    """Decorator to register a condition evaluator function."""
    def decorator(fn: Callable[[str, dict[str, Any]], bool]):
        _EVALUATORS[name] = fn
        return fn
    return decorator


@register_evaluator("always_true")
def _always_true(config_str: str, inputs: dict[str, Any]) -> bool:
    return True


@register_evaluator("always_false")
def _always_false(config_str: str, inputs: dict[str, Any]) -> bool:
    return False


# Safe operators for expression evaluation
_SAFE_OPS = {
    ast.Eq: operator.eq,
    ast.NotEq: operator.ne,
    ast.Lt: operator.lt,
    ast.LtE: operator.le,
    ast.Gt: operator.gt,
    ast.GtE: operator.ge,
    ast.Add: operator.add,
    ast.Sub: operator.sub,
    ast.Mult: operator.mul,
    ast.Div: operator.truediv,
    ast.Mod: operator.mod,
    ast.And: lambda a, b: a and b,
    ast.Or: lambda a, b: a or b,
    ast.Not: operator.not_,
    ast.Is: operator.is_,
    ast.IsNot: operator.is_not,
    ast.In: lambda a, b: a in b,
    ast.NotIn: lambda a, b: a not in b,
}


def _safe_eval_node(node: ast.AST, variables: dict[str, Any]) -> Any:
    """Recursively evaluate an AST node using only safe operations."""
    if isinstance(node, ast.Expression):
        return _safe_eval_node(node.body, variables)
    if isinstance(node, ast.Constant):
        return node.value
    if isinstance(node, ast.Name):
        if node.id in variables:
            return variables[node.id]
        if node.id == "True":
            return True
        if node.id == "False":
            return False
        if node.id == "None":
            return None
        raise NameError(f"Name '{node.id}' is not defined")
    if isinstance(node, ast.BoolOp):
        op_fn = _SAFE_OPS.get(type(node.op))
        if op_fn is None:
            raise ValueError(f"Unsupported boolean operator: {type(node.op).__name__}")
        result = _safe_eval_node(node.values[0], variables)
        for val in node.values[1:]:
            result = op_fn(result, _safe_eval_node(val, variables))
        return result
    if isinstance(node, ast.UnaryOp):
        if isinstance(node.op, ast.Not):
            return not _safe_eval_node(node.operand, variables)
        if isinstance(node.op, ast.USub):
            return -_safe_eval_node(node.operand, variables)
        raise ValueError(f"Unsupported unary operator: {type(node.op).__name__}")
    if isinstance(node, ast.BinOp):
        op_fn = _SAFE_OPS.get(type(node.op))
        if op_fn is None:
            raise ValueError(f"Unsupported binary operator: {type(node.op).__name__}")
        left = _safe_eval_node(node.left, variables)
        right = _safe_eval_node(node.right, variables)
        return op_fn(left, right)
    if isinstance(node, ast.Compare):
        left = _safe_eval_node(node.left, variables)
        for op, comparator in zip(node.ops, node.comparators):
            op_fn = _SAFE_OPS.get(type(op))
            if op_fn is None:
                raise ValueError(f"Unsupported comparison: {type(op).__name__}")
            right = _safe_eval_node(comparator, variables)
            if not op_fn(left, right):
                return False
            left = right
        return True
    if isinstance(node, ast.IfExp):
        test = _safe_eval_node(node.test, variables)
        return _safe_eval_node(node.body if test else node.orelse, variables)
    if isinstance(node, ast.Attribute):
        value = _safe_eval_node(node.value, variables)
        return getattr(value, node.attr)
    if isinstance(node, ast.Subscript):
        value = _safe_eval_node(node.value, variables)
        key = _safe_eval_node(node.slice, variables)
        return value[key]
    if isinstance(node, (ast.List, ast.Tuple)):
        return [_safe_eval_node(e, variables) for e in node.elts]
    raise ValueError(f"Unsupported expression node: {type(node).__name__}")


def safe_eval_expression(expression: str, variables: dict[str, Any] | None = None) -> Any:
    """Evaluate a simple Python expression safely (no import/exec/eval/call).

    Supports: comparisons, arithmetic, boolean logic, attribute access,
    subscript access, and literal values.
    """
    # Block dangerous patterns
    forbidden = {"import", "__", "exec", "eval", "compile", "globals", "locals", "open", "getattr"}
    expr_lower = expression.lower()
    for word in forbidden:
        if word in expr_lower:
            raise ValueError(f"Forbidden keyword in expression: {word}")

    # Block function calls (parentheses that aren't part of tuples/grouping)
    # We do this by parsing the AST and checking for Call nodes
    try:
        tree = ast.parse(expression, mode="eval")
    except SyntaxError as e:
        raise ValueError(f"Invalid expression syntax: {e}")

    # Walk tree to block Call nodes
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            raise ValueError("Function calls are not allowed in condition expressions")

    return _safe_eval_node(tree, variables or {})


@register_evaluator("custom_expression")
def _custom_expression(config_str: str, inputs: dict[str, Any]) -> bool:
    """Evaluate a custom Python expression safely.

    Available variables: `condition_result` from inputs, plus `True`, `False`, `None`.
    """
    if not config_str.strip():
        return True  # Empty expression = always true

    variables = {"condition_result": inputs.get("condition_result")}
    result = safe_eval_expression(config_str, variables)
    return bool(result)


class ConditionEvaluator:
    """Registry-based condition evaluator for heartbeat triggers."""

    @staticmethod
    def evaluate(condition_type: str, condition_config: str, inputs: dict[str, Any]) -> bool:
        """Evaluate a condition using the registered evaluator."""
        evaluator = _EVALUATORS.get(condition_type)
        if evaluator is None:
            raise ValueError(
                f"Unknown condition type: '{condition_type}'. "
                f"Available: {', '.join(_EVALUATORS.keys())}"
            )
        return evaluator(condition_config, inputs)

    @staticmethod
    def list_evaluators() -> list[str]:
        """Return list of registered evaluator names."""
        return list(_EVALUATORS.keys())


def _parse_interval(interval: str) -> int:
    """Parse interval string like '30m', '1h', '2d' into seconds."""
    match = re.match(r"^(\d+)(s|m|h|d)$", interval.strip().lower())
    if not match:
        raise ValueError(f"Invalid interval format: '{interval}'. Use format like '30m', '1h', '2d'.")
    value = int(match.group(1))
    unit = match.group(2)
    multipliers = {"s": 1, "m": 60, "h": 3600, "d": 86400}
    return value * multipliers[unit]


def _is_in_quiet_hours(now: datetime, start: str, end: str) -> bool:
    """Check if current time falls within quiet hours.

    Args:
        now: Current datetime (timezone-aware).
        start: Quiet hours start in HH:MM format.
        end: Quiet hours end in HH:MM format.
    """
    if not start or not end:
        return False

    try:
        start_h, start_m = map(int, start.split(":"))
        end_h, end_m = map(int, end.split(":"))
    except (ValueError, AttributeError):
        return False

    current_minutes = now.hour * 60 + now.minute
    start_minutes = start_h * 60 + start_m
    end_minutes = end_h * 60 + end_m

    if start_minutes <= end_minutes:
        # Same day range: e.g., 22:00 - 23:00
        return start_minutes <= current_minutes < end_minutes
    else:
        # Crosses midnight: e.g., 22:00 - 06:00
        return current_minutes >= start_minutes or current_minutes < end_minutes


class HeartbeatTriggerTool(BaseTool):
    """Heartbeat trigger — evaluates a condition periodically.

    When the condition evaluates to true, the graph executes.
    Supports quiet hours to suppress evaluations during certain times.
    """

    spec = ToolSpec(
        tool_type="trigger/heartbeat",
        version="1.0.0",
        display_name="Heartbeat",
        description="Periodic condition-based trigger",
        category="trigger",
        icon="heartbeat",
        intents=[
            "el agente evalua una condicion cada cierto intervalo de tiempo",
            "monitoreo condicional: ejecutar solo si se cumple una condicion",
            "trigger inteligente que combina periodicidad con logica de evaluacion",
        ],
        inputs=[
            ToolInput(
                name="condition_result",
                type="any",
                required=False,
                default=None,
                description="External condition result to evaluate",
            ),
        ],
        outputs=[
            ToolOutput(name="triggered", type="boolean", description="Whether the condition was met"),
            ToolOutput(name="condition_result", type="any", description="The condition result value"),
            ToolOutput(name="evaluated_at", type="string", description="ISO timestamp of evaluation"),
        ],
        config=[
            ConfigField(
                name="interval",
                type="string",
                default="30m",
                description="Evaluation interval (e.g., 30m, 1h, 2d)",
            ),
            ConfigField(
                name="condition_type",
                type="select",
                default="custom_expression",
                description="Type of condition evaluator",
                options=["custom_expression", "always_true", "always_false"],
            ),
            ConfigField(
                name="condition_config",
                type="string",
                default="",
                description="Condition configuration (expression for custom_expression)",
            ),
            ConfigField(
                name="quiet_hours_start",
                type="string",
                default="",
                description="Quiet hours start (HH:MM, 24h format)",
            ),
            ConfigField(
                name="quiet_hours_end",
                type="string",
                default="",
                description="Quiet hours end (HH:MM, 24h format)",
            ),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: Any,
    ) -> dict[str, Any]:
        now = datetime.now(timezone.utc)
        evaluated_at = now.isoformat()

        # Check quiet hours
        quiet_start = config.get("quiet_hours_start", "")
        quiet_end = config.get("quiet_hours_end", "")
        if _is_in_quiet_hours(now, quiet_start, quiet_end):
            return {
                "triggered": False,
                "condition_result": inputs.get("condition_result"),
                "evaluated_at": evaluated_at,
            }

        # Evaluate condition
        condition_type = config.get("condition_type", "custom_expression")
        condition_config = config.get("condition_config", "")

        triggered = ConditionEvaluator.evaluate(condition_type, condition_config, inputs)

        return {
            "triggered": triggered,
            "condition_result": inputs.get("condition_result"),
            "evaluated_at": evaluated_at,
        }
