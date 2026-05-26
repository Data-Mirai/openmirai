"""ShortTermMemory — Session trace and context during graph execution.

Lives in RAM during execution. Persisted to Postgres on session completion.
"""

from __future__ import annotations

import time
from typing import Any


class ShortTermMemory:
    """Per-session memory: logs, decisions, and per-block metrics.

    Captures the step-by-step detail of current execution:
    - Logs: free-form messages during execution
    - Decisions: why the runner took a specific path at bifurcations
    - Metrics: duration, tokens, etc. per block execution
    """

    def __init__(self, session_id: str, agent_id: str) -> None:
        self.session_id = session_id
        self.agent_id = agent_id
        self._logs: list[dict[str, Any]] = []
        self._decisions: list[dict[str, Any]] = []
        self._metrics: list[dict[str, Any]] = []

    def log(self, message: str) -> None:
        self._logs.append({"message": message, "timestamp": time.time()})

    def record_decision(
        self,
        node_id: str,
        decision: str,
        alternatives: list[str] | None = None,
    ) -> None:
        self._decisions.append({
            "node_id": node_id,
            "decision": decision,
            "alternatives": alternatives or [],
            "timestamp": time.time(),
        })

    def record_metrics(
        self,
        node_id: str,
        tool_type: str,
        *,
        duration_ms: float = 0,
        tokens_used: int = 0,
        **extra: Any,
    ) -> None:
        entry = {
            "node_id": node_id,
            "tool_type": tool_type,
            "duration_ms": duration_ms,
            "tokens_used": tokens_used,
            "timestamp": time.time(),
        }
        entry.update(extra)
        self._metrics.append(entry)

    def get_logs(self) -> list[dict[str, Any]]:
        return list(self._logs)

    def get_decisions(self) -> list[dict[str, Any]]:
        return list(self._decisions)

    def get_metrics(self) -> list[dict[str, Any]]:
        return list(self._metrics)

    def get_trace(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "agent_id": self.agent_id,
            "logs": self.get_logs(),
            "decisions": self.get_decisions(),
            "metrics": self.get_metrics(),
        }

    def to_dict(self) -> dict[str, Any]:
        """Serialize for persistence to Postgres."""
        return self.get_trace()
