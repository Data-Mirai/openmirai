"""Tests for PlaybookManager — rule injection and feedback loop."""

import pytest

from datamirai_engine.intelligence.playbook import PlaybookManager, MAX_RULES_PER_PROMPT


# --- Helpers ---


def _make_rule(
    rule_id: str = "rule-1",
    rule_text: str = "Always check input validity",
    rule_type: str = "optimization",
    node_filter: str | None = None,
    helpful_count: int = 0,
    harmful_count: int = 0,
    status: str = "active",
    reflection_id: str | None = "refl-1",
) -> dict:
    return {
        "id": rule_id,
        "agent_id": "agent-1",
        "reflection_id": reflection_id,
        "rule_text": rule_text,
        "rule_type": rule_type,
        "node_filter": node_filter,
        "helpful_count": helpful_count,
        "harmful_count": harmful_count,
        "status": status,
    }


# --- inject_rules tests ---


def test_inject_rules_format():
    """Injected rules use [PLAYBOOK RULES] block format."""
    pm = PlaybookManager()
    rules = [
        _make_rule(rule_text="Validate input schema", rule_type="guardrail"),
        _make_rule(rule_id="rule-2", rule_text="Use concise responses", rule_type="preference"),
    ]

    result = pm.inject_rules("Original prompt here", rules)

    assert "[PLAYBOOK RULES]" in result
    assert "[/PLAYBOOK RULES]" in result
    assert "1. [GUARDRAIL] Validate input schema" in result
    assert "2. [PREFERENCE] Use concise responses" in result
    assert "Original prompt here" in result
    # Rules block should be at the beginning
    assert result.index("[PLAYBOOK RULES]") < result.index("Original prompt here")


def test_inject_empty_rules():
    """Empty rules list returns original prompt unchanged."""
    pm = PlaybookManager()
    prompt = "This is the original prompt"

    result = pm.inject_rules(prompt, [])

    assert result == prompt


def test_inject_rules_preserves_prompt():
    """Original prompt text is fully preserved after the rules block."""
    pm = PlaybookManager()
    rules = [_make_rule()]
    prompt = "Line 1\nLine 2\nLine 3"

    result = pm.inject_rules(prompt, rules)

    assert prompt in result


# --- get_relevant_rules tests ---


def test_max_rules_limit():
    """REGLA-46: get_relevant_rules caps at MAX_RULES_PER_PROMPT."""
    pm = PlaybookManager()
    rules = [_make_rule(rule_id=f"rule-{i}") for i in range(20)]

    result = pm.get_relevant_rules(all_rules=rules)

    assert len(result) == MAX_RULES_PER_PROMPT


def test_max_rules_custom_limit():
    """Custom max_rules parameter is respected."""
    pm = PlaybookManager()
    rules = [_make_rule(rule_id=f"rule-{i}") for i in range(10)]

    result = pm.get_relevant_rules(all_rules=rules, max_rules=3)

    assert len(result) == 3


def test_get_relevant_rules_filters_inactive():
    """Only active rules are returned."""
    pm = PlaybookManager()
    rules = [
        _make_rule(rule_id="r1", status="active"),
        _make_rule(rule_id="r2", status="disabled"),
        _make_rule(rule_id="r3", status="active"),
    ]

    result = pm.get_relevant_rules(all_rules=rules)

    assert len(result) == 2
    assert all(r["status"] == "active" for r in result)


def test_get_relevant_rules_node_filter():
    """node_id filter matches rule.node_filter."""
    pm = PlaybookManager()
    rules = [
        _make_rule(rule_id="r1", node_filter="llm-1"),
        _make_rule(rule_id="r2", node_filter="scrape-1"),
        _make_rule(rule_id="r3", node_filter=None),  # applies to all
    ]

    result = pm.get_relevant_rules(all_rules=rules, node_id="llm-1")

    assert len(result) == 2
    rule_ids = {r["id"] for r in result}
    assert "r1" in rule_ids
    assert "r3" in rule_ids


