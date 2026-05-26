"""Tests for AgentSpec — YAML/JSON serialization, validation, and conversion."""

from __future__ import annotations

import pytest

from datamirai_engine.core.agent_spec import AgentSpec
from datamirai_engine.core.graph import GraphDef


SAMPLE_YAML = """\
name: document-pipeline
description: Procesa documentos, transcribe audio, genera embeddings
version: v3
graph:
  nodes:
    - id: t1
      tool_type: trigger/webhook
      config:
        method: POST
        path: /webhooks/document-pipeline
    - id: n1
      tool_type: ai/transcribe
      config:
        model: whisper-large-v3
    - id: n2
      tool_type: ai/embeddings
    - id: n3
      tool_type: data/db_write
      config:
        table: documents
  edges:
    - id: e1
      source: t1
      target: n1
    - id: e2
      source: n1
      target: n2
    - id: e3
      source: n2
      target: n3
triggers:
  - type: webhook
    path: /webhooks/document-pipeline
    auth: api_key
  - type: schedule
    interval_seconds: 3600
config:
  max_iterations: 50
  retry:
    max_retries: 3
    backoff: exponential
    on_failure: route_to_error
  timeout_ms: 60000
resources:
  - docs-s3
  - pgvector
  - anthropic-prod
metadata:
  created_by: mateo
  tags:
    - documents
    - transcription
    - embeddings
"""

MINIMAL_YAML = """\
name: minimal-agent
graph:
  nodes:
    - id: n1
      tool_type: trigger/manual
  edges: []
"""


# --- Parsing ---


