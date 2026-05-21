"""Tests for Trigger system — TriggerSpec, webhook, manual."""

from __future__ import annotations

from datamirai_engine.triggers.base import ManualTrigger, TriggerSpec, WebhookTrigger


class TestTriggerSpec:
    def test_create(self):
        spec = TriggerSpec(
            trigger_type="webhook",
            name="My Webhook",
            config={"method": "POST", "path": "/hook"},
        )
        assert spec.trigger_type == "webhook"
        assert spec.config["method"] == "POST"

    def test_serialization(self):
        spec = TriggerSpec(
            trigger_type="schedule",
            name="Daily",
            config={"cron": "0 0 * * *"},
        )
        data = spec.model_dump()
        restored = TriggerSpec.model_validate(data)
        assert restored == spec


class TestWebhookTrigger:
    def test_create(self):
        trigger = WebhookTrigger(
            name="API Hook",
            method="POST",
            path="/api/hooks/ingest",
            auth_mode="api_key",
        )
        assert trigger.spec.trigger_type == "webhook"
        assert trigger.method == "POST"

    def test_build_output(self):
        trigger = WebhookTrigger(name="Hook", method="POST", path="/hook")
        output = trigger.build_output(
            body={"event": "file_uploaded"},
            headers={"content-type": "application/json"},
            query_params={"source": "api"},
        )
        assert output["body"]["event"] == "file_uploaded"
        assert output["headers"]["content-type"] == "application/json"
        assert output["query_params"]["source"] == "api"

    def test_default_auth_mode(self):
        trigger = WebhookTrigger(name="Hook", method="POST", path="/hook")
        assert trigger.auth_mode == "none"


class TestManualTrigger:
    def test_create(self):
        trigger = ManualTrigger(name="Run Manually")
        assert trigger.spec.trigger_type == "manual"

    def test_build_output_with_form(self):
        trigger = ManualTrigger(
            name="Run",
            input_form=[
                {"name": "query", "type": "string", "required": True},
                {"name": "limit", "type": "number", "required": False, "default": 10},
            ],
        )
        output = trigger.build_output(
            user_input={"query": "hello"},
            triggered_by="user-123",
        )
        assert output["user_input"]["query"] == "hello"
        assert output["triggered_by"] == "user-123"

    def test_build_output_empty(self):
        trigger = ManualTrigger(name="Run")
        output = trigger.build_output()
        assert output["user_input"] == {}
        assert "triggered_by" in output
