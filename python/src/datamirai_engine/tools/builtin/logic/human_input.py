"""Human Input tool — pauses execution and waits for human decision."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)
from datamirai_engine.core.context import ExecutionContext


class HumanInputTool(BaseTool):
    """Declarative human-in-the-loop block.

    When the cursor reaches this node, the GraphRunner pauses execution,
    creates a checkpoint and an interrupt, and waits for the user to
    respond before continuing.

    The actual pause/resume logic lives in GraphRunner — this tool's
    execute() is only called when the interrupt is resolved with a response.
    """

    spec = ToolSpec(
        tool_type="logic/human_input",
        version="1.0.0",
        display_name="Decision Humana",
        description="Pausa la ejecucion y espera input del usuario antes de continuar",
        category="logic",
        icon="user-check",
        intents=[
            "requiere aprobacion o decision de un humano antes de continuar",
            "human-in-the-loop: revisar resultados antes de seguir",
            "pedir input adicional al usuario durante la ejecucion",
        ],
        inputs=[
            ToolInput(name="prompt", type="string", required=False,
                      description="Pregunta o contexto para el usuario"),
            ToolInput(name="options", type="array", required=False,
                      description="Opciones predefinidas (si aplica)"),
        ],
        outputs=[
            ToolOutput(name="response", type="any",
                       description="Respuesta del usuario"),
            ToolOutput(name="responded_by", type="string",
                       description="Identificador del usuario que respondio"),
            ToolOutput(name="response_time_ms", type="number",
                       description="Tiempo que tardo en responder"),
        ],
        config=[
            ConfigField(name="prompt", type="text", default="Requiere decision humana"),
            ConfigField(name="options", type="text", default=""),
            ConfigField(name="timeout_minutes", type="number", default=None),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        # This execute() is called when the interrupt is resolved
        # and the response has been injected into inputs by the runner.
        # In normal flow, GraphRunner intercepts this tool_type before
        # execute() is called and creates an interrupt instead.
        return {
            "response": inputs.get("response"),
            "responded_by": inputs.get("responded_by", "unknown"),
            "response_time_ms": inputs.get("response_time_ms", 0),
        }