class TestParsing:
    def test_from_yaml_full(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        assert spec.name == "document-pipeline"
        assert spec.version == "v3"
        assert len(spec.graph.nodes) == 4
        assert len(spec.graph.edges) == 3
        assert len(spec.triggers) == 2
        assert spec.config.max_iterations == 50
        assert spec.config.retry.backoff == "exponential"
        assert spec.resources == ["docs-s3", "pgvector", "anthropic-prod"]
        assert spec.metadata["created_by"] == "mateo"

    def test_from_yaml_minimal(self):
        spec = AgentSpec.from_yaml(MINIMAL_YAML)
        assert spec.name == "minimal-agent"
        assert spec.version == "v1"
        assert len(spec.graph.nodes) == 1
        assert len(spec.graph.edges) == 0
        assert spec.config.max_iterations == 50

    def test_from_dict(self):
        data = {
            "name": "test-agent",
            "graph": {
                "nodes": [{"id": "n1", "tool_type": "trigger/manual"}],
                "edges": [],
            },
        }
        spec = AgentSpec.from_dict(data)
        assert spec.name == "test-agent"

    def test_from_yaml_invalid_not_mapping(self):
        with pytest.raises(ValueError, match="mapping"):
            AgentSpec.from_yaml("- item1\n- item2")

    def test_from_yaml_invalid_missing_name(self):
        with pytest.raises(Exception):
            AgentSpec.from_yaml("graph:\n  nodes: []\n  edges: []")


# --- Validation ---


class TestValidation:
    def test_duplicate_node_ids(self):
        with pytest.raises(ValueError, match="Duplicate node IDs"):
            AgentSpec(
                name="bad",
                graph={
                    "nodes": [
                        {"id": "n1", "tool_type": "a"},
                        {"id": "n1", "tool_type": "b"},
                    ],
                    "edges": [],
                },
            )

    def test_duplicate_edge_ids(self):
        with pytest.raises(ValueError, match="Duplicate edge IDs"):
            AgentSpec(
                name="bad",
                graph={
                    "nodes": [
                        {"id": "n1", "tool_type": "a"},
                        {"id": "n2", "tool_type": "b"},
                    ],
                    "edges": [
                        {"id": "e1", "source": "n1", "target": "n2"},
                        {"id": "e1", "source": "n2", "target": "n1"},
                    ],
                },
            )

    def test_edge_references_nonexistent_source(self):
        with pytest.raises(ValueError, match="non-existent source"):
            AgentSpec(
                name="bad",
                graph={
                    "nodes": [{"id": "n1", "tool_type": "a"}],
                    "edges": [{"id": "e1", "source": "nope", "target": "n1"}],
                },
            )

    def test_edge_references_nonexistent_target(self):
        with pytest.raises(ValueError, match="non-existent target"):
            AgentSpec(
                name="bad",
                graph={
                    "nodes": [{"id": "n1", "tool_type": "a"}],
                    "edges": [{"id": "e1", "source": "n1", "target": "nope"}],
                },
            )


# --- Roundtrip ---


class TestRoundtrip:
    def test_yaml_roundtrip(self):
        original = AgentSpec.from_yaml(SAMPLE_YAML)
        yaml_out = original.to_yaml()
        restored = AgentSpec.from_yaml(yaml_out)

        assert original.name == restored.name
        assert original.version == restored.version
        assert len(original.graph.nodes) == len(restored.graph.nodes)
        assert len(original.graph.edges) == len(restored.graph.edges)
        assert len(original.triggers) == len(restored.triggers)
        assert original.config.max_iterations == restored.config.max_iterations
        assert original.resources == restored.resources

    def test_dict_roundtrip(self):
        original = AgentSpec.from_yaml(SAMPLE_YAML)
        d = original.to_dict()
        restored = AgentSpec.from_dict(d)
        assert original.name == restored.name
        assert original.to_dict() == restored.to_dict()

    def test_yaml_to_dict_consistency(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        from_yaml = AgentSpec.from_yaml(spec.to_yaml())
        from_dict = AgentSpec.from_dict(spec.to_dict())
        assert from_yaml.to_dict() == from_dict.to_dict()


# --- Conversion to GraphDef ---


class TestToGraph:
    def test_to_graph_basic(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        graph = spec.to_graph(graph_id="g-test")

        assert isinstance(graph, GraphDef)
        assert graph.id == "g-test"
        assert graph.name == "document-pipeline"
        assert graph.version == "v3"
        assert len(graph.nodes) == 4
        assert len(graph.edges) == 3

    def test_to_graph_node_mapping(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        graph = spec.to_graph()

        node = graph.get_node("t1")
        assert node is not None
        assert node.tool_type == "trigger/webhook"
        assert node.config["method"] == "POST"

    def test_to_graph_edge_mapping(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        graph = spec.to_graph()

        edges = graph.get_outgoing_edges("t1")
        assert len(edges) == 1
        assert edges[0].target == "n1"

    def test_to_graph_auto_id(self):
        spec = AgentSpec.from_yaml(MINIMAL_YAML)
        graph = spec.to_graph()
        assert graph.id  # auto-generated, not empty

    def test_to_graph_with_conditions(self):
        yaml_str = """\
name: conditional-agent
graph:
  nodes:
    - id: n1
      tool_type: trigger/manual
    - id: n2
      tool_type: logic/condition
    - id: n3
      tool_type: ai/llm_call
  edges:
    - id: e1
      source: n1
      target: n2
    - id: e2
      source: n2
      target: n3
      condition:
        field: result
        operator: eq
        value: true
"""
        spec = AgentSpec.from_yaml(yaml_str)
        graph = spec.to_graph()
        edge = [e for e in graph.edges if e.id == "e2"][0]
        assert edge.condition == {"field": "result", "operator": "eq", "value": True}

    def test_to_graph_with_data_map(self):
        yaml_str = """\
name: mapped-agent
graph:
  nodes:
    - id: n1
      tool_type: ai/transcribe
    - id: n2
      tool_type: ai/llm_call
  edges:
    - id: e1
      source: n1
      target: n2
      data_map:
        n1.output.text: n2.input.prompt
"""
        spec = AgentSpec.from_yaml(yaml_str)
        graph = spec.to_graph()
        edge = graph.edges[0]
        assert edge.data_map == {"n1.output.text": "n2.input.prompt"}


# --- Triggers ---


class TestTriggers:
    def test_to_triggers_list(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        triggers = spec.to_triggers_list()

        assert len(triggers) == 2
        assert triggers[0]["type"] == "webhook"
        assert triggers[0]["path"] == "/webhooks/document-pipeline"
        assert triggers[0]["auth"] == "api_key"
        assert triggers[1]["type"] == "schedule"
        assert triggers[1]["interval_seconds"] == 3600

    def test_trigger_excludes_none_fields(self):
        spec = AgentSpec.from_yaml(MINIMAL_YAML)
        # No triggers defined
        assert spec.to_triggers_list() == []

    def test_single_trigger(self):
        yaml_str = """\
name: webhook-only
graph:
  nodes:
    - id: n1
      tool_type: trigger/webhook
  edges: []
triggers:
  - type: webhook
    path: /hooks/test
    method: POST
"""
        spec = AgentSpec.from_yaml(yaml_str)
        triggers = spec.to_triggers_list()
        assert len(triggers) == 1
        assert triggers[0] == {"type": "webhook", "path": "/hooks/test", "method": "POST"}


# --- Edge cases ---


class TestEdgeCases:
    def test_empty_graph(self):
        spec = AgentSpec(name="empty", graph={"nodes": [], "edges": []})
        assert len(spec.graph.nodes) == 0
        graph = spec.to_graph()
        assert len(graph.nodes) == 0

    def test_node_defaults(self):
        spec = AgentSpec(
            name="defaults",
            graph={
                "nodes": [{"id": "n1", "tool_type": "test/block"}],
                "edges": [],
            },
        )
        node = spec.graph.nodes[0]
        assert node.version == "1.0.0"
        assert node.config == {}
        assert node.position == {"x": 0.0, "y": 0.0}

    def test_config_defaults(self):
        spec = AgentSpec(name="defaults", graph={"nodes": [], "edges": []})
        assert spec.config.max_iterations == 50
        assert spec.config.retry.max_retries == 3
        assert spec.config.timeout_ms == 60000

    def test_metadata_preserved(self):
        spec = AgentSpec.from_yaml(SAMPLE_YAML)
        graph = spec.to_graph()
        assert graph.metadata["created_by"] == "mateo"
        assert "tags" in graph.metadata
