"""Energy data models — atomic metering events and rate configuration."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any


class OperationType(str, Enum):
    LLM_CALL = "LLM_CALL"
    MCP_CALL = "MCP_CALL"
    TOOL_EXEC = "TOOL_EXEC"
    COMPUTE_TIME = "COMPUTE_TIME"
    STORAGE_OP = "STORAGE_OP"
    DB_OP = "DB_OP"


class CostCategory(str, Enum):
    INTERNAL_INFRA = "INTERNAL_INFRA"
    EXTERNAL_SERVICE = "EXTERNAL_SERVICE"
    PLATFORM_FEE = "PLATFORM_FEE"


class QuantityUnit(str, Enum):
    TOKENS_IN = "TOKENS_IN"
    TOKENS_OUT = "TOKENS_OUT"
    BYTES = "BYTES"
    SECONDS = "SECONDS"
    INVOCATIONS = "INVOCATIONS"


@dataclass(frozen=True)
class EnergyEvent:
    """Atomic record of a single energy-consuming operation. Immutable."""

    id: str
    agent_id: str
    operation_type: OperationType
    cost_category: CostCategory
    quantity: float
    quantity_unit: QuantityUnit
    actual_cost_usd: float
    energy_charged: float
    created_at: str
    session_id: str | None = None
    run_id: str | None = None
    cycle_id: str | None = None
    node_id: str | None = None
    provider: str | None = None
    model: str | None = None
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass
class EnergyRate:
    """Conversion rate: how much energy per unit of a given operation."""

    id: str
    operation_type: str
    cost_per_unit_usd: float
    energy_per_unit: float
    quantity_unit: str
    is_active: bool = True
    provider: str | None = None
    model_pattern: str | None = None
    created_at: str = ""
    updated_at: str = ""


@dataclass
class EnergyBalance:
    """Energy balance for a universe. Local = informational, Cloud = enforcement."""

    id: str
    universe_id: str
    available: float = 0.0
    total_consumed: float = 0.0
    total_purchased: float = 0.0
    updated_at: str = ""
