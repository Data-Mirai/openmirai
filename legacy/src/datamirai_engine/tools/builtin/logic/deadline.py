"""Deadline tool — evaluates if N hours have passed since a reference timestamp.

FEAT-025: Generic deadline evaluation. Combines with trigger/heartbeat
for periodic deadline checking.
"""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class DeadlineTool(BaseTool):
    spec = ToolSpec(
        tool_type="logic/deadline",
        version="1.0.0",
        display_name="Deadline",
        description="Evaluates if N hours have passed since a reference timestamp",
        category="logic",
        icon="clock",
        intents=[
            "verificar si un deadline ha expirado",
            "evaluar si pasaron N horas desde un evento",
            "controlar timeouts en procesos de negocio",
            "descalificar si no respondio en 48 horas",
            "cancelar orden si no pago en 24 horas",
        ],
        inputs=[
            ToolInput(name="reference_time", type="string", required=False,
                      description="ISO timestamp from which to count (e.g. entity's updated_at)"),
            ToolInput(name="entity", type="object", required=False,
                      description="Entity object — will read the configured field as reference_time"),
        ],
        outputs=[
            ToolOutput(name="expired", type="boolean", description="True if deadline has passed"),
            ToolOutput(name="remaining_hours", type="number", description="Hours remaining (negative if expired)"),
            ToolOutput(name="deadline_at", type="string", description="ISO timestamp of when the deadline expires"),
        ],
        config=[
            ConfigField(name="hours", type="number", default=48,
                        description="Deadline duration in hours"),
            ConfigField(name="field", type="string", default="updated_at",
                        description="Timestamp field to read from entity input"),
            ConfigField(name="static_reference_time", type="string", default="",
                        description="Static reference timestamp (ISO). Used when reference_time is not in inputs."),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        hours = config.get("hours", 48)
        if isinstance(hours, str):
            try:
                hours = float(hours)
            except ValueError:
                hours = 48
        field = config.get("field", "updated_at")

        # Determine reference time: input > config > entity field
        ref_time_str = inputs.get("reference_time") or config.get("static_reference_time")

        if not ref_time_str:
            entity = inputs.get("entity", {})
            if isinstance(entity, dict):
                ref_time_str = entity.get(field)

        if not ref_time_str:
            # Default to now (deadline starts now)
            ref_time_str = datetime.now(timezone.utc).isoformat()

        # Parse reference time
        ref_time = _parse_iso(ref_time_str)
        now = datetime.now(timezone.utc)

        deadline_at = ref_time + __import__("datetime").timedelta(hours=hours)
        remaining = (deadline_at - now).total_seconds() / 3600
        expired = remaining <= 0

        return {
            "expired": expired,
            "remaining_hours": round(remaining, 2),
            "deadline_at": deadline_at.isoformat(),
        }


def _parse_iso(s: str) -> datetime:
    """Parse an ISO 8601 timestamp, tolerant of common variations."""
    s = str(s).strip()
    try:
        dt = datetime.fromisoformat(s)
    except ValueError:
        # Try stripping trailing Z
        dt = datetime.fromisoformat(s.replace("Z", "+00:00"))

    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt
