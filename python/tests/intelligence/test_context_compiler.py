"""Tests for ContextCompiler — 5-phase context assembly for LLM nodes."""

import pytest

from datamirai_engine.intelligence.context_compiler import ContextCompiler


# --- Helpers ---

def _make_rule(text: str = "Validate inputs") -> dict:
    return {"rule_text": text, "rule_type": "guardrail", "status": "active"}


def _make_memory(summary: str = "Previous session learned X") -> dict:
    return {"summary": summary, "tags": ["test"]}


# --- Tests ---


def test_compile_all_phases():
    """All 5 phases present in output when all inputs provided."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="Analyze this data",
        session_context="=== Session Context ===\ndata here",
        identity_prompt="You are a research assistant.",
        playbook_rules=[_make_rule("Rule A"), _make_rule("Rule B")],
        memory_results=[_make_memory("Memory 1"), _make_memory("Memory 2")],
    )

    # Phase 1: Identity
    assert "[AGENT IDENTITY]" in result
    assert "You are a research assistant." in result
    assert "[/AGENT IDENTITY]" in result

    # Phase 2: Playbook
    assert "[PLAYBOOK RULES]" in result
    assert "- Rule A" in result
    assert "- Rule B" in result
    assert "[/PLAYBOOK RULES]" in result

    # Phase 3: Session context (always)
    assert "=== Session Context ===" in result
    assert "data here" in result

    # Phase 4: Memory
    assert "[AGENT MEMORY]" in result
    assert "- Memory 1" in result
    assert "- Memory 2" in result
    assert "[/AGENT MEMORY]" in result

    # Prompt is at the beginning
    assert result.startswith("Analyze this data")


def test_session_context_always_included():
    """REGLA-56: Session context is always present even if others disabled."""
    cc = ContextCompiler(config={
        "enable_identity": False,
        "enable_playbook": False,
        "enable_memory": False,
    })

    result = cc.compile(
        prompt="Do something",
        session_context="important session data",
        identity_prompt="identity",
        playbook_rules=[_make_rule()],
        memory_results=[_make_memory()],
    )

    assert "important session data" in result
    assert "[AGENT IDENTITY]" not in result
    assert "[PLAYBOOK RULES]" not in result
    assert "[AGENT MEMORY]" not in result


def test_disable_playbook():
    """enable_playbook=False suppresses rules block."""
    cc = ContextCompiler(config={"enable_playbook": False})

    result = cc.compile(
        prompt="prompt",
        session_context="context",
        playbook_rules=[_make_rule("Should not appear")],
    )

    assert "[PLAYBOOK RULES]" not in result
    assert "Should not appear" not in result
    assert "context" in result


def test_disable_memory():
    """enable_memory=False suppresses memory block."""
    cc = ContextCompiler(config={"enable_memory": False})

    result = cc.compile(
        prompt="prompt",
        session_context="context",
        memory_results=[_make_memory("Should not appear")],
    )

    assert "[AGENT MEMORY]" not in result
    assert "Should not appear" not in result
    assert "context" in result


def test_max_playbook_rules():
    """max_playbook_rules config is respected."""
    cc = ContextCompiler(config={"max_playbook_rules": 2})

    rules = [_make_rule(f"Rule {i}") for i in range(5)]
    result = cc.compile(
        prompt="prompt",
        session_context="context",
        playbook_rules=rules,
    )

    assert "- Rule 0" in result
    assert "- Rule 1" in result
    assert "- Rule 2" not in result
    assert "- Rule 3" not in result
    assert "- Rule 4" not in result


def test_max_memory_results():
    """max_memory_results config is respected."""
    cc = ContextCompiler(config={"max_memory_results": 1})

    memories = [_make_memory(f"Mem {i}") for i in range(5)]
    result = cc.compile(
        prompt="prompt",
        session_context="context",
        memory_results=memories,
    )

    assert "- Mem 0" in result
    assert "- Mem 1" not in result


def test_compression_triggered():
    """Large context gets compressed when exceeding token budget."""
    cc = ContextCompiler()

    # Create a large session context (~10000 chars)
    large_context = "data " * 2000  # 10000 chars

    result = cc.compile(
        prompt="prompt",
        session_context=large_context,
        max_tokens=100,  # 100 tokens * 4 chars * 0.6 threshold = 240 char limit
    )

    assert "[... context compressed ...]" in result


def test_no_compression_under_threshold():
    """Small context is not compressed."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="prompt",
        session_context="short data",
        max_tokens=100000,  # very large budget
    )

    assert "[... context compressed ...]" not in result
    assert "short data" in result


def test_identity_prompt():
    """Identity prompt included when provided and enabled."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="prompt",
        session_context="context",
        identity_prompt="You are a data analyst expert.",
    )

    assert "[AGENT IDENTITY]" in result
    assert "You are a data analyst expert." in result
    assert "[/AGENT IDENTITY]" in result


def test_empty_rules_and_memory():
    """No crash when rules and memory are empty lists."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="Just a prompt",
        session_context="Some context",
        playbook_rules=[],
        memory_results=[],
    )

    assert "Just a prompt" in result
    assert "Some context" in result
    assert "[PLAYBOOK RULES]" not in result
    assert "[AGENT MEMORY]" not in result


def test_none_config_uses_defaults():
    """None config uses default enabled settings."""
    cc = ContextCompiler(config=None)

    result = cc.compile(
        prompt="p",
        session_context="ctx",
        identity_prompt="id",
        playbook_rules=[_make_rule()],
        memory_results=[_make_memory()],
    )

    # All phases present with defaults
    assert "[AGENT IDENTITY]" in result
    assert "[PLAYBOOK RULES]" in result
    assert "[AGENT MEMORY]" in result
    assert "ctx" in result


def test_rule_as_string():
    """Rules can be plain strings instead of dicts."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="p",
        session_context="ctx",
        playbook_rules=["Plain string rule"],
    )

    assert "- Plain string rule" in result


def test_memory_as_string():
    """Memory results can be plain strings instead of dicts."""
    cc = ContextCompiler()

    result = cc.compile(
        prompt="p",
        session_context="ctx",
        memory_results=["Plain memory string"],
    )

    assert "- Plain memory string" in result
