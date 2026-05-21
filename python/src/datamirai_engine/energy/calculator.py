"""EnergyCalculator — resolves energy_rate for an operation and computes energy cost."""

from __future__ import annotations

import fnmatch
import logging
from typing import Any, Protocol

from datamirai_engine.energy.models import CostCategory, OperationType, QuantityUnit

logger = logging.getLogger(__name__)


class RateStore(Protocol):
    """Interface for reading energy rates from persistence."""

    def get_active_rates(self) -> list[dict[str, Any]]: ...


class EnergyCalculator:
    """Resolves rate for an operation and calculates energy + actual cost."""

    def __init__(self, rate_store: RateStore) -> None:
        self._store = rate_store
        self._cache: list[dict[str, Any]] | None = None

    def invalidate_cache(self) -> None:
        self._cache = None

    def _rates(self) -> list[dict[str, Any]]:
        if self._cache is None:
            self._cache = self._store.get_active_rates()
        return self._cache

    def calculate(
        self,
        operation_type: OperationType | str,
        quantity: float,
        quantity_unit: QuantityUnit | str,
        provider: str | None = None,
        model: str | None = None,
    ) -> tuple[float, float]:
        """Return (actual_cost_usd, energy_charged) for the given operation.

        Matching priority: operation_type + provider + model_pattern (most specific wins).
        If no rate found → (0.0, 0.0) + warning.
        """
        op = operation_type.value if hasattr(operation_type, "value") else str(operation_type)
        unit = quantity_unit.value if hasattr(quantity_unit, "value") else str(quantity_unit)

        best_rate: dict[str, Any] | None = None
        best_specificity = -1

        for rate in self._rates():
            if rate["operation_type"] != op:
                continue
            if rate["quantity_unit"] != unit:
                continue

            specificity = 0

            # Provider match
            rate_provider = rate.get("provider")
            if rate_provider:
                if provider and rate_provider.lower() == provider.lower():
                    specificity += 1
                else:
                    continue  # provider mismatch → skip
            # Model pattern match
            rate_pattern = rate.get("model_pattern")
            if rate_pattern:
                if model and fnmatch.fnmatch(model.lower(), rate_pattern.lower()):
                    specificity += 2
                else:
                    continue  # pattern mismatch → skip

            if specificity > best_specificity:
                best_specificity = specificity
                best_rate = rate

        if best_rate is None:
            logger.warning(
                "No energy rate for %s/%s (provider=%s, model=%s) — energy=0",
                op,
                unit,
                provider,
                model,
            )
            return 0.0, 0.0

        cost = quantity * best_rate["cost_per_unit_usd"]
        energy = quantity * best_rate["energy_per_unit"]
        return cost, energy
