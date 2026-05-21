"""EnergyRecorder — creates energy events and persists them atomically."""

from __future__ import annotations

import logging
import uuid
from datetime import datetime, timezone
from typing import Any, Callable, Protocol

from datamirai_engine.energy.calculator import EnergyCalculator
from datamirai_engine.energy.models import CostCategory, EnergyEvent, OperationType, QuantityUnit

logger = logging.getLogger(__name__)


class EventStore(Protocol):
    """Interface for persisting energy events."""

    def insert_energy_event(self, event: dict[str, Any]) -> None: ...


class BalanceStore(Protocol):
    """Interface for reading/updating energy balance."""

    def get_balance(self, universe_id: str) -> dict[str, Any] | None: ...
    def deduct(self, universe_id: str, amount: float) -> float: ...


EventCallback = Callable[[EnergyEvent], None]


class EnergyRecorder:
    """Records energy consumption for every metered operation.

    Usage:
        recorder.record_llm_call(agent_id, session_id, node_id, provider, model, tokens_in, tokens_out)
        recorder.record_mcp_call(agent_id, session_id, node_id, provider)
        recorder.record_compute_time(agent_id, session_id, duration_seconds)
    """

    def __init__(
        self,
        calculator: EnergyCalculator,
        event_store: EventStore,
        balance_store: BalanceStore | None = None,
        on_event: EventCallback | None = None,
        enforce_balance: bool = False,
    ) -> None:
        self._calc = calculator
        self._store = event_store
        self._balance = balance_store
        self._on_event = on_event
        self._enforce = enforce_balance

    def record_llm_call(
        self,
        *,
        agent_id: str,
        session_id: str | None = None,
        run_id: str | None = None,
        cycle_id: str | None = None,
        node_id: str | None = None,
        provider: str,
        model: str,
        tokens_in: int,
        tokens_out: int,
    ) -> list[EnergyEvent]:
        """Record energy for an LLM call. Creates 2 events (tokens_in + tokens_out)."""
        events: list[EnergyEvent] = []

        for qty, unit in [(tokens_in, QuantityUnit.TOKENS_IN), (tokens_out, QuantityUnit.TOKENS_OUT)]:
            if qty <= 0:
                continue
            cost, energy = self._calc.calculate(
                OperationType.LLM_CALL, qty, unit, provider=provider, model=model
            )
            ev = self._create_event(
                agent_id=agent_id,
                session_id=session_id,
                run_id=run_id,
                cycle_id=cycle_id,
                node_id=node_id,
                operation_type=OperationType.LLM_CALL,
                cost_category=CostCategory.EXTERNAL_SERVICE,
                provider=provider,
                model=model,
                quantity=qty,
                quantity_unit=unit,
                actual_cost_usd=cost,
                energy_charged=energy,
                metadata={"tokens_in": tokens_in, "tokens_out": tokens_out},
            )
            events.append(ev)

        return events

    def record_mcp_call(
        self,
        *,
        agent_id: str,
        session_id: str | None = None,
        run_id: str | None = None,
        cycle_id: str | None = None,
        node_id: str | None = None,
        provider: str = "",
        tool_name: str = "",
    ) -> EnergyEvent:
        cost, energy = self._calc.calculate(
            OperationType.MCP_CALL, 1, QuantityUnit.INVOCATIONS, provider=provider
        )
        return self._create_event(
            agent_id=agent_id,
            session_id=session_id,
            run_id=run_id,
            cycle_id=cycle_id,
            node_id=node_id,
            operation_type=OperationType.MCP_CALL,
            cost_category=CostCategory.EXTERNAL_SERVICE,
            provider=provider,
            quantity=1,
            quantity_unit=QuantityUnit.INVOCATIONS,
            actual_cost_usd=cost,
            energy_charged=energy,
            metadata={"tool_name": tool_name},
        )

    def record_tool_exec(
        self,
        *,
        agent_id: str,
        session_id: str | None = None,
        run_id: str | None = None,
        cycle_id: str | None = None,
        node_id: str | None = None,
        tool_type: str = "",
    ) -> EnergyEvent:
        cost, energy = self._calc.calculate(
            OperationType.TOOL_EXEC, 1, QuantityUnit.INVOCATIONS
        )
        return self._create_event(
            agent_id=agent_id,
            session_id=session_id,
            run_id=run_id,
            cycle_id=cycle_id,
            node_id=node_id,
            operation_type=OperationType.TOOL_EXEC,
            cost_category=CostCategory.INTERNAL_INFRA,
            quantity=1,
            quantity_unit=QuantityUnit.INVOCATIONS,
            actual_cost_usd=cost,
            energy_charged=energy,
            metadata={"tool_type": tool_type},
        )

    def record_compute_time(
        self,
        *,
        agent_id: str,
        session_id: str | None = None,
        run_id: str | None = None,
        cycle_id: str | None = None,
        duration_seconds: float,
    ) -> EnergyEvent:
        cost, energy = self._calc.calculate(
            OperationType.COMPUTE_TIME, duration_seconds, QuantityUnit.SECONDS
        )
        return self._create_event(
            agent_id=agent_id,
            session_id=session_id,
            run_id=run_id,
            cycle_id=cycle_id,
            operation_type=OperationType.COMPUTE_TIME,
            cost_category=CostCategory.INTERNAL_INFRA,
            quantity=duration_seconds,
            quantity_unit=QuantityUnit.SECONDS,
            actual_cost_usd=cost,
            energy_charged=energy,
        )

    def check_balance(self, universe_id: str) -> float:
        """Return available energy. If no balance store or enforcement off → infinity."""
        if not self._enforce or not self._balance:
            return float("inf")
        bal = self._balance.get_balance(universe_id)
        if not bal:
            return float("inf")
        return bal.get("available", 0.0)

    def _create_event(self, **kwargs: Any) -> EnergyEvent:
        event = EnergyEvent(
            id=str(uuid.uuid4()),
            created_at=datetime.now(timezone.utc).isoformat(),
            **kwargs,
        )
        # Persist
        self._store.insert_energy_event(_event_to_dict(event))

        # Deduct from balance if enforcing
        if self._enforce and self._balance and event.energy_charged > 0:
            # Find universe_id from agent context — for now, deduction is caller's responsibility
            pass

        # Notify listeners
        if self._on_event:
            try:
                self._on_event(event)
            except Exception:
                logger.warning("Energy event callback failed", exc_info=True)

        return event


def _event_to_dict(ev: EnergyEvent) -> dict[str, Any]:
    import json

    return {
        "id": ev.id,
        "agent_id": ev.agent_id,
        "session_id": ev.session_id,
        "run_id": ev.run_id,
        "cycle_id": ev.cycle_id,
        "node_id": ev.node_id,
        "operation_type": str(ev.operation_type.value) if isinstance(ev.operation_type, OperationType) else str(ev.operation_type),
        "cost_category": str(ev.cost_category.value) if isinstance(ev.cost_category, CostCategory) else str(ev.cost_category),
        "provider": ev.provider,
        "model": ev.model,
        "quantity": ev.quantity,
        "quantity_unit": str(ev.quantity_unit.value) if isinstance(ev.quantity_unit, QuantityUnit) else str(ev.quantity_unit),
        "actual_cost_usd": ev.actual_cost_usd,
        "energy_charged": ev.energy_charged,
        "metadata": json.dumps(ev.metadata) if isinstance(ev.metadata, dict) else ev.metadata,
        "created_at": ev.created_at,
    }