def test_get_relevant_rules_tool_type_filter():
    """tool_type filter matches rule.node_filter."""
    pm = PlaybookManager()
    rules = [
        _make_rule(rule_id="r1", node_filter="ai/llm_call"),
        _make_rule(rule_id="r2", node_filter="data/web_scrape"),
        _make_rule(rule_id="r3", node_filter=None),
    ]

    result = pm.get_relevant_rules(all_rules=rules, tool_type="ai/llm_call")

    assert len(result) == 2
    rule_ids = {r["id"] for r in result}
    assert "r1" in rule_ids
    assert "r3" in rule_ids


def test_get_relevant_rules_with_search_fn():
    """FTS search_fn is called when prompt_text is provided."""
    pm = PlaybookManager()

    searched_with = []

    def mock_search(query: str) -> list[dict]:
        searched_with.append(query)
        return [_make_rule(rule_id="found-1")]

    result = pm.get_relevant_rules(
        all_rules=[],
        prompt_text="analyze images",
        search_fn=mock_search,
    )

    assert len(searched_with) == 1
    assert searched_with[0] == "analyze images"
    assert len(result) == 1
    assert result[0]["id"] == "found-1"


# --- record_feedback tests ---


def test_record_feedback_helpful():
    """Helpful outcome increments helpful_count."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=3, harmful_count=1)

    updated = pm.record_feedback(rule, "helpful")

    assert updated["helpful_count"] == 4
    assert updated["harmful_count"] == 1


def test_record_feedback_harmful():
    """Harmful outcome increments harmful_count."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=3, harmful_count=1)

    updated = pm.record_feedback(rule, "harmful")

    assert updated["helpful_count"] == 3
    assert updated["harmful_count"] == 2


def test_record_feedback_calls_update_fn():
    """update_fn callback is invoked with correct args."""
    pm = PlaybookManager()
    rule = _make_rule(rule_id="r-42", helpful_count=5)

    calls = []

    def mock_update(rid, field, value):
        calls.append((rid, field, value))

    pm.record_feedback(rule, "helpful", update_fn=mock_update)

    assert len(calls) == 1
    assert calls[0] == ("r-42", "helpful_count", 6)


def test_record_feedback_does_not_mutate_original():
    """Original rule dict is not mutated."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=0)
    original_count = rule["helpful_count"]

    pm.record_feedback(rule, "helpful")

    assert rule["helpful_count"] == original_count


# --- check_auto_disable tests ---


def test_auto_disable_harmful_rule():
    """REGLA-47: harmful > helpful triggers auto-disable."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=2, harmful_count=5, reflection_id="refl-x")

    disabled_ids = []

    def mock_disable(rid):
        disabled_ids.append(rid)

    result = pm.check_auto_disable(rule, disable_fn=mock_disable)

    assert result is True
    assert rule["id"] in disabled_ids


def test_auto_disable_equal_counts_no_disable():
    """Equal helpful/harmful counts do NOT trigger disable."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=3, harmful_count=3, reflection_id="refl-x")

    result = pm.check_auto_disable(rule)

    assert result is False


def test_auto_disable_helpful_dominant_no_disable():
    """More helpful than harmful does NOT trigger disable."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=10, harmful_count=2, reflection_id="refl-x")

    result = pm.check_auto_disable(rule)

    assert result is False


def test_manual_rule_not_auto_disabled():
    """REGLA-48: manual rules (reflection_id=None) are NEVER auto-disabled."""
    pm = PlaybookManager()
    rule = _make_rule(helpful_count=0, harmful_count=100, reflection_id=None)

    disabled_ids = []

    def mock_disable(rid):
        disabled_ids.append(rid)

    result = pm.check_auto_disable(rule, disable_fn=mock_disable)

    assert result is False
    assert len(disabled_ids) == 0
