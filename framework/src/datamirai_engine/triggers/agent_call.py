"""AgentCall trigger — another agent invokes this one."""

from __future__ import annotations

import time
from typing import Any

from datamirai_engine.triggers.base import TriggerSpec


class AgentCallTrigger:
    """Fires when another agent calls this agent as sub-execution.

    Sub-agent inherits environment resources but creates own session (REGLA-11).
    """

    def __init__(self, name: str, target_agent_id: str) -> None:
        self.target_agent_id = target_agent_id
        self.spec = TriggerSpec(
            trigger_type="agent_call",
            name=name,
            config={"target_agent_id": target_agent_id},
        )

    def build_output(
        self,
        caller_agent_id: str = "",
        caller_session_id: str = "",
        payload: dict[str, Any] | None = None,
        environment_id: str | None = None,
    ) -> dict[str, Any]:
        return {
            "caller_agent_id": caller_agent_id,
            "caller_session_id": caller_session_id,
            "payload": payload or {},
            "environment_id": environment_id,
            "triggered_at": time.time(),
        }
