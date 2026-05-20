"""Trigger system — TriggerSpec, WebhookTrigger, ManualTrigger."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


class TriggerSpec(BaseModel):
    """Declarative spec for a trigger — type, name, config."""

    trigger_type: str  # webhook | schedule | event | manual | agent_call
    name: str
    config: dict[str, Any] = Field(default_factory=dict)

    model_config = {"frozen": True}


class WebhookTrigger:
    """HTTP webhook trigger. Receives external requests."""

    def __init__(
        self,
        name: str,
        method: str = "POST",
        path: str = "/webhook",
        auth_mode: str = "none",
    ) -> None:
        self.method = method
        self.path = path
        self.auth_mode = auth_mode
        self.spec = TriggerSpec(
            trigger_type="webhook",
            name=name,
            config={"method": method, "path": path, "auth_mode": auth_mode},
        )

    def build_output(
        self,
        body: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        query_params: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        return {
            "body": body or {},
            "headers": headers or {},
            "query_params": query_params or {},
        }


class ManualTrigger:
    """Manual trigger. User clicks Execute from UI/API."""

    def __init__(
        self,
        name: str,
        input_form: list[dict[str, Any]] | None = None,
    ) -> None:
        self.input_form = input_form or []
        self.spec = TriggerSpec(
            trigger_type="manual",
            name=name,
            config={"input_form": self.input_form},
        )

    def build_output(
        self,
        user_input: dict[str, Any] | None = None,
        triggered_by: str = "anonymous",
    ) -> dict[str, Any]:
        return {
            "user_input": user_input or {},
            "triggered_by": triggered_by,
        }
