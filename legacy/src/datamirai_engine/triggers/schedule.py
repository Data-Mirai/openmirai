"""Schedule trigger — cron or interval based activation."""

from __future__ import annotations

import time
from typing import Any

from datamirai_engine.triggers.base import TriggerSpec


class ScheduleTrigger:
    """Fires on cron schedule or fixed interval."""

    def __init__(
        self,
        name: str,
        cron: str | None = None,
        interval_seconds: int | None = None,
        timezone: str = "UTC",
    ) -> None:
        self.cron = cron
        self.interval_seconds = interval_seconds
        self.timezone = timezone
        self.mode = "cron" if cron else "interval"
        self.spec = TriggerSpec(
            trigger_type="schedule",
            name=name,
            config={
                "mode": self.mode,
                "cron": cron,
                "interval_seconds": interval_seconds,
                "timezone": timezone,
            },
        )

    def build_output(self, run_count: int = 0) -> dict[str, Any]:
        return {
            "triggered_at": time.time(),
            "run_count": run_count,
            "mode": self.mode,
        }
