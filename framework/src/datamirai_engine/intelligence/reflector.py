"""Reflector — LLM-powered analysis of execution traces to extract insights.

Takes execution traces from completed sessions and produces structured
reflections: patterns (success/failure), optimizations, and anomalies.
"""

from __future__ import annotations

import json
from typing import Any, Callable, Awaitable

# Supported reflection types
REFLECTION_TYPES = ("success_pattern", "failure_pattern", "optimization", "anomaly")

# REGLA-45: max traces per batch to keep prompt size manageable
MAX_TRACES_PER_BATCH = 50


class Reflector:
    """Analyzes execution traces via LLM to extract actionable insights.

    Usage:
        reflector = Reflector()
        reflections = await reflector.reflect(
            agent_id="agent-1",
            agent_name="My Agent",
            traces=[...],
            llm_call_fn=my_llm_call,
        )
    """

    def _build_batch(self, traces: list[dict[str, Any]], max_traces: int = MAX_TRACES_PER_BATCH) -> list[dict[str, Any]]:
        """Limit traces to max_traces (REGLA-45). Returns most recent traces."""
        if len(traces) <= max_traces:
            return traces
        return traces[-max_traces:]

    def _build_prompt(self, traces: list[dict[str, Any]], agent_name: str) -> str:
        """Build a structured prompt for the LLM to analyze traces."""
        serialized_traces = []
        for t in traces:
            entry = {
                "node_id": t.get("node_id", "unknown"),
                "tool_type": t.get("tool_type", "unknown"),
                "status": t.get("status", "unknown"),
                "duration_ms": t.get("duration_ms"),
                "tokens_input": t.get("tokens_input", 0),
                "tokens_output": t.get("tokens_output", 0),
                "error_type": t.get("error_type"),
                "error_message": t.get("error_message"),
                "retry_count": t.get("retry_count", 0),
            }
            serialized_traces.append(entry)

        traces_json = json.dumps(serialized_traces, ensure_ascii=False, indent=2)

        return f"""Analyze the execution traces of agent "{agent_name}" and extract insights.

TRACES:
{traces_json}

Respond ONLY with a JSON array. Each element must have:
- "type": one of "success_pattern", "failure_pattern", "optimization", "anomaly"
- "node_id": the node_id this insight relates to (or null if general)
- "insight": a concise description of the pattern or recommendation (1-2 sentences)
- "confidence": a float between 0.0 and 1.0

Rules:
- Look for repeated errors on the same node (failure_pattern)
- Look for nodes that consistently succeed fast (success_pattern)
- Look for nodes with high token usage or slow duration (optimization)
- Look for unusual patterns like high retry counts (anomaly)
- Return an empty array [] if no meaningful insights can be extracted
- Return ONLY valid JSON, no markdown, no explanation"""

    def _parse_response(self, llm_output: str) -> list[dict[str, Any]]:
        """Parse LLM response as JSON array. Falls back to generic reflection on parse error."""
        text = llm_output.strip()

        # Strip markdown code fences if present
        if text.startswith("```"):
            lines = text.split("\n")
            # Remove first and last fence lines
            if lines[0].startswith("```"):
                lines = lines[1:]
            if lines and lines[-1].strip() == "```":
                lines = lines[:-1]
            text = "\n".join(lines).strip()

        try:
            parsed = json.loads(text)
            if not isinstance(parsed, list):
                raise ValueError("Expected JSON array")

            # Validate each reflection
            validated = []
            for item in parsed:
                if not isinstance(item, dict):
                    continue
                rtype = item.get("type", "")
                if rtype not in REFLECTION_TYPES:
                    continue
                validated.append({
                    "type": rtype,
                    "node_id": item.get("node_id"),
                    "insight": str(item.get("insight", "")),
                    "confidence": min(1.0, max(0.0, float(item.get("confidence", 0.5)))),
                })
            return validated

        except (json.JSONDecodeError, ValueError, TypeError):
            # Fallback: return a generic reflection from the raw text
            return [{
                "type": "anomaly",
                "node_id": None,
                "insight": f"LLM analysis could not be parsed. Raw output snippet: {text[:200]}",
                "confidence": 0.1,
            }]

    async def reflect(
        self,
        agent_id: str,
        agent_name: str,
        traces: list[dict[str, Any]],
        llm_call_fn: Callable[..., Awaitable[str]],
    ) -> list[dict[str, Any]]:
        """Run reflection on traces using the provided LLM call function.

        Args:
            agent_id: ID of the agent being analyzed.
            agent_name: Display name for the prompt.
            traces: List of trace dicts (from ExecutionTracer or DB).
            llm_call_fn: Async function that takes (prompt: str) and returns str.

        Returns:
            List of reflection dicts [{type, node_id, insight, confidence}].
            Empty list if no traces provided.
        """
        if not traces:
            return []

        batch = self._build_batch(traces)
        prompt = self._build_prompt(batch, agent_name)

        llm_output = await llm_call_fn(prompt)
        reflections = self._parse_response(llm_output)

        return reflections
