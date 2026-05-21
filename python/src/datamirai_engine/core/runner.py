"""GraphRunner — Sequential cursor that traverses graph nodes following edges."""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable

from datamirai_engine.core.events import (
    CheckpointCallback,
    EventEmitter,
    EventType,
    ExecutionEvent,
    HookHandler,
)
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.state import SharedState


class GraphExecutionError(Exception):
    """Raised when graph execution encounters an unrecoverable error."""


class MaxIterationsError(GraphExecutionError):
    """Raised when loop exceeds max_iterations safeguard."""


class InterruptRequested(Exception):
    """Raised when a human_input block or manual pause interrupts execution."""

    def __init__(self, interrupt_type: str, node_id: str, prompt: dict | None = None):
        self.interrupt_type = interrupt_type
        self.node_id = node_id
        self.prompt = prompt
        super().__init__(f"Interrupt requested at node '{node_id}' ({interrupt_type})")


@dataclass(frozen=True)
class RetryPolicy:
    max_retries: int = 0
    backoff: str = "none"  # none | linear | exponential
    initial_delay_seconds: float = 1.0
    on_failure: str = "stop"  # stop | skip | route_to_error


@dataclass(frozen=True)
class TranscriptEvent:
    """Human-readable event generated during graph execution."""

    type: str  # started | block_start | block_end | decision | error | completed
    message: str
    timestamp: float
    node_id: str | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "type": self.type,
            "message": self.message,
            "timestamp": self.timestamp,
        }
        if self.node_id:
            d["node_id"] = self.node_id
        if self.metadata:
            d["metadata"] = self.metadata
        return d


# Display names for block categories
_TOOL_LABELS: dict[str, str] = {
    "trigger/webhook": "Webhook recibido",
    "trigger/schedule": "Ejecucion programada",
    "trigger/event": "Evento detectado",
    "trigger/manual": "Ejecucion manual",
    "trigger/agent_call": "Llamada de agente",
    "logic/condition": "Evaluando condicion",
    "logic/switch": "Evaluando switch",
    "logic/loop": "Iteracion de loop",
    "logic/merge": "Punto de convergencia",
    "logic/wait": "Esperando",
    "logic/human_input": "Esperando decision humana",
    "ai/llm_call": "Llamada a LLM",
    "ai/transcribe": "Transcribiendo audio",
    "ai/embeddings": "Generando embeddings",
    "data/db_read": "Leyendo base de datos",
    "data/db_write": "Escribiendo en base de datos",
    "data/storage_read": "Leyendo archivo",
    "data/storage_write": "Guardando archivo",
    "output/response": "Generando resultado final",
    "agent/run_agent": "Ejecutando sub-agente",
}


def _tool_label(tool_type: str) -> str:
    return _TOOL_LABELS.get(tool_type, tool_type)


@runtime_checkable
class ToolExecutor(Protocol):
    async def __call__(
        self, node: NodeDef, inputs: dict[str, Any], context: Any
    ) -> dict[str, Any]: ...


@dataclass
class ExecutionResult:
    status: str  # completed | failed | timeout | interrupted
    state: SharedState
    trace: list[dict[str, Any]] = field(default_factory=list)
    transcript: list[dict[str, Any]] = field(default_factory=list)
    error: str | None = None
    interrupt_node_id: str | None = None
    interrupt_prompt: dict | None = None


