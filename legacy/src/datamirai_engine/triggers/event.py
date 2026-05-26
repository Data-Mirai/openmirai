"""Event trigger — fires on resource notifications (storage, database)."""

from __future__ import annotations

import time
from typing import Any

from datamirai_engine.triggers.base import TriggerSpec


class EventTrigger:
    """Fires when a resource emits a notification (file uploaded, row inserted, etc.)."""

    def __init__(
        self,
        name: str,
        source: str,  # storage | database
        event_type: str,  # file_uploaded | file_deleted | row_inserted | row_updated
        filter: dict[str, Any] | None = None,
    ) -> None:
        self.source = source
        self.event_type = event_type
        self.filter = filter or {}
        self.spec = TriggerSpec(
            trigger_type="event",
            name=name,
            config={
                "source": source,
                "event_type": event_type,
                "filter": self.filter,
            },
        )

    def build_output(self, event_data: dict[str, Any] | None = None) -> dict[str, Any]:
        return {
            "source": self.source,
            "event_type": self.event_type,
            "event_data": event_data or {},
            "triggered_at": time.time(),
        }
