"""RunAgent tool — executes another agent as a synchronous sub-task.

Enables agent composition: a parent agent can delegate work to a child
agent by referencing its agent_id. The child runs its full graph and
returns its output to the parent.

Safety rules:
- REGLA-68: max nesting depth = 3 (parent > child > grandchild)
- REGLA-70: no circular references (agent A calls B calls A)
"""

from __future__ import annotations

import time
from typing import Any

from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)
from datamirai_engine.core.context import ExecutionContext


# Sentinel key in execution context metadata for tracking call chain
_NESTING_KEY = "__agent_call_chain__"
_MAX_NESTING_DEPTH = 3


class NestingDepthExceeded(Exception):
    """Raised when sub-agent nesting exceeds the allowed depth (REGLA-68)."""


class CircularAgentReference(Exception):
    """Raised when an agent call chain contains a cycle (REGLA-70)."""


class RunAgentTool(BaseTool):
    spec = ToolSpec(
        tool_type="agent/run_agent",
        version="1.0.0",
        display_name="Ejecutar Agente",
        description=(
            "Ejecuta otro agente completo como sub-tarea sincrona. "
            "El agente hijo corre su grafo completo y retorna su resultado."
        ),
        category="agent",
        icon="users",
        intents=[
            "ejecutar otro agente",
            "delegar tarea a sub-agente",
            "componer agentes",
        ],
        inputs=[
            ToolInput(
                name="agent_id",
                type="string",
                required=False,
                description="ID del agente a ejecutar como sub-tarea (puede venir de config)",
            ),
            ToolInput(
                name="input_data",
                type="object",
                required=False,
                default={},
                description="Datos de entrada para el sub-agente (trigger_data)",
            ),
        ],
        outputs=[
            ToolOutput(name="agent_id", type="string", description="ID del agente ejecutado"),
            ToolOutput(name="session_id", type="string", description="ID de la sesion hija creada"),
            ToolOutput(name="status", type="string", description="Estado final de la ejecucion"),
            ToolOutput(name="result", type="object", description="Resultado completo del sub-agente"),
            ToolOutput(name="duration_ms", type="number", description="Duracion total en ms"),
        ],
        config=[
            ConfigField(
                name="agent_id",
                type="string",
                default="",
                description="ID del agente a ejecutar (seleccionable en el editor)",
            ),
            ConfigField(
                name="timeout_seconds",
                type="number",
                default=300,
                description="Timeout maximo para la ejecucion del sub-agente (seg)",
            ),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        # Resolve agent_id: input takes priority, fallback to config
        agent_id = inputs.get("agent_id") or config.get("agent_id", "")
        if not agent_id:
            raise ValueError(
                "agent_id is required (via input or config). "
                "Select the target agent in the node configuration."
            )

        input_data = inputs.get("input_data", {}) or {}
        timeout = config.get("timeout_seconds", 300)

        # --- REGLA-68: nesting depth check ---
        call_chain = self._get_call_chain(context)
        if len(call_chain) >= _MAX_NESTING_DEPTH:
            raise NestingDepthExceeded(
                f"Sub-agent nesting depth exceeded (max={_MAX_NESTING_DEPTH}). "
                f"Call chain: {' -> '.join(call_chain)} -> {agent_id}"
            )

        # --- REGLA-70: circular reference check ---
        if agent_id in call_chain:
            raise CircularAgentReference(
                f"Circular agent reference detected: "
                f"{' -> '.join(call_chain)} -> {agent_id}"
            )

        # --- Execute sub-agent ---
        # In the framework layer, we return a mock result.
        # The real execution happens in the app layer which has access
        # to AgentRepo, GraphRepo, SessionRepo, and GraphRunner.
        # The app layer overrides this by registering a custom executor
        # or by hooking into the run_agent tool_type in RegistryExecutor.
        start_ms = time.time()

        result = {
            "agent_id": agent_id,
            "session_id": "",  # Populated by app layer
            "status": "pending",
            "result": {},
            "duration_ms": 0,
            "_input_data": input_data,
            "_timeout_seconds": timeout,
            "_call_chain": [*call_chain, agent_id],
        }

        duration_ms = int((time.time() - start_ms) * 1000)
        result["duration_ms"] = duration_ms

        return result

    @staticmethod
    def _get_call_chain(context: ExecutionContext | None) -> list[str]:
        """Extract the agent call chain from execution context metadata."""
        if context is None:
            return []
        # The call chain is injected by the app layer when executing
        # sub-agents. If not present, this is a top-level execution.
        metadata = getattr(context, "metadata", None)
        if metadata is None:
            return []
        if isinstance(metadata, dict):
            return list(metadata.get(_NESTING_KEY, []))
        return []
