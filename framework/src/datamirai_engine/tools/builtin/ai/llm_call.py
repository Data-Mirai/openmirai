"""LLM Call tool — data processor that generates grounded responses.

The LLM block is NOT a free-form text generator. It processes data collected
by previous nodes in the session (web scrape, DB queries, etc.) and produces
natural language output grounded in that data.

Session Context: all inputs mapped via data_map (except 'prompt') are
automatically collected, formatted, and injected as context so the LLM
responds based on real data — never invented.

FEAT-008: Supports streaming via context.llm.stream() with LLM_TOKEN event
emission for real-time typewriter rendering in the UI.

Output Schema: When output_schema is configured, the tool enforces structured
JSON output. The LLM is instructed to respond ONLY with JSON matching the
schema. The response is parsed and validated. On failure, auto-retry up to
max_retries times with error feedback. This guarantees idempotent, API-like
responses that downstream nodes can consume reliably.
"""

from __future__ import annotations

import json as _json
import logging
import re as _re
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

logger = logging.getLogger(__name__)


class LLMCallTool(BaseTool):
    spec = ToolSpec(
        tool_type="ai/llm_call",
        version="2.1.0",
        display_name="LLM Call",
        description=(
            "Processes session data through an LLM to produce grounded, "
            "natural language responses. Requires data from previous nodes."
        ),
        category="ai",
        icon="brain",
        intents=[
            "analizar, resumir, clasificar o transformar datos recopilados por nodos anteriores",
            "generar reportes, informes, listas o conclusiones a partir de datos reales",
            "responder preguntas basandose en contexto de sesion (no conocimiento general)",
            "procesar texto: extraer entidades, sentimiento, traducir, reformatear",
        ],
        inputs=[
            ToolInput(name="prompt", type="string", required=False,
                      description="Instruction for the LLM on how to process the data"),
        ],
        outputs=[
            ToolOutput(name="response", type="string", description="Raw LLM text response"),
            ToolOutput(name="tokens_used", type="number"),
            ToolOutput(name="structured_output", type="object",
                       description="Parsed JSON when output_schema is defined, null otherwise"),
            ToolOutput(name="schema_valid", type="boolean",
                       description="Whether response matched output_schema"),
        ],
        config=[
            ConfigField(name="model", type="string", default="claude"),
            ConfigField(name="temperature", type="number", default=0.7),
            ConfigField(name="max_tokens", type="number", default=1024),
            ConfigField(name="prompt", type="string", default="", required=True,
                        description="Instruccion para el LLM sobre como procesar los datos"),
            ConfigField(name="num_ctx", type="number", default=None,
                        description="Context window size (tokens). Leave empty for model default."),
            ConfigField(name="max_context_length", type="number", default=12000,
                        description="Max chars for session context. Content beyond this is truncated proportionally. 0 = unlimited."),
            ConfigField(name="stream", type="boolean", default=True,
                        description="Enable token-by-token streaming (FEAT-008). Disable for faster non-streaming."),
            ConfigField(name="output_schema", type="string", default="",
                        description="JSON Schema para forzar respuesta estructurada. El LLM responde SOLO con JSON que cumpla este schema. Si falla, reintenta automaticamente."),
            ConfigField(name="schema_retries", type="number", default=2,
                        description="Max reintentos si la respuesta no cumple el output_schema (0 = sin reintentos)."),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        # --- Resolve prompt ---
        prompt = inputs.get("prompt") or config.get("prompt", "")
        if not prompt:
            raise ValueError("prompt is required (via input or config)")

        if not isinstance(prompt, str):
            prompt = _json.dumps(prompt, ensure_ascii=False, default=str)

        # --- Build session context from all non-prompt inputs ---
        session_data = {k: v for k, v in inputs.items() if k != "prompt"}

        if not session_data:
            raise ValueError(
                "session context is empty — LLM requires data from previous nodes "
                "to generate grounded responses. Connect data sources via data_map."
            )

        max_ctx_len = int(config.get("max_context_length", 12000))
        session_context = self._format_session_context(session_data, max_length=max_ctx_len)

        # --- Parse output_schema if defined ---
        output_schema = self._parse_output_schema(config.get("output_schema", ""))
        max_retries = int(config.get("schema_retries", 2)) if output_schema else 0

        # --- If output_schema, enrich prompt with JSON format instructions ---
        effective_prompt = prompt
        if output_schema:
            effective_prompt = self._enrich_prompt_with_schema(prompt, output_schema)

        # --- Build system prompt (agent-level + node-level) ---
        system_prompt_parts: list[str] = []
        if context and getattr(context, "system_prompt", None):
            system_prompt_parts.append(context.system_prompt)
        node_system_prompt = config.get("system_prompt", "")
        if node_system_prompt:
            system_prompt_parts.append(node_system_prompt)

        # Combine system_prompt + session_context into a single context
        # for the LLM adapter (context = system message role)
        if system_prompt_parts:
            combined_context = "\n\n".join(system_prompt_parts) + "\n\n" + session_context
        else:
            combined_context = session_context

        # --- Prepare LLM call params ---
        num_ctx = config.get("num_ctx")
        llm_kwargs: dict[str, Any] = {
            "model": config.get("model", "claude"),
            "prompt": effective_prompt,
            "context": combined_context,
            "temperature": config.get("temperature", 0.7),
            "max_tokens": config.get("max_tokens", 1024),
        }
        if num_ctx:
            llm_kwargs["num_ctx"] = num_ctx

        # --- Execute with optional schema validation + retries ---
        for attempt in range(max_retries + 1):
            raw_result = await self._call_llm(context, config, llm_kwargs)

            if not output_schema:
                # No schema — return raw response
                return {
                    **raw_result,
                    "structured_output": None,
                    "schema_valid": False,
                }

            # --- Validate against schema ---
            parsed, errors = self._validate_response(raw_result["response"], output_schema)
            if parsed is not None and not errors:
                return {
                    "response": raw_result["response"],
                    "tokens_used": raw_result["tokens_used"],
                    "structured_output": parsed,
                    "schema_valid": True,
                }

            # --- Retry with error feedback ---
            if attempt < max_retries:
                logger.warning(
                    "output_schema validation failed (attempt %d/%d): %s",
                    attempt + 1, max_retries + 1, errors,
                )
                llm_kwargs["prompt"] = self._build_retry_prompt(
                    effective_prompt, raw_result["response"], errors
                )
                # Disable streaming for retries (faster)
                llm_kwargs["_force_no_stream"] = True

        # --- All retries exhausted — return best effort ---
        logger.error("output_schema validation failed after %d attempts", max_retries + 1)
        # Try to salvage partial JSON
        parsed, _ = self._validate_response(raw_result["response"], output_schema)
        return {
            "response": raw_result["response"],
            "tokens_used": raw_result["tokens_used"],
            "structured_output": parsed,
            "schema_valid": parsed is not None,
        }

    async def _call_llm(
        self, context: ExecutionContext, config: dict[str, Any], llm_kwargs: dict[str, Any]
    ) -> dict[str, Any]:
        """Execute a single LLM call (streaming or non-streaming)."""
        force_no_stream = llm_kwargs.pop("_force_no_stream", False)
        stream_enabled = config.get("stream", True) and not force_no_stream

        if stream_enabled and hasattr(context.llm, "stream"):
            try:
                return await self._stream_call(context, llm_kwargs)
            except Exception as e:
                logger.warning("LLM stream() failed, falling back to call(): %s", e)

        result = await context.llm.call(**llm_kwargs)
        # NormalizedResponse is a dataclass — use attribute access
        return {
            "response": result.response,
            "tokens_used": result.tokens_used.get("output", 0) if isinstance(result.tokens_used, dict) else 0,
        }

    async def _stream_call(
        self, context: ExecutionContext, llm_kwargs: dict[str, Any]
    ) -> dict[str, Any]:
        """Execute LLM with streaming, emitting LLM_TOKEN events for each chunk."""
        accumulated = ""
        tokens_used = 0
        emitter = getattr(context, "events", None)
        session_id = getattr(context, "session_id", None)
        node_id = getattr(context, "node_id", None)

        async for chunk in context.llm.stream(
            prompt=llm_kwargs["prompt"],
            model=llm_kwargs.get("model"),
            context=llm_kwargs.get("context"),
            temperature=llm_kwargs.get("temperature"),
            max_tokens=llm_kwargs.get("max_tokens"),
            num_ctx=llm_kwargs.get("num_ctx"),
        ):
            # Support both NormalizedChunk (dataclass) and raw dicts
            delta = getattr(chunk, "delta", "") if not isinstance(chunk, dict) else chunk.get("delta", "")
            done = getattr(chunk, "done", False) if not isinstance(chunk, dict) else chunk.get("done", False)
            accumulated += delta

            # Emit LLM_TOKEN event for real-time rendering (fire-and-forget, REGLA-119)
            if emitter and delta and session_id:
                try:
                    from datamirai_engine.core.events import EventType, ExecutionEvent
                    await emitter.emit(ExecutionEvent(
                        type=EventType.LLM_TOKEN,
                        session_id=session_id,
                        node_id=node_id,
                        data={"delta": delta, "accumulated_length": len(accumulated)},
                    ))
                except Exception:
                    pass  # Never block execution for event emission

            if done:
                chunk_tokens = getattr(chunk, "tokens_used", None) if not isinstance(chunk, dict) else chunk.get("tokens_used")
                if isinstance(chunk_tokens, dict):
                    tokens_used = chunk_tokens.get("output", 0)
                elif isinstance(chunk_tokens, (int, float)):
                    tokens_used = int(chunk_tokens)
                else:
                    tokens_used = 0

        # Emit LLM_COMPLETED event
        if emitter and session_id:
            try:
                from datamirai_engine.core.events import EventType, ExecutionEvent
                await emitter.emit(ExecutionEvent(
                    type=EventType.LLM_COMPLETED,
                    session_id=session_id,
                    node_id=node_id,
                    data={
                        "response_length": len(accumulated),
                        "tokens_used": tokens_used,
                    },
                ))
            except Exception:
                pass

        return {
            "response": accumulated,
            "tokens_used": tokens_used,
        }

    @staticmethod
    def _format_session_context(data: dict[str, Any], *, max_length: int = 0) -> str:
        """Format session data into structured context for the LLM.

        If max_length > 0 and total exceeds it, truncates values
        proportionally — largest values lose the most, each keeps
        at least 200 chars.
        """
        entries: list[tuple[str, str]] = []
        for key, value in data.items():
            if isinstance(value, str):
                formatted = value
            else:
                formatted = _json.dumps(value, ensure_ascii=False, default=str, indent=2)
            entries.append((key, formatted))

        if max_length > 0:
            # Overhead: wrapper lines + key labels + separators
            overhead = 60  # "=== Session Context/End ===" markers
            overhead += sum(len(k) + 6 for k, _ in entries)  # "[key]:\n"
            overhead += max(0, len(entries) - 1) * 2  # "\n\n" between

            content_budget = max_length - overhead
            total_content = sum(len(v) for _, v in entries)

            if total_content > content_budget > 0:
                ratio = content_budget / total_content
                truncated: list[tuple[str, str]] = []
                for key, value in entries:
                    cap = max(200, int(len(value) * ratio))
                    if len(value) > cap:
                        logger.info(
                            "Truncating session context key '%s': %d → %d chars",
                            key, len(value), cap,
                        )
                        value = value[:cap] + f"\n[... truncated {len(value)} → {cap} chars]"
                    truncated.append((key, value))
                entries = truncated

        parts = [f"[{key}]:\n{value}" for key, value in entries]
        return "=== Session Context ===\n" + "\n\n".join(parts) + "\n=== End Session Context ==="

    # ── Output Schema ──────────────────────────────────────────────

    @staticmethod
    def _parse_output_schema(raw: str | dict | None) -> dict | None:
        """Parse output_schema from config (JSON string or dict). Returns None if empty."""
        if not raw:
            return None
        if isinstance(raw, dict):
            return raw
        if isinstance(raw, str):
            raw = raw.strip()
            if not raw:
                return None
            try:
                return _json.loads(raw)
            except _json.JSONDecodeError:
                logger.warning("output_schema is not valid JSON, ignoring: %s", raw[:100])
                return None
        return None

    @staticmethod
    def _enrich_prompt_with_schema(prompt: str, schema: dict) -> str:
        """Prepend/append JSON schema instructions to the prompt."""
        schema_str = _json.dumps(schema, ensure_ascii=False, indent=2)
        return (
            f"{prompt}\n\n"
            "=== OUTPUT FORMAT (MANDATORY) ===\n"
            "Respond ONLY with a valid JSON object matching this schema. "
            "No markdown, no explanation, no text before or after the JSON.\n\n"
            f"{schema_str}\n\n"
            "=== END OUTPUT FORMAT ==="
        )

    @staticmethod
    def _extract_json_from_response(text: str) -> str | None:
        """Extract JSON from LLM response, handling markdown code blocks."""
        text = text.strip()

        # Try direct parse first
        if text.startswith("{") or text.startswith("["):
            return text

        # Extract from ```json ... ``` blocks
        match = _re.search(r"```(?:json)?\s*\n?([\s\S]*?)\n?\s*```", text)
        if match:
            return match.group(1).strip()

        # Find first { ... } or [ ... ] block
        for opener, closer in [("{", "}"), ("[", "]")]:
            start = text.find(opener)
            if start == -1:
                continue
            depth = 0
            for i in range(start, len(text)):
                if text[i] == opener:
                    depth += 1
                elif text[i] == closer:
                    depth -= 1
                if depth == 0:
                    return text[start:i + 1]

        return None

    def _validate_response(
        self, response: str, schema: dict
    ) -> tuple[dict | None, list[str]]:
        """Parse response as JSON and validate against schema.

        Returns (parsed_dict, errors). If errors is empty, validation passed.
        """
        errors: list[str] = []

        json_str = self._extract_json_from_response(response)
        if json_str is None:
            return None, ["Response does not contain valid JSON"]

        try:
            parsed = _json.loads(json_str)
        except _json.JSONDecodeError as e:
            return None, [f"JSON parse error: {e}"]

        if not isinstance(parsed, dict):
            return parsed, ["Expected JSON object, got " + type(parsed).__name__]

        # Validate required fields
        required = schema.get("required", [])
        properties = schema.get("properties", {})

        for field in required:
            if field not in parsed:
                errors.append(f"Missing required field: '{field}'")

        # Validate types for present fields
        for field, field_schema in properties.items():
            if field not in parsed:
                continue
            value = parsed[field]
            expected_type = field_schema.get("type")
            if expected_type and not self._check_type(value, expected_type):
                errors.append(
                    f"Field '{field}' expected type '{expected_type}', got '{type(value).__name__}'"
                )

            # Validate enum
            enum_values = field_schema.get("enum")
            if enum_values and value not in enum_values:
                errors.append(
                    f"Field '{field}' must be one of {enum_values}, got '{value}'"
                )

        return parsed, errors

    @staticmethod
    def _check_type(value: Any, expected: str) -> bool:
        """Check if a value matches a JSON Schema type."""
        type_map = {
            "string": str,
            "number": (int, float),
            "integer": int,
            "boolean": bool,
            "array": list,
            "object": dict,
        }
        expected_types = type_map.get(expected)
        if expected_types is None:
            return True  # Unknown type, skip validation
        return isinstance(value, expected_types)

    @staticmethod
    def _build_retry_prompt(original_prompt: str, bad_response: str, errors: list[str]) -> str:
        """Build a retry prompt with error feedback."""
        error_list = "\n".join(f"- {e}" for e in errors)
        return (
            f"{original_prompt}\n\n"
            "=== RETRY — PREVIOUS RESPONSE WAS INVALID ===\n"
            f"Your previous response had these errors:\n{error_list}\n\n"
            f"Previous response (DO NOT repeat this):\n{bad_response[:500]}\n\n"
            "Fix these errors and respond ONLY with valid JSON matching the schema.\n"
            "=== END RETRY ==="
        )
