"""Trigger tools — entry point tools for graph execution."""

from __future__ import annotations

from datamirai_engine.tools.base import BaseTool, ToolOutput, ToolSpec, ConfigField


class WebhookTriggerTool(BaseTool):
    spec = ToolSpec(
        tool_type="trigger/webhook",
        version="1.0.0",
        display_name="Webhook",
        description="HTTP webhook entry point",
        category="trigger",
        icon="webhook",
        intents=[
            "el agente se activa cuando recibe una llamada HTTP externa",
            "integracion con servicios externos que envian datos via API",
            "recibir notificaciones, formularios, o eventos de otros sistemas",
        ],
        outputs=[
            ToolOutput(name="body", type="object"),
            ToolOutput(name="headers", type="object"),
            ToolOutput(name="query_params", type="object"),
        ],
    )

    async def execute(self, inputs, config, context):
        return {
            "body": config.get("body", config.get("mock_payload", {})),
            "headers": config.get("headers", {}),
            "query_params": config.get("query_params", {}),
        }


class ManualTriggerTool(BaseTool):
    spec = ToolSpec(
        tool_type="trigger/manual",
        version="1.0.0",
        display_name="Manual",
        description="Manual execution entry point",
        category="trigger",
        icon="play",
        intents=[
            "el usuario ejecuta el agente manualmente desde la interfaz",
            "pruebas, ejecucion bajo demanda, investigacion puntual",
            "emite fecha actual y rango de 7 dias para busquedas temporales",
        ],
        outputs=[
            ToolOutput(name="user_input", type="object"),
            ToolOutput(name="triggered_by", type="string"),
            ToolOutput(name="current_date", type="string"),
            ToolOutput(name="date_7d_ago", type="string"),
            ToolOutput(name="date_range", type="string"),
            ToolOutput(name="timestamp", type="number"),
        ],
        config=[
            ConfigField(name="mock_payload", type="object", default={}),
        ],
    )

    async def execute(self, inputs, config, context):
        from datetime import datetime, timedelta, timezone
        now = datetime.now(timezone.utc)
        week_ago = now - timedelta(days=7)
        return {
            "user_input": config.get("user_input", config.get("mock_payload", {})),
            "triggered_by": config.get("triggered_by", "manual"),
            "current_date": now.strftime("%Y-%m-%d"),
            "date_7d_ago": week_ago.strftime("%Y-%m-%d"),
            "date_range": f"{week_ago.strftime('%Y-%m-%d')}..{now.strftime('%Y-%m-%d')}",
            "timestamp": now.timestamp(),
        }


class ScheduleTriggerTool(BaseTool):
    spec = ToolSpec(
        tool_type="trigger/schedule",
        version="1.0.0",
        display_name="Schedule",
        description="Cron/interval trigger",
        category="trigger",
        icon="clock",
        intents=[
            "el agente se ejecuta automaticamente cada cierto tiempo",
            "monitoreo periodico, reportes diarios, scraping recurrente",
            "tareas programadas tipo cron",
        ],
        outputs=[
            ToolOutput(name="triggered_at", type="number"),
            ToolOutput(name="run_count", type="number"),
        ],
    )

    async def execute(self, inputs, config, context):
        import time
        return {
            "triggered_at": time.time(),
            "run_count": config.get("run_count", 0),
        }


class EventTriggerTool(BaseTool):
    spec = ToolSpec(
        tool_type="trigger/event",
        version="1.0.0",
        display_name="Event",
        description="Resource event trigger",
        category="trigger",
        icon="zap",
        intents=[
            "el agente reacciona a un evento del sistema (nuevo registro, archivo subido, etc)",
            "automatizacion reactiva: cuando pasa X, ejecutar Y",
        ],
        outputs=[
            ToolOutput(name="source", type="string"),
            ToolOutput(name="event_type", type="string"),
            ToolOutput(name="event_data", type="object"),
        ],
    )

    async def execute(self, inputs, config, context):
        return {
            "source": config.get("source", ""),
            "event_type": config.get("event_type", ""),
            "event_data": config.get("event_data", {}),
        }
