"""Tests for GraphSuggester — LLM-powered graph self-improvement."""

import json

import pytest

from datamirai_engine.intelligence.suggester import GraphSuggester, SUGGESTION_TYPES


# --- Helpers ---


def _make_graph(nodes=None, edges=None):
    """Create a minimal graph_def dict."""
    return {
        "id": "graph-1",
        "name": "Test Graph",
        "nodes": nodes or [
            {"id": "n1", "tool_type": "trigger/manual", "config": {}},
            {"id": "n2", "tool_type": "ai/llm_call", "config": {"prompt": "Summarize: {{input}}"}},
            {"id": "n3", "tool_type": "output/response", "config": {}},
        ],
        "edges": edges or [
            {"id": "e1", "source": "n1", "target": "n2"},
            {"id": "e2", "source": "n2", "target": "n3", "data_map": {"summary": "{{n2.output}}"}},
        ],
    }


def _make_reflections():
    """Create sample reflections."""
    return [
        {
            "reflection_type": "failure_pattern",
            "node_id": "n2",
            "insight": "LLM call times out frequently under heavy load",
            "confidence": 0.9,
        },
        {
            "reflection_type": "optimization",
            "node_id": "n2",
            "insight": "High token usage — prompt could be more concise",
            "confidence": 0.75,
        },
    ]


def _make_stats():
    """Create sample trace stats."""
    return {
        "total_traces": 100,
        "success_rate_by_node": {"n1": 1.0, "n2": 0.7, "n3": 0.95},
        "avg_duration_by_node": {"n1": 5.0, "n2": 3200.0, "n3": 10.0},
        "top_errors": [
            {"error_type": "timeout", "error_message": "LLM call timed out", "count": 20},
        ],
    }


# --- Tests ---


def test_build_prompt_includes_graph():
    """Prompt contains serialized graph nodes and edges."""
    suggester = GraphSuggester()
    graph = _make_graph()
    prompt = suggester._build_prompt(graph, [], [], {})

    assert "ai/llm_call" in prompt
    assert "trigger/manual" in prompt
    assert "output/response" in prompt
    assert "n1" in prompt
    assert "n2" in prompt
    assert "prompt_change" in prompt
    assert "add_node" in prompt
    assert "modify_config" in prompt


def test_build_prompt_includes_reflections():
    """Prompt includes reflection data."""
    suggester = GraphSuggester()
    graph = _make_graph()
    reflections = _make_reflections()
    prompt = suggester._build_prompt(graph, reflections, [], {})

    assert "failure_pattern" in prompt
    assert "optimization" in prompt
    assert "LLM call times out" in prompt


def test_build_prompt_includes_stats():
    """Prompt includes trace statistics."""
    suggester = GraphSuggester()
    graph = _make_graph()
    stats = _make_stats()
    prompt = suggester._build_prompt(graph, [], [], stats)

    assert "total_traces" in prompt
    assert "3200.0" in prompt


def test_parse_suggestions_valid_json():
    """Valid JSON array is parsed correctly."""
    suggester = GraphSuggester()
    response = json.dumps([
        {
            "suggestion_type": "prompt_change",
            "target_node_id": "n2",
            "description": "Simplify the LLM prompt to reduce token usage",
            "proposed_change": {"field": "prompt", "new_value": "Summarize briefly: {{input}}"},
            "reason": "High token usage detected in reflections",
            "confidence": 0.85,
        },
        {
            "suggestion_type": "modify_config",
            "target_node_id": "n2",
            "description": "Increase timeout for LLM calls",
            "proposed_change": {"field": "timeout", "new_value": 120},
            "reason": "Frequent timeouts in execution traces",
            "confidence": 0.9,
        },
    ])

    result = suggester._parse_suggestions(response)

    assert len(result) == 2
    assert result[0]["suggestion_type"] == "prompt_change"
    assert result[0]["target_node_id"] == "n2"
    assert result[0]["confidence"] == 0.85
    assert result[1]["suggestion_type"] == "modify_config"


def test_parse_suggestions_invalid_fallback():
    """Invalid JSON returns empty list (not crash)."""
    suggester = GraphSuggester()
    result = suggester._parse_suggestions("This is not valid JSON at all.")

    assert result == []


def test_parse_suggestions_filters_invalid_types():
    """Suggestions with unknown types are filtered out."""
    suggester = GraphSuggester()
    response = json.dumps([
        {"suggestion_type": "prompt_change", "target_node_id": "n1", "description": "Good", "confidence": 0.8},
        {"suggestion_type": "invented_type", "target_node_id": "n2", "description": "Bad", "confidence": 0.5},
    ])

    result = suggester._parse_suggestions(response)

    assert len(result) == 1
    assert result[0]["suggestion_type"] == "prompt_change"


def test_parse_suggestions_strips_markdown():
    """JSON wrapped in markdown code fences is parsed correctly."""
    suggester = GraphSuggester()
    response = '```json\n[{"suggestion_type": "modify_config", "target_node_id": "n1", "description": "Tweak", "proposed_change": {"field": "x", "new_value": 1}, "reason": "perf", "confidence": 0.7}]\n```'

    result = suggester._parse_suggestions(response)

    assert len(result) == 1
    assert result[0]["suggestion_type"] == "modify_config"


def test_parse_suggestions_clamps_confidence():
    """Confidence values are clamped to [0.0, 1.0]."""
    suggester = GraphSuggester()
    response = json.dumps([
        {"suggestion_type": "prompt_change", "target_node_id": "n1", "description": "Over", "confidence": 1.5},
        {"suggestion_type": "prompt_change", "target_node_id": "n2", "description": "Under", "confidence": -0.3},
    ])

    result = suggester._parse_suggestions(response)

    assert result[0]["confidence"] == 1.0
    assert result[1]["confidence"] == 0.0


def test_apply_prompt_change():
    """apply_suggestion handles prompt_change correctly."""
    suggester = GraphSuggester()
    graph = _make_graph()
    suggestion = {
        "suggestion_type": "prompt_change",
        "target_node_id": "n2",
        "proposed_change": {"field": "prompt", "new_value": "Be concise: {{input}}"},
    }

    result = suggester.apply_suggestion(graph, suggestion)

    # Original is not mutated
    assert graph["nodes"][1]["config"]["prompt"] == "Summarize: {{input}}"
    # New graph has updated prompt
    n2 = next(n for n in result["nodes"] if n["id"] == "n2")
    assert n2["config"]["prompt"] == "Be concise: {{input}}"


def test_apply_modify_config():
    """apply_suggestion handles modify_config correctly."""
    suggester = GraphSuggester()
    graph = _make_graph()
    suggestion = {
        "suggestion_type": "modify_config",
        "target_node_id": "n2",
        "proposed_change": {"field": "temperature", "new_value": 0.3},
    }

    result = suggester.apply_suggestion(graph, suggestion)

    n2 = next(n for n in result["nodes"] if n["id"] == "n2")
    assert n2["config"]["temperature"] == 0.3
    # Original prompt is preserved
    assert n2["config"]["prompt"] == "Summarize: {{input}}"


def test_apply_add_node():
    """apply_suggestion handles add_node correctly."""
    suggester = GraphSuggester()
    graph = _make_graph()
    suggestion = {
        "suggestion_type": "add_node",
        "target_node_id": None,
        "proposed_change": {
            "tool_type": "logic/condition",
            "config": {"expression": "{{n2.output}} != ''"},
            "insert_after": "n2",
        },
    }

    result = suggester.apply_suggestion(graph, suggestion)

    # Should have 4 nodes now
    assert len(result["nodes"]) == 4
    new_node = result["nodes"][-1]
    assert new_node["tool_type"] == "logic/condition"

    # The edge n2 -> n3 should be split into n2 -> new_node -> n3
    sources = [e["source"] for e in result["edges"]]
    targets = [e["target"] for e in result["edges"]]
    assert new_node["id"] in targets
    assert new_node["id"] in sources


