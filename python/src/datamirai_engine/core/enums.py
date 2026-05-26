"""Typed enums for all finite-set configuration values.

FEAT-034 / API-01: Every config parameter with a finite set of values
MUST be a typed enum, never a loose string.
"""

from __future__ import annotations

from enum import Enum


class Op(str, Enum):
    """Edge condition operators."""

    EQ = "eq"
    NEQ = "neq"
    GT = "gt"
    LT = "lt"
    GTE = "gte"
    LTE = "lte"
    IN = "in"
    CONTAINS = "contains"


class Backoff(str, Enum):
    """Retry backoff strategies."""

    NONE = "none"
    LINEAR = "linear"
    EXPONENTIAL = "exponential"


class OnFailure(str, Enum):
    """Behavior when retries are exhausted."""

    STOP = "stop"
    SKIP = "skip"
    ROUTE_TO_ERROR = "route_to_error"


class DataType(str, Enum):
    """Logical data types for tool inputs/outputs."""

    STRING = "string"
    NUMBER = "number"
    BOOLEAN = "boolean"
    OBJECT = "object"
    ARRAY = "array"
    ANY = "any"


class TriggerType(str, Enum):
    """Agent trigger types."""

    WEBHOOK = "webhook"
    SCHEDULE = "schedule"
    EVENT = "event"
    MANUAL = "manual"
    AGENT_CALL = "agent_call"


class SessionStatus(str, Enum):
    """Session execution status."""

    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    TIMEOUT = "timeout"
    INTERRUPTED = "interrupted"


class ConfigFieldType(str, Enum):
    """Config field types for editor panel."""

    STRING = "string"
    NUMBER = "number"
    BOOLEAN = "boolean"
    SELECT = "select"
    SLIDER = "slider"
    OBJECT = "object"
