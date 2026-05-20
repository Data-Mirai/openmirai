"""GraphSuggester — LLM-powered graph self-improvement suggestions.

Takes a graph definition, active reflections, operational rules, and trace
statistics, then produces structured suggestions for improving the graph.
"""

from __future__ import annotations

import copy
import json
from typing import Any, Callable, Awaitable

# Supported suggestion types
SUGGESTION_TYPES = (
    "prompt_change",
    "add_node",
    "modify_config",
    "modify_data_map",
    "remove_node",
)

# REGLA-50: max reflections per prompt to keep size manageable
MAX_REFLECTIONS_PER_PROMPT = 30


class GraphSuggester:
    """Generates and applies graph improvement suggestions via LLM analysis.

    Usage:
        suggester = GraphSuggester()
        suggestions = await suggester.suggest(
            graph_def={...},
            reflections=[...],
            rules=[...],
            trace_stats={...},
            llm_call_fn=my_llm_call,
        )
        # Apply a suggestion
        new_graph = suggester.apply_suggestion(graph_def, suggestions[0])
    """

    def _build_prompt(
        self,
        graph_def: dict[str, Any],
        reflections: list[dict[str, Any]],
        rules: list[dict[str, Any]],
        trace_stats: dict[str, Any],
    ) -> str:
        """Build a structured prompt for the LLM to generate improvement suggestions."""
        # Serialize graph — keep only structural info
        serialized_nodes = []
        for n in graph_def.get("nodes", []):
            entry: dict[str, Any] = {
                "id": n.get("id", "unknown"),
                "tool_type": n.get("tool_type", "unknown"),
            }
            config = n.get("config", {})
            if config:
                entry["config"] = config
            serialized_nodes.append(entry)

        serialized_edges = []
        for e in graph_def.get("edges", []):
            edge_entry: dict[str, Any] = {
                "source": e.get("source", ""),
                "target": e.get("target", ""),
            }
            if e.get("data_map"):
                edge_entry["data_map"] = e["data_map"]
            if e.get("condition"):
                edge_entry["condition"] = e["condition"]
            serialized_edges.append(edge_entry)

        graph_json = json.dumps(
            {"nodes": serialized_nodes, "edges": serialized_edges},
            ensure_ascii=False,
            indent=2,
        )

        # Serialize reflections (cap to REGLA-50)
        capped_reflections = reflections[:MAX_REFLECTIONS_PER_PROMPT]
        reflections_json = json.dumps(
            [
                {
                    "type": r.get("reflection_type", r.get("type", "")),
                    "node_id": r.get("node_id"),
                    "insight": r.get("insight", ""),
                    "confidence": r.get("confidence", 0.5),
                }
                for r in capped_reflections
            ],
            ensure_ascii=False,
            indent=2,
        )

        # Serialize rules
        rules_json = json.dumps(rules, ensure_ascii=False, indent=2) if rules else "[]"

        # Serialize stats
        stats_json = json.dumps(trace_stats, ensure_ascii=False, indent=2) if trace_stats else "{}"

        return f"""Analyze this agent graph and suggest improvements based on the reflections and execution statistics.

GRAPH:
{graph_json}

REFLECTIONS (insights from past executions):
{reflections_json}

OPERATIONAL RULES:
{rules_json}

EXECUTION STATISTICS:
{stats_json}

Respond ONLY with a JSON array. Each element must have:
- "suggestion_type": one of "prompt_change", "add_node", "modify_config", "modify_data_map", "remove_node"
- "target_node_id": the node id this suggestion affects (or null for add_node)
- "description": a concise description of the improvement (1-2 sentences)
- "proposed_change": an object with the specific change to apply:
  - For prompt_change: {{"field": "prompt", "new_value": "..."}}
  - For add_node: {{"tool_type": "...", "config": {{...}}, "insert_after": "node_id"}}
  - For modify_config: {{"field": "config_key", "new_value": "..."}}
  - For modify_data_map: {{"edge_source": "...", "edge_target": "...", "new_data_map": {{...}}}}
  - For remove_node: {{"reason": "..."}}
- "reason": why this change would improve the graph (based on the reflections/stats)
- "confidence": a float between 0.0 and 1.0

Rules:
- Focus on the most impactful improvements
- Use reflections to identify problem areas (failure_pattern, optimization)
- Use statistics to find slow or error-prone nodes
- Suggest prompt improvements for LLM nodes with low success rates
- Suggest adding error handling nodes where failures are common
- Suggest removing unnecessary nodes that add latency without value
- Return an empty array [] if no meaningful improvements can be suggested
- Return at most 5 suggestions, ordered by confidence (highest first)
- Return ONLY valid JSON, no markdown, no explanation"""

    def _parse_suggestions(self, llm_output: str) -> list[dict[str, Any]]:
        """Parse LLM response as JSON array. Falls back to empty list on parse error."""
        text = llm_output.strip()

        # Strip markdown code fences if present
        if text.startswith("```"):
            lines = text.split("\n")
            if lines[0].startswith("```"):
                lines = lines[1:]
            if lines and lines[-1].strip() == "```":
                lines = lines[:-1]
            text = "\n".join(lines).strip()

        try:
            parsed = json.loads(text)
            if not isinstance(parsed, list):
                raise ValueError("Expected JSON array")

            validated = []
            for item in parsed:
                if not isinstance(item, dict):
                    continue
                stype = item.get("suggestion_type", "")
                if stype not in SUGGESTION_TYPES:
                    continue
                validated.append({
                    "suggestion_type": stype,
                    "target_node_id": item.get("target_node_id"),
                    "description": str(item.get("description", "")),
                    "proposed_change": item.get("proposed_change", {}),
                    "reason": str(item.get("reason", "")),
                    "confidence": min(1.0, max(0.0, float(item.get("confidence", 0.5)))),
                })
            return validated

        except (json.JSONDecodeError, ValueError, TypeError):
            return []

    async def suggest(
        self,
        graph_def: dict[str, Any],
        reflections: list[dict[str, Any]],
        rules: list[dict[str, Any]],
        trace_stats: dict[str, Any],
        llm_call_fn: Callable[..., Awaitable[str]],
    ) -> list[dict[str, Any]]:
        """Generate improvement suggestions for a graph.

        Args:
            graph_def: The graph definition dict (nodes, edges).
            reflections: Active reflections from the Reflector.
            rules: Operational rules/constraints.
            trace_stats: Aggregated trace statistics.
            llm_call_fn: Async function that takes (prompt: str) and returns str.

        Returns:
            List of suggestion dicts. Empty list if no suggestions.
        """
        if not graph_def.get("nodes"):
            return []

        prompt = self._build_prompt(graph_def, reflections, rules, trace_stats)
        llm_output = await llm_call_fn(prompt)
        suggestions = self._parse_suggestions(llm_output)

        return suggestions

    def apply_suggestion(
        self,
        graph_def: dict[str, Any],
        suggestion: dict[str, Any],
    ) -> dict[str, Any]:
        """Apply a single suggestion to a graph definition.

        Args:
            graph_def: The current graph definition (will NOT be mutated).
            suggestion: A suggestion dict from suggest().

        Returns:
            A new graph_def with the suggestion applied.
        """
        result = copy.deepcopy(graph_def)
        stype = suggestion.get("suggestion_type", "")
        target = suggestion.get("target_node_id")
        change = suggestion.get("proposed_change", {})

        if stype == "prompt_change":
            # Update a config field (typically "prompt") on the target node
            field = change.get("field", "prompt")
            new_value = change.get("new_value", "")
            for node in result.get("nodes", []):
                if node.get("id") == target:
                    if "config" not in node:
                        node["config"] = {}
                    node["config"][field] = new_value
                    break

        elif stype == "modify_config":
            # Update a specific config key on the target node
            field = change.get("field", "")
            new_value = change.get("new_value")
            if field:
                for node in result.get("nodes", []):
                    if node.get("id") == target:
                        if "config" not in node:
                            node["config"] = {}
                        node["config"][field] = new_value
                        break

        elif stype == "add_node":
            # Add a new node and connect it
            tool_type = change.get("tool_type", "logic/condition")
            config = change.get("config", {})
            insert_after = change.get("insert_after")
            # Generate a node id
            existing_ids = {n.get("id", "") for n in result.get("nodes", [])}
            counter = len(existing_ids) + 1
            while f"n{counter}" in existing_ids:
                counter += 1
            new_id = f"n{counter}"

            new_node = {
                "id": new_id,
                "tool_type": tool_type,
                "config": config,
                "position": {"x": 300, "y": 60 + len(result.get("nodes", [])) * 120},
            }
            result.setdefault("nodes", []).append(new_node)

            # If insert_after specified, rewire edges
            if insert_after:
                edges = result.get("edges", [])
                new_edges = []
                connected = False
                for edge in edges:
                    if edge.get("source") == insert_after and not connected:
                        original_target = edge.get("target")
                        # Edge from insert_after → new_node
                        new_edges.append({
                            "id": f"e-sug-a-{new_id}",
                            "source": insert_after,
                            "target": new_id,
                        })
                        # Edge from new_node → original target
                        new_edges.append({
                            "id": f"e-sug-b-{new_id}",
                            "source": new_id,
                            "target": original_target,
                        })
                        connected = True
                    else:
                        new_edges.append(edge)
                result["edges"] = new_edges

        elif stype == "modify_data_map":
            # Update the data_map on a specific edge
            source = change.get("edge_source", "")
            edge_target = change.get("edge_target", "")
            new_data_map = change.get("new_data_map", {})
            for edge in result.get("edges", []):
                if edge.get("source") == source and edge.get("target") == edge_target:
                    edge["data_map"] = new_data_map
                    break

        elif stype == "remove_node":
            # Remove the target node and its edges
            if target:
                result["nodes"] = [
                    n for n in result.get("nodes", []) if n.get("id") != target
                ]
                # Remove edges that reference the removed node
                # and reconnect if possible
                incoming = []
                outgoing = []
                other_edges = []
                for edge in result.get("edges", []):
                    if edge.get("target") == target:
                        incoming.append(edge)
                    elif edge.get("source") == target:
                        outgoing.append(edge)
                    else:
                        other_edges.append(edge)

                # Reconnect: each incoming source → each outgoing target
                reconnected = []
                for inc in incoming:
                    for out in outgoing:
                        reconnected.append({
                            "id": f"e-recon-{inc.get('source')}-{out.get('target')}",
                            "source": inc.get("source"),
                            "target": out.get("target"),
                        })
                result["edges"] = other_edges + reconnected

        return result