def test_apply_remove_node():
    """apply_suggestion handles remove_node with reconnection."""
    suggester = GraphSuggester()
    graph = _make_graph()
    suggestion = {
        "suggestion_type": "remove_node",
        "target_node_id": "n2",
        "proposed_change": {"reason": "Unnecessary step"},
    }

    result = suggester.apply_suggestion(graph, suggestion)

    # Node n2 is removed
    node_ids = [n["id"] for n in result["nodes"]]
    assert "n2" not in node_ids
    assert "n1" in node_ids
    assert "n3" in node_ids

    # Edges are reconnected: n1 -> n3
    assert len(result["edges"]) == 1
    assert result["edges"][0]["source"] == "n1"
    assert result["edges"][0]["target"] == "n3"


def test_apply_modify_data_map():
    """apply_suggestion handles modify_data_map correctly."""
    suggester = GraphSuggester()
    graph = _make_graph()
    suggestion = {
        "suggestion_type": "modify_data_map",
        "target_node_id": "n3",
        "proposed_change": {
            "edge_source": "n2",
            "edge_target": "n3",
            "new_data_map": {"result": "{{n2.text}}"},
        },
    }

    result = suggester.apply_suggestion(graph, suggestion)

    edge = next(e for e in result["edges"] if e["source"] == "n2" and e["target"] == "n3")
    assert edge["data_map"] == {"result": "{{n2.text}}"}


@pytest.mark.asyncio
async def test_suggest_with_mock_llm():
    """suggest() orchestrates prompt -> LLM -> parse correctly."""
    suggester = GraphSuggester()
    graph = _make_graph()
    reflections = _make_reflections()
    stats = _make_stats()

    llm_response = json.dumps([
        {
            "suggestion_type": "prompt_change",
            "target_node_id": "n2",
            "description": "Simplify prompt",
            "proposed_change": {"field": "prompt", "new_value": "Summarize briefly: {{input}}"},
            "reason": "Reduce token usage",
            "confidence": 0.85,
        },
    ])

    captured_prompts = []

    async def mock_llm(prompt: str) -> str:
        captured_prompts.append(prompt)
        return llm_response

    result = await suggester.suggest(
        graph_def=graph,
        reflections=reflections,
        rules=[],
        trace_stats=stats,
        llm_call_fn=mock_llm,
    )

    assert len(result) == 1
    assert result[0]["suggestion_type"] == "prompt_change"
    assert result[0]["target_node_id"] == "n2"
    assert len(captured_prompts) == 1
    assert "ai/llm_call" in captured_prompts[0]


@pytest.mark.asyncio
async def test_suggest_empty_graph_returns_empty():
    """No nodes -> no suggestions, LLM is not called."""
    suggester = GraphSuggester()
    call_count = 0

    async def mock_llm(prompt: str) -> str:
        nonlocal call_count
        call_count += 1
        return "[]"

    result = await suggester.suggest(
        graph_def={"nodes": [], "edges": []},
        reflections=[],
        rules=[],
        trace_stats={},
        llm_call_fn=mock_llm,
    )

    assert result == []
    assert call_count == 0


def test_apply_suggestion_does_not_mutate_original():
    """Applying a suggestion returns a new dict, original is unchanged."""
    suggester = GraphSuggester()
    graph = _make_graph()
    original_prompt = graph["nodes"][1]["config"]["prompt"]

    suggestion = {
        "suggestion_type": "prompt_change",
        "target_node_id": "n2",
        "proposed_change": {"field": "prompt", "new_value": "New prompt"},
    }

    result = suggester.apply_suggestion(graph, suggestion)

    # Original unchanged
    assert graph["nodes"][1]["config"]["prompt"] == original_prompt
    # Result has new value
    n2 = next(n for n in result["nodes"] if n["id"] == "n2")
    assert n2["config"]["prompt"] == "New prompt"
