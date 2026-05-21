"""Tests for advanced triggers — Schedule, Event, AgentCall."""

from __future__ import annotations

from datamirai_engine.triggers.agent_call import AgentCallTrigger
from datamirai_engine.triggers.event import EventTrigger
from datamirai_engine.triggers.schedule import ScheduleTrigger


class TestScheduleTrigger:
    def test_create_cron(self):
        trigger = ScheduleTrigger(name="Daily Report", cron="0 9 * * *")
        assert trigger.spec.trigger_type == "schedule"
        assert trigger.cron == "0 9 * * *"
        assert trigger.mode == "cron"

    def test_create_interval(self):
        trigger = ScheduleTrigger(name="Hourly", interval_seconds=3600)
        assert trigger.mode == "interval"
        assert trigger.interval_seconds == 3600

    def test_build_output(self):
        trigger = ScheduleTrigger(name="Test", cron="* * * * *")
        output = trigger.build_output(run_count=5)
        assert "triggered_at" in output
        assert output["run_count"] == 5

    def test_timezone_default(self):
        trigger = ScheduleTrigger(name="Test", cron="0 0 * * *")
        assert trigger.timezone == "UTC"


class TestEventTrigger:
    def test_create_storage_event(self):
        trigger = EventTrigger(
            name="File Upload",
            source="storage",
            event_type="file_uploaded",
            filter={"prefix": "uploads/", "extension": ".pdf"},
        )
        assert trigger.spec.trigger_type == "event"
        assert trigger.source == "storage"
        assert trigger.event_type == "file_uploaded"

    def test_create_db_event(self):
        trigger = EventTrigger(
            name="Row Insert",
            source="database",
            event_type="row_inserted",
            filter={"table": "orders"},
        )
        assert trigger.source == "database"

    def test_build_output(self):
        trigger = EventTrigger(
            name="Test",
            source="storage",
            event_type="file_uploaded",
        )
        output = trigger.build_output(
            event_data={"file_name": "report.pdf", "size": 1024}
        )
        assert output["source"] == "storage"
        assert output["event_type"] == "file_uploaded"
        assert output["event_data"]["file_name"] == "report.pdf"


class TestAgentCallTrigger:
    def test_create(self):
        trigger = AgentCallTrigger(
            name="Sub-agent",
            target_agent_id="agent-002",
        )
        assert trigger.spec.trigger_type == "agent_call"
        assert trigger.target_agent_id == "agent-002"

    def test_build_output(self):
        trigger = AgentCallTrigger(name="Sub", target_agent_id="a1")
        output = trigger.build_output(
            caller_agent_id="parent-001",
            caller_session_id="sess-001",
            payload={"task": "summarize"},
        )
        assert output["caller_agent_id"] == "parent-001"
        assert output["payload"]["task"] == "summarize"

    def test_inherits_environment(self):
        trigger = AgentCallTrigger(name="Sub", target_agent_id="a1")
        output = trigger.build_output(
            caller_agent_id="p1",
            caller_session_id="s1",
            environment_id="env-prod",
        )
        assert output["environment_id"] == "env-prod"
