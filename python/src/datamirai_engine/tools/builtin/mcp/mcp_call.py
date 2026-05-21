"""MCP Call tool — executes a tool from an external MCP server.

FEAT-024: Connects to MCP servers (WhatsApp, Gmail, Slack, etc.) via stdio protocol.
The MCP server runs as a separate process; this tool acts as the client.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

logger = logging.getLogger(__name__)


class MCPCallTool(BaseTool):
    spec = ToolSpec(
        tool_type="mcp/call",
        version="1.0.0",
        display_name="MCP Call",
        description="Ejecuta una herramienta de un MCP server externo (WhatsApp, Gmail, Slack, etc.)",
        category="mcp",
        icon="plug",
        intents=[
            "conectar con un servicio externo via MCP",
            "enviar mensaje por WhatsApp o Slack",
            "leer emails de Gmail",
            "interactuar con APIs externas via MCP protocol",
            "usar herramientas de un MCP server instalado",
        ],
        inputs=[
            ToolInput(
                name="arguments",
                type="object",
                required=False,
                description="Argumentos para la tool del MCP server (override del config). Se pasan como JSON al server.",
            ),
        ],
        outputs=[
            ToolOutput(name="result", type="any", description="Resultado devuelto por la tool del MCP server"),
            ToolOutput(name="server_name", type="string", description="Nombre del MCP server que ejecuto la tool"),
            ToolOutput(name="tool_name", type="string", description="Nombre de la tool ejecutada"),
            ToolOutput(name="success", type="boolean", description="Si la ejecucion fue exitosa"),
            ToolOutput(name="error", type="string", description="Mensaje de error si fallo"),
        ],
        config=[
            ConfigField(
                name="server_name",
                type="string",
                default="",
                required=True,
                description="Nombre del MCP server instalado (ej: whatsapp-baileys, gmail, slack)",
            ),
            ConfigField(
                name="tool_name",
                type="string",
                default="",
                required=True,
                description="Nombre de la tool a ejecutar en el MCP server (ej: send_message, search_emails)",
            ),
            ConfigField(
                name="arguments",
                type="object",
                default=None,
                description="Argumentos estaticos para la tool (JSON). Se pueden override via input.",
            ),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        server_name = config.get("server_name", "")
        tool_name = config.get("tool_name", "")

        if not server_name:
            return {"result": None, "server_name": "", "tool_name": "", "success": False, "error": "server_name is required"}
        if not tool_name:
            return {"result": None, "server_name": server_name, "tool_name": "", "success": False, "error": "tool_name is required"}

        # Merge arguments: config defaults + input overrides
        args = config.get("arguments") or {}
        if inputs.get("arguments"):
            args = {**args, **inputs["arguments"]}

        # Try to get MCP client from context
        mcp_client = getattr(context, "mcp_client", None) if context else None

        if mcp_client is None:
            # Fallback: try to connect directly using server config
            logger.warning("No MCP client in context for server '%s' — tool will fail", server_name)
            return {
                "result": None,
                "server_name": server_name,
                "tool_name": tool_name,
                "success": False,
                "error": f"MCP server '{server_name}' is not connected. Install and start it from Settings.",
            }

        try:
            result = await mcp_client.call_tool(server_name, tool_name, args)
            return {
                "result": result,
                "server_name": server_name,
                "tool_name": tool_name,
                "success": True,
                "error": "",
            }
        except Exception as e:
            logger.error("MCP call failed: server=%s tool=%s error=%s", server_name, tool_name, e)
            return {
                "result": None,
                "server_name": server_name,
                "tool_name": tool_name,
                "success": False,
                "error": str(e),
            }
