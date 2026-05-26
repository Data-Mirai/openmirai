"""Context Compiler — 5-phase context assembly for LLM nodes."""

from __future__ import annotations


class ContextCompiler:
    """Assembles optimized context for LLM calls in 5 phases.

    Phases (in order):
    1. Agent Identity — system prompt, personality, role
    2. Playbook Rules — relevant learned rules (FTS search)
    3. Session Context — data from data_map (current execution data)
    4. Long-term Memory — relevant past learnings (semantic search)
    5. Compression — truncate if exceeding token budget
    """

    def __init__(self, config: dict | None = None):
        self._config = config or {}

    def compile(
        self,
        *,
        prompt: str,
        session_context: str,
        identity_prompt: str | None = None,
        playbook_rules: list[dict] | None = None,
        memory_results: list[dict] | None = None,
        max_tokens: int | None = None,
    ) -> str:
        """Assemble context from all phases into final prompt.

        Returns assembled prompt string ready for LLM.
        """
        parts: list[str] = []

        # Phase 1: Identity
        if identity_prompt and self._config.get("enable_identity", True):
            parts.append(f"[AGENT IDENTITY]\n{identity_prompt}\n[/AGENT IDENTITY]")

        # Phase 2: Playbook Rules (max 5 by default)
        if playbook_rules and self._config.get("enable_playbook", True):
            max_rules = self._config.get("max_playbook_rules", 5)
            rules = playbook_rules[:max_rules]
            if rules:
                rules_text = "\n".join(
                    f"- {r.get('rule_text', r) if isinstance(r, dict) else r}"
                    for r in rules
                )
                parts.append(f"[PLAYBOOK RULES]\n{rules_text}\n[/PLAYBOOK RULES]")

        # Phase 3: Session Context (ALWAYS included — REGLA-56)
        parts.append(session_context)

        # Phase 4: Long-term Memory
        if memory_results and self._config.get("enable_memory", True):
            max_mem = self._config.get("max_memory_results", 3)
            memories = memory_results[:max_mem]
            if memories:
                mem_text = "\n".join(
                    f"- {m.get('summary', m) if isinstance(m, dict) else m}"
                    for m in memories
                )
                parts.append(f"[AGENT MEMORY]\n{mem_text}\n[/AGENT MEMORY]")

        assembled = "\n\n".join(parts)

        # Phase 5: Compression if needed
        if max_tokens:
            # Rough estimation: 1 token ~ 4 chars
            max_chars = max_tokens * 4
            threshold = self._config.get("compression_threshold", 0.6)
            char_limit = int(max_chars * threshold)
            if len(assembled) > char_limit:
                assembled = self._compress(assembled, char_limit)

        return prompt + "\n\n" + assembled

    def _compress(self, text: str, max_chars: int) -> str:
        """Compress by truncating middle sections, preserving start and end."""
        if len(text) <= max_chars:
            return text
        # Keep first 60% and last 20%, cut middle
        keep_start = int(max_chars * 0.6)
        keep_end = int(max_chars * 0.2)
        return (
            text[:keep_start]
            + "\n\n[... context compressed ...]\n\n"
            + text[-keep_end:]
        )
