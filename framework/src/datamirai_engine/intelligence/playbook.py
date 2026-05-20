"""Playbook — rule-based prompt injection from agent playbook rules.

Takes accumulated rules (from reflections or manual input) and injects
the most relevant ones into LLM prompts before execution.

Rules can be auto-disabled when harmful feedback exceeds helpful (REGLA-47/48).
"""

from __future__ import annotations

from typing import Any, Callable

# REGLA-46: max rules to inject per prompt
MAX_RULES_PER_PROMPT = 10

# Supported rule types
RULE_TYPES = ("optimization", "guardrail", "preference")


class PlaybookManager:
    """Manages playbook rule injection and feedback loop.

    This class is framework-only — it does NOT depend on any DB layer.
    The caller provides callbacks/repos for persistence.

    Usage:
        pm = PlaybookManager()
        rules = pm.get_relevant_rules(
            all_rules=[...],
            prompt_text="...",
            search_fn=my_fts_fn,
        )
        prompt = pm.inject_rules("original prompt", rules)
        # After execution:
        pm.record_feedback(rule, "helpful", update_fn=my_update_fn)
        pm.check_auto_disable(rule, disable_fn=my_disable_fn)
    """

    def get_relevant_rules(
        self,
        all_rules: list[dict[str, Any]],
        prompt_text: str | None = None,
        node_id: str | None = None,
        tool_type: str | None = None,
        search_fn: Callable[[str], list[dict[str, Any]]] | None = None,
        max_rules: int = MAX_RULES_PER_PROMPT,
    ) -> list[dict[str, Any]]:
        """Select the most relevant active rules for a given context.

        Args:
            all_rules: Full list of rules (pre-filtered to active status by caller).
            prompt_text: Current prompt text for FTS matching.
            node_id: Optional node_id filter (matches rule.node_filter).
            tool_type: Optional tool_type filter (matches rule.node_filter).
            search_fn: Optional FTS search callback(query) -> matching rules.
            max_rules: Max rules to return (REGLA-46, default 10).

        Returns:
            List of rule dicts, capped at max_rules.
        """
        # If FTS search function provided and we have prompt text, use it
        if search_fn and prompt_text:
            candidates = search_fn(prompt_text)
        else:
            candidates = list(all_rules)

        # Filter by node_filter if node_id or tool_type provided
        if node_id or tool_type:
            filtered = []
            for rule in candidates:
                nf = rule.get("node_filter", "") or ""
                if not nf:
                    # No filter = applies to all nodes
                    filtered.append(rule)
                elif node_id and nf == node_id:
                    filtered.append(rule)
                elif tool_type and nf == tool_type:
                    filtered.append(rule)
            candidates = filtered

        # Only active rules
        candidates = [r for r in candidates if r.get("status") == "active"]

        # Cap at max_rules (REGLA-46)
        return candidates[:max_rules]

    def inject_rules(self, prompt: str, rules: list[dict[str, Any]]) -> str:
        """Inject rules as a [PLAYBOOK RULES] block at the beginning of the prompt.

        Args:
            prompt: Original prompt text.
            rules: List of rule dicts (must have 'rule_text' and 'rule_type' keys).

        Returns:
            Modified prompt with rules injected, or original if no rules.
        """
        if not rules:
            return prompt

        lines = ["[PLAYBOOK RULES]"]
        for i, rule in enumerate(rules, 1):
            rtype = rule.get("rule_type", "preference")
            text = rule.get("rule_text", "")
            lines.append(f"{i}. [{rtype.upper()}] {text}")
        lines.append("[/PLAYBOOK RULES]")
        lines.append("")

        block = "\n".join(lines)
        return f"{block}\n{prompt}"

    def record_feedback(
        self,
        rule: dict[str, Any],
        outcome: str,
        update_fn: Callable[[str, str, int], None] | None = None,
    ) -> dict[str, Any]:
        """Record feedback for a rule application.

        Args:
            rule: The rule dict (must have 'id', 'helpful_count', 'harmful_count').
            outcome: Either 'helpful' or 'harmful'.
            update_fn: Optional callback(rule_id, field, new_value) for persistence.

        Returns:
            Updated rule dict with incremented counter.
        """
        rule = dict(rule)  # Don't mutate original

        if outcome == "helpful":
            rule["helpful_count"] = rule.get("helpful_count", 0) + 1
            if update_fn:
                update_fn(rule["id"], "helpful_count", rule["helpful_count"])
        elif outcome == "harmful":
            rule["harmful_count"] = rule.get("harmful_count", 0) + 1
            if update_fn:
                update_fn(rule["id"], "harmful_count", rule["harmful_count"])

        return rule

    def check_auto_disable(
        self,
        rule: dict[str, Any],
        disable_fn: Callable[[str], None] | None = None,
    ) -> bool:
        """Check if a rule should be auto-disabled based on feedback.

        REGLA-47: If harmful_count > helpful_count → disabled.
        REGLA-48: Manual rules (reflection_id is None) are NEVER auto-disabled.

        Args:
            rule: The rule dict.
            disable_fn: Optional callback(rule_id) to persist the disable.

        Returns:
            True if rule was disabled, False otherwise.
        """
        # REGLA-48: manual rules (no reflection_id) are never auto-disabled
        if rule.get("reflection_id") is None:
            return False

        helpful = rule.get("helpful_count", 0)
        harmful = rule.get("harmful_count", 0)

        # REGLA-47: harmful > helpful → disable
        if harmful > helpful:
            if disable_fn:
                disable_fn(rule["id"])
            return True

        return False
