"""Tests for GraphDef, NodeDef, EdgeDef."""

import pytest
from pydantic import ValidationError

from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef


class TestNodeDef:
    def test_create_minimal(self):
        node = NodeDef(id="n1", tool_type="logic/condition")
        assert node.id == "n1"
        assert node.tool_type == "logic/condition"
        assert node.config == {}
        assert node.position == {"x": 0.0, "y": 0.0}

    def test_create_full(self):
        node = NodeDef(
            id="n1",
            tool_type="ai/llm_call",
            version="1.0.0",
            config={"model": "claude", "temperature": 0.7},
            position={"x": 100.0, "y": 200.0},
        )
        assert node.version == "1.0.0"
        assert node.config["model"] == "claude"
        assert node.position["x"] == 100.0

    def test_id_required(self):
        with pytest.raises(ValidationError):
            NodeDef(tool_type="logic/condition")

    def test_tool_type_required(self):
        with pytest.raises(ValidationError):
            NodeDef(id="n1")


class TestEdgeDef:
    def test_create_minimal(self):
        edge = EdgeDef(id="e1", source="n1", target="n2")
        assert edge.source == "n1"
        assert edge.target == "n2"
        assert edge.condition is None
        assert edge.data_map is None

    def test_create_with_condition(self):
        edge = EdgeDef(
            id="e1",
            source="n1",
            target="n2",
            condition={"field": "output.status", "op": "eq", "value": "success"},
        )
        assert edge.condition["field"] == "output.status"

    def test_create_with_data_map(self):
        edge = EdgeDef(
            id="e1",
            source="n1",
            target="n2",
            data_map={"prompt": "n1.output.text"},
        )
        assert edge.data_map["prompt"] == "n1.output.text"

    def test_unconditional_self_loop_rejected(self):
        with pytest.raises(ValidationError, match="self-loop"):
            EdgeDef(id="e1", source="n1", target="n1")

    def test_conditional_self_loop_allowed(self):
        edge = EdgeDef(
            id="e1",
            source="n1",
            target="n1",
            condition={"field": "done", "op": "eq", "value": False},
        )
        assert edge.source == edge.target


class TestGraphDef:
    @pytest.fixture
    def simple_graph(self):
        return GraphDef(
            id="g1",
            name="Test Graph",
            nodes=[
                NodeDef(id="n1", tool_type="trigger/webhook"),
                NodeDef(id="n2", tool_type="ai/llm_call"),
                NodeDef(id="n3", tool_type="data/db_write"),
            ],
            edges=[
                EdgeDef(id="e1", source="n1", target="n2"),
                EdgeDef(id="e2", source="n2", target="n3"),
            ],
        )

    def test_create_simple_graph(self, simple_graph):
        assert simple_graph.id == "g1"
        assert len(simple_graph.nodes) == 3
        assert len(simple_graph.edges) == 2
        assert simple_graph.version == "0.0.1"

    def test_empty_graph_valid(self):
        g = GraphDef(id="g1", name="Empty")
        assert g.nodes == []
        assert g.edges == []

    def test_duplicate_node_ids_rejected(self):
        with pytest.raises(ValidationError, match="duplicate node"):
            GraphDef(
                id="g1",
                name="Bad",
                nodes=[
                    NodeDef(id="n1", tool_type="logic/condition"),
                    NodeDef(id="n1", tool_type="ai/llm_call"),
                ],
            )

    def test_duplicate_edge_ids_rejected(self):
        with pytest.raises(ValidationError, match="duplicate edge"):
            GraphDef(
                id="g1",
                name="Bad",
                nodes=[
                    NodeDef(id="n1", tool_type="a"),
                    NodeDef(id="n2", tool_type="b"),
                ],
                edges=[
                    EdgeDef(id="e1", source="n1", target="n2"),
                    EdgeDef(id="e1", source="n1", target="n2"),
                ],
            )

    def test_edge_references_invalid_source(self):
        with pytest.raises(ValidationError, match="references non-existent node"):
            GraphDef(
                id="g1",
                name="Bad",
                nodes=[NodeDef(id="n1", tool_type="a")],
                edges=[EdgeDef(id="e1", source="n999", target="n1")],
            )

    def test_edge_references_invalid_target(self):
        with pytest.raises(ValidationError, match="references non-existent node"):
            GraphDef(
                id="g1",
                name="Bad",
                nodes=[NodeDef(id="n1", tool_type="a")],
                edges=[EdgeDef(id="e1", source="n1", target="n999")],
            )

    def test_metadata_defaults_empty(self, simple_graph):
        assert simple_graph.metadata == {}

    def test_metadata_custom(self):
        g = GraphDef(
            id="g1",
            name="With Meta",
            metadata={"author": "test", "tags": ["demo"]},
        )
        assert g.metadata["author"] == "test"

    def test_serialization_roundtrip(self, simple_graph):
        data = simple_graph.model_dump()
        restored = GraphDef.model_validate(data)
        assert restored == simple_graph

    def test_json_roundtrip(self, simple_graph):
        json_str = simple_graph.model_dump_json()
        restored = GraphDef.model_validate_json(json_str)
        assert restored == simple_graph

    def test_get_node_by_id(self, simple_graph):
        node = simple_graph.get_node("n2")
        assert node is not None
        assert node.tool_type == "ai/llm_call"

    def test_get_node_not_found(self, simple_graph):
        assert simple_graph.get_node("n999") is None

    def test_get_outgoing_edges(self, simple_graph):
        edges = simple_graph.get_outgoing_edges("n1")
        assert len(edges) == 1
        assert edges[0].target == "n2"

    def test_get_outgoing_edges_terminal_node(self, simple_graph):
        edges = simple_graph.get_outgoing_edges("n3")
        assert edges == []