class GraphRunner:
    """Executes a graph by traversing nodes sequentially following edges.

    One active path at a time (REGLA-01). Evaluates edge conditions
    against block output to determine next node.

    Supports:
    - Checkpointing: saves state after each block (REGLA-12)
    - Event emission: streams events to SSE subscribers
    - Hooks: intercepts execution at pre/post block, LLM, error
    - Interrupts: pauses for human_input blocks or manual pause
    """

    def __init__(
        self,
        executor: ToolExecutor,
        *,
        session_id: str = "",
        event_emitter: EventEmitter | None = None,
        checkpoint_callback: CheckpointCallback | None = None,
        hook_handler: HookHandler | None = None,
    ) -> None:
        self._executor = executor
        self._session_id = session_id
        self._emitter = event_emitter
        self._checkpoint = checkpoint_callback
        self._hook = hook_handler
        self._pause_requested = False

    def request_pause(self) -> None:
        """Request a pause at the next safe point (after current block finishes)."""
        self._pause_requested = True

    async def resume_from_state(
        self,
        graph: GraphDef,
        *,
        context: Any,
        state_snapshot: dict[str, Any],
        entry_node_id: str,
        start_step: int = 0,
    ) -> ExecutionResult:
        """Resume execution from a previously saved state.

        Restores SharedState from snapshot, then starts execution from
        entry_node_id. Otherwise behaves identically to run().
        """
        return await self.run(
            graph,
            context=context,
            entry_node_id=entry_node_id,
            initial_state=state_snapshot,
            start_step=start_step,
        )

    async def run(
        self,
        graph: GraphDef,
        context: Any,
        entry_node_id: str,
        *,
        initial_state: dict[str, Any] | None = None,
        start_step: int = 0,
    ) -> ExecutionResult:
        node = graph.get_node(entry_node_id)
        if node is None:
            raise GraphExecutionError(
                f"Entry node '{entry_node_id}' not found in graph '{graph.id}'"
            )

        state = SharedState()
        # Restore state from checkpoint if resuming
        if initial_state:
            for nid, output in initial_state.items():
                state.set(nid, output)

        trace: list[dict[str, Any]] = []
        transcript: list[dict[str, Any]] = []
        visit_counts: dict[str, int] = {}
        max_iterations = graph.metadata.get("max_iterations", 100)
        step = start_step

        # Emit + transcript: execution started
        transcript.append(TranscriptEvent(
            type="started",
            message=f"Ejecutando grafo '{graph.name}' desde nodo '{entry_node_id}'",
            timestamp=time.time(),
            metadata={"graph_id": graph.id, "graph_name": graph.name},
        ).to_dict())

        await self._emit(EventType.SESSION_STARTED, data={
            "graph_id": graph.id, "graph_name": graph.name,
            "entry_node": entry_node_id,
        })

        # Hook: on_graph_start
        hook_result = await self._run_hook("on_graph_start", None, {
            "graph_id": graph.id, "graph_name": graph.name,
        })
        if hook_result.get("action") == "abort":
            return ExecutionResult(
                status="failed", state=state, trace=trace, transcript=transcript,
                error=f"Hook on_graph_start aborted: {hook_result.get('reason', '')}",
            )

        while node is not None:
            # Check for manual pause request
            if self._pause_requested:
                self._pause_requested = False
                # Create checkpoint before pausing
                await self._save_checkpoint(step, node.id, state, node.id)
                await self._emit(EventType.SESSION_INTERRUPTED, node_id=node.id, data={
                    "reason": "session_control",
                })
                return ExecutionResult(
                    status="interrupted", state=state, trace=trace, transcript=transcript,
                    interrupt_node_id=node.id,
                )

            # Loop detection
            visit_counts[node.id] = visit_counts.get(node.id, 0) + 1
            if visit_counts[node.id] > max_iterations:
                raise MaxIterationsError(
                    f"Node '{node.id}' exceeded max_iterations ({max_iterations})"
                )

            # Resolve inputs from data_map of incoming edge
            inputs = self._resolve_inputs(node.id, graph, state, trace)

            label = _tool_label(node.tool_type)

            # Hook: pre_block_exec
            hook_result = await self._run_hook("pre_block_exec", node.id, {
                "tool_type": node.tool_type, "inputs": inputs,
            })
            if hook_result.get("action") == "abort":
                await self._emit(EventType.HOOK_BLOCKED, node_id=node.id, data={
                    "hook_type": "pre_block_exec", "reason": hook_result.get("reason", ""),
                })
                return ExecutionResult(
                    status="failed", state=state, trace=trace, transcript=transcript,
                    error=f"Hook pre_block_exec aborted at '{node.id}': {hook_result.get('reason', '')}",
                )
            if hook_result.get("action") == "skip":
                state.set(node.id, hook_result.get("output", {}))
                node = self._resolve_next_node(node, state, graph)
                step += 1
                continue
            if "inputs" in hook_result:
                inputs = hook_result["inputs"]

            # Emit: block starting — include config hints for UI
            block_hint = self._build_block_hint(node)
            await self._emit(EventType.BLOCK_STARTED, node_id=node.id, data={
                "block_type": node.tool_type, "step": step,
                "label": label, "hint": block_hint,
            })

            # Transcript: block starting
            transcript.append(TranscriptEvent(
                type="block_start",
                message=f"{label} ({node.id})",
                timestamp=time.time(),
                node_id=node.id,
                metadata={"tool_type": node.tool_type},
            ).to_dict())

            # Check if this is a human_input block
            if node.tool_type == "logic/human_input":
                prompt_data = {
                    "prompt": node.config.get("prompt", inputs.get("prompt", "Requiere decision")),
                    "options": node.config.get("options", inputs.get("options")),
                    "timeout_minutes": node.config.get("timeout_minutes"),
                }
                # Create checkpoint before interrupting
                await self._save_checkpoint(step, node.id, state, node.id)
                await self._emit(EventType.INTERRUPT_CREATED, node_id=node.id, data=prompt_data)
                await self._emit(EventType.SESSION_INTERRUPTED, node_id=node.id, data={
                    "reason": "human_input_block",
                })

                transcript.append(TranscriptEvent(
                    type="block_start",
                    message=f"Esperando decision humana en '{node.id}'",
                    timestamp=time.time(),
                    node_id=node.id,
                ).to_dict())

                return ExecutionResult(
                    status="interrupted", state=state, trace=trace, transcript=transcript,
                    interrupt_node_id=node.id, interrupt_prompt=prompt_data,
                )

            # Execute block with retry
            retry_policy = self._get_retry_policy(node)
            trace_entry = {
                "node_id": node.id,
                "tool_type": node.tool_type,
                "started_at": time.time(),
                "status": "running",
                "retry_count": 0,
                "step": step,
            }

            output, exec_error = await self._execute_with_retry(
                node, inputs, context, retry_policy, trace_entry
            )

            trace_entry["finished_at"] = time.time()
            duration_ms = (trace_entry["finished_at"] - trace_entry["started_at"]) * 1000

            if exec_error is not None:
                # Hook: on_error
                hook_result = await self._run_hook("on_error", node.id, {
                    "tool_type": node.tool_type, "error": str(exec_error),
                    "retry_count": trace_entry["retry_count"],
                })

                trace_entry["status"] = "failed"
                trace_entry["error"] = str(exec_error)
                trace.append(trace_entry)

                # Emit: block error
                await self._emit(EventType.BLOCK_ERROR, node_id=node.id, data={
                    "error": str(exec_error), "block_type": node.tool_type,
                })

                transcript.append(TranscriptEvent(
                    type="error",
                    message=f"{label} fallo: {exec_error}",
                    timestamp=time.time(),
                    node_id=node.id,
                    metadata={
                        "tool_type": node.tool_type,
                        "error": str(exec_error),
                        "retries": trace_entry["retry_count"],
                        "on_failure": retry_policy.on_failure,
                    },
                ).to_dict())

                if hook_result.get("action") == "retry":
                    # Hook wants a retry — re-execute this node
                    continue

                if retry_policy.on_failure == "stop" or hook_result.get("action") == "abort":
                    transcript.append(TranscriptEvent(
                        type="completed",
                        message=f"Ejecucion detenida por error en '{node.id}'",
                        timestamp=time.time(),
                    ).to_dict())
                    await self._emit(EventType.SESSION_FAILED, data={
                        "error": str(exec_error), "last_node": node.id,
                    })
                    return ExecutionResult(
                        status="failed", state=state, trace=trace,
                        transcript=transcript, error=str(exec_error),
                    )
                elif retry_policy.on_failure == "skip":
                    state.set(node.id, {})
                    step += 1
                    await self._save_checkpoint(step, node.id, state, None)
                    node = self._resolve_next_node(node, state, graph)
                    continue
                elif retry_policy.on_failure == "route_to_error":
                    state.set(node.id, {"__error__": str(exec_error)})
                    error_edge = self._find_error_edge(node.id, graph)
                    if error_edge:
                        step += 1
                        await self._save_checkpoint(step, node.id, state, error_edge.target)
                        node = graph.get_node(error_edge.target)
                    else:
                        await self._emit(EventType.SESSION_FAILED, data={
                            "error": f"No error edge for node '{node.id}'",
                        })
                        return ExecutionResult(
                            status="failed", state=state, trace=trace,
                            transcript=transcript,
                            error=f"No error edge for node '{node.id}'",
                        )
                    continue
            else:
                trace_entry["status"] = "success"
                trace.append(trace_entry)
                is_revisit = visit_counts[node.id] > 1
                state.set(node.id, output, overwrite=is_revisit)

                # Hook: post_block_exec
                hook_result = await self._run_hook("post_block_exec", node.id, {
                    "tool_type": node.tool_type, "outputs": output,
                    "duration_ms": duration_ms,
                })
                if "outputs" in hook_result:
                    output = hook_result["outputs"]
                    state.set(node.id, output, overwrite=True)

                # Emit: block completed
                await self._emit(EventType.BLOCK_COMPLETED, node_id=node.id, data={
                    "outputs": output, "duration_ms": duration_ms,
                    "block_type": node.tool_type,
                })

                # Transcript: block completed
                output_summary = self._summarize_output(output)
                transcript.append(TranscriptEvent(
                    type="block_end",
                    message=f"{label} completado en {duration_ms:.0f}ms{output_summary}",
                    timestamp=time.time(),
                    node_id=node.id,
                    metadata={"tool_type": node.tool_type, "duration_ms": duration_ms},
                ).to_dict())

            # Checkpoint after successful execution (REGLA-12)
            step += 1
            next_node = self._resolve_next_node(node, state, graph)

            await self._save_checkpoint(
                step, node.id, state,
                next_node.id if next_node else None,
            )

            # Transcript: decision at conditional edges
            outgoing = graph.get_outgoing_edges(node.id)
            conditional = [
                e for e in outgoing
                if e.condition and e.condition.get("type") != "on_error"
            ]
            if len(conditional) > 1 and next_node:
                transcript.append(TranscriptEvent(
                    type="decision",
                    message=f"Condicion evaluada: siguiendo hacia '{next_node.id}'",
                    timestamp=time.time(),
                    node_id=node.id,
                    metadata={"next_node": next_node.id, "alternatives": len(conditional)},
                ).to_dict())

            node = next_node

        # Hook: on_graph_end
        await self._run_hook("on_graph_end", None, {
            "blocks_executed": len(trace),
        })

        # Transcript + emit: execution completed
        transcript.append(TranscriptEvent(
            type="completed",
            message=f"Ejecucion completada — {len(trace)} herramientas ejecutadas",
            timestamp=time.time(),
            metadata={"blocks_executed": len(trace)},
        ).to_dict())

        await self._emit(EventType.SESSION_COMPLETED, data={
            "blocks_executed": len(trace),
            "duration_ms": sum(
                t.get("finished_at", 0) - t.get("started_at", 0)
                for t in trace
            ) * 1000,
        })

        return ExecutionResult(
            status="completed", state=state, trace=trace, transcript=transcript,
        )

    # --- Internal helpers ---

    async def _emit(
        self,
        event_type: EventType,
        *,
        node_id: str | None = None,
        data: dict[str, Any] | None = None,
    ) -> None:
        """Emit event if emitter is configured."""
        if self._emitter is None:
            return
        await self._emitter.emit(ExecutionEvent(
            type=event_type,
            session_id=self._session_id,
            node_id=node_id,
            data=data or {},
        ))

    async def _save_checkpoint(
        self,
        step: int,
        node_id: str,
        state: SharedState,
        cursor_position: str | None,
    ) -> None:
        """Save checkpoint if callback is configured (REGLA-12)."""
        if self._checkpoint is None:
            return
        snapshot = state.snapshot()
        checkpoint_id = await self._checkpoint(
            self._session_id, step, node_id, snapshot, cursor_position,
        )
        await self._emit(EventType.CHECKPOINT_CREATED, node_id=node_id, data={
            "step_number": step, "checkpoint_id": checkpoint_id,
        })

    async def _run_hook(
        self,
        hook_type: str,
        node_id: str | None,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        """Run hook if handler is configured. Returns action dict."""
        if self._hook is None:
            return {"action": "continue"}
        try:
            result = await asyncio.wait_for(
                self._hook(hook_type, node_id, data),
                timeout=30.0,  # REGLA-22: 30s timeout
            )
            if result.get("action") != "continue":
                await self._emit(EventType.HOOK_FIRED, node_id=node_id, data={
                    "hook_type": hook_type, "action": result.get("action"),
                })
            return result
        except asyncio.TimeoutError:
            # REGLA-22: timeout = continue
            return {"action": "continue"}
        except Exception as e:
            import logging
            logging.getLogger(__name__).warning("Hook '%s' failed: %s", hook_type, e)
            return {"action": "continue"}

    async def _execute_with_retry(
        self,
        node: NodeDef,
        inputs: dict[str, Any],
        context: Any,
        policy: RetryPolicy,
        trace_entry: dict,
    ) -> tuple[dict[str, Any] | None, Exception | None]:
        """Execute block with retry policy. Returns (output, error)."""
        last_error: Exception | None = None

        for attempt in range(1 + policy.max_retries):
            try:
                # Hook: pre_llm_call (for AI blocks)
                if node.tool_type.startswith("ai/"):
                    hook_result = await self._run_hook("pre_llm_call", node.id, {
                        "tool_type": node.tool_type, "inputs": inputs,
                    })
                    if hook_result.get("action") == "skip":
                        return hook_result.get("output", {}), None
                    if "inputs" in hook_result:
                        inputs = hook_result["inputs"]

                # Set node_id on context for event emission (FEAT-008)
                if hasattr(context, "node_id"):
                    context.node_id = node.id

                output = await self._executor(node, inputs, context)

                # Hook: post_llm_call (for AI blocks)
                if node.tool_type.startswith("ai/"):
                    hook_result = await self._run_hook("post_llm_call", node.id, {
                        "tool_type": node.tool_type, "output": output,
                    })
                    if "output" in hook_result:
                        output = hook_result["output"]

                return output, None
            except Exception as e:
                last_error = e
                trace_entry["retry_count"] = attempt

                if attempt < policy.max_retries:
                    delay = self._compute_delay(policy, attempt)
                    if delay > 0:
                        await asyncio.sleep(delay)

        return None, last_error

    def _compute_delay(self, policy: RetryPolicy, attempt: int) -> float:
        if policy.backoff == "none":
            return 0
        elif policy.backoff == "linear":
            return policy.initial_delay_seconds * (attempt + 1)
        elif policy.backoff == "exponential":
            return policy.initial_delay_seconds * (2**attempt)
        return 0

    def _get_retry_policy(self, node: NodeDef) -> RetryPolicy:
        raw = node.config.get("retry_policy")
        if raw is None:
            return RetryPolicy()
        return RetryPolicy(**raw)

    def _resolve_next_node(
        self, current: NodeDef, state: SharedState, graph: GraphDef
    ) -> NodeDef | None:
        """Evaluate outgoing edges and return next node."""
        outgoing = graph.get_outgoing_edges(current.id)
        if not outgoing:
            return None

        conditional: list[EdgeDef] = []
        unconditional: list[EdgeDef] = []

        for edge in outgoing:
            if edge.condition is not None:
                if edge.condition.get("type") == "on_error":
                    continue
                conditional.append(edge)
            else:
                unconditional.append(edge)

        # Evaluate conditional edges — first match wins (REGLA-03)
        output = state.get(current.id)
        for edge in conditional:
            if self._evaluate_condition(edge.condition, output):
                return graph.get_node(edge.target)

        if unconditional:
            return graph.get_node(unconditional[0].target)

        raise GraphExecutionError(
            f"No matching edge from node '{current.id}'. "
            f"Output: {output}, Conditions: {[e.condition for e in conditional]}"
        )

    def _evaluate_condition(self, condition: dict, output: dict[str, Any]) -> bool:
        field_name = condition["field"]
        op = condition["op"]
        expected = condition["value"]
        actual = output.get(field_name)

        if op == "eq":
            return actual == expected
        elif op == "neq":
            return actual != expected
        elif op == "gt":
            return actual > expected
        elif op == "lt":
            return actual < expected
        elif op == "gte":
            return actual >= expected
        elif op == "lte":
            return actual <= expected
        elif op == "in":
            return actual in expected
        elif op == "contains":
            return expected in actual
        else:
            raise GraphExecutionError(f"Unknown condition operator: '{op}'")

    def _resolve_ref(self, ref: str, state: SharedState) -> Any | None:
        """Resolve a reference from state.

        Supports:
        - 'node_id' → returns the entire output dict of the node
        - 'node_id.key' → returns a specific field
        - 'node_id.key.subkey' → nested access
        """
        parts = ref.split(".")
        ref_node = parts[0]
        if ref_node not in state:
            return None
        value = state.get(ref_node)
        for part in parts[1:]:
            if isinstance(value, dict) and part in value:
                value = value[part]
            else:
                return None
        return value

    def _resolve_inputs(
        self,
        node_id: str,
        graph: GraphDef,
        state: SharedState,
        trace: list[dict],
    ) -> dict[str, Any]:
        """Resolve inputs for a node from data_map of incoming edges.

        Supports two modes:
        - Direct reference: "node.key" → maps the value directly
        - Template string: "text with ${node.key} interpolation" → builds a string
        """
        import re

        inputs: dict[str, Any] = {}

        for edge in graph.edges:
            if edge.target == node_id and edge.data_map:
                for input_key, source_ref in edge.data_map.items():
                    # Template mode: contains ${...} patterns
                    if "${" in source_ref:
                        def _replace(m: re.Match) -> str:
                            val = self._resolve_ref(m.group(1), state)
                            if val is None:
                                return m.group(0)  # keep original if unresolved
                            if isinstance(val, str):
                                return val
                            import json
                            return json.dumps(val, ensure_ascii=False, default=str)

                        result = re.sub(r"\$\{([^}]+)\}", _replace, source_ref)
                        inputs[input_key] = result
                    else:
                        # Direct reference mode
                        value = self._resolve_ref(source_ref, state)
                        if value is not None:
                            inputs[input_key] = value

        return inputs

    def _summarize_output(self, output: dict[str, Any]) -> str:
        if not output:
            return ""
        keys = list(output.keys())
        if len(keys) == 1:
            val = output[keys[0]]
            if isinstance(val, str) and len(val) > 80:
                return f" — {keys[0]}: {val[:80]}..."
            return f" — {keys[0]}: {val}"
        return f" — {len(keys)} campos: {', '.join(keys[:4])}"

    @staticmethod
    def _build_block_hint(node: NodeDef) -> str:
        """Build a human-readable hint for what the block is doing."""
        cfg = node.config or {}
        if node.tool_type == "data/db_write":
            table = cfg.get("table", "default")
            return f"Tabla: {table}"
        elif node.tool_type == "data/db_read":
            table = cfg.get("table", "default")
            return f"Tabla: {table}"
        elif node.tool_type == "ai/llm_call":
            model = cfg.get("model", "default")
            return f"Modelo: {model}"
        elif node.tool_type == "data/web_scrape":
            query = cfg.get("query", "")
            if query:
                return f"Query: {query[:60]}..."
            urls = cfg.get("urls", "")
            if urls:
                return f"URL: {urls[:60]}"
            return ""
        elif node.tool_type == "output/response":
            fmt = cfg.get("format", "text")
            return f"Formato: {fmt}"
        return ""

    def _find_error_edge(self, node_id: str, graph: GraphDef) -> EdgeDef | None:
        for edge in graph.get_outgoing_edges(node_id):
            if edge.condition and edge.condition.get("type") == "on_error":
                return edge
        return None


class RegistryExecutor:
    """Bridge between GraphRunner and ToolRegistry.

    Looks up tool class by node.tool_type, instantiates it,
    and calls tool.run() with validated inputs and config defaults.
    """

    def __init__(self, registry: Any) -> None:
        self._registry = registry

    async def __call__(
        self, node: NodeDef, inputs: dict[str, Any], context: Any
    ) -> dict[str, Any]:
        tool_cls = self._registry.get(node.tool_type)
        if tool_cls is None:
            raise GraphExecutionError(
                f"Block type '{node.tool_type}' not found in registry"
            )
        tool = tool_cls()
        return await tool.run(inputs, node.config, context)
