"""Core types for the Mirai Python SDK."""

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Optional


class ExecutionStatus(str, Enum):
    COMPLETED = "Completed"
    FAILED = "Failed"
    INTERRUPTED = "Interrupted"


@dataclass
class TraceEntry:
    node_id: str
    tool_type: str
    status: str
    duration_ms: int
    retries: int
    error: Optional[str] = None


@dataclass
class ExecutionResult:
    status: ExecutionStatus
    state: dict[str, Any] = field(default_factory=dict)
    trace: list[TraceEntry] = field(default_factory=list)
    transcript: list[dict] = field(default_factory=list)
    error: Optional[str] = None

    @property
    def output(self) -> dict[str, Any]:
        """Convenience: return the full state as output."""
        return self.state

    @property
    def succeeded(self) -> bool:
        return self.status == ExecutionStatus.COMPLETED


@dataclass
class StreamEvent:
    event: str
    data: dict[str, Any] = field(default_factory=dict)
