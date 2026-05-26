"""GraphDef — Graph structure definition with nodes and edges."""

from __future__ import annotations

from pydantic import BaseModel, Field, model_validator


class NodeDef(BaseModel):
    """Node in a graph. Instance of a block with specific config."""

    id: str
    tool_type: str
    version: str = "1.0.0"
    config: dict = Field(default_factory=dict)
    position: dict[str, float] = Field(default_factory=lambda: {"x": 0.0, "y": 0.0})

    model_config = {"frozen": True}


class EdgeDef(BaseModel):
    """Connection between two nodes. Optional condition and data mapping.

    FEAT-034:
    - id is optional — auto-generated as '{source}__{target}' if omitted
    - data_map is optional — default passthrough when edge is the only
      unconditional outgoing edge (handled in GraphRunner)
    """

    id: str = ""
    source: str
    target: str
    condition: dict | None = None
    data_map: dict[str, str] | None = None

    model_config = {"frozen": True}

    @model_validator(mode="after")
    def _no_self_loop(self) -> EdgeDef:
        if self.source == self.target and self.condition is None:
            raise ValueError(
                f"Edge '{self.id or '(auto)'}' is an unconditional self-loop "
                f"(source == target == '{self.source}'). "
                f"Self-loops require a condition to avoid infinite execution."
            )
        return self


class GraphDef(BaseModel):
    """Complete graph definition: nodes + edges + metadata."""

    id: str
    name: str
    version: str = "0.0.1"
    nodes: list[NodeDef] = Field(default_factory=list)
    edges: list[EdgeDef] = Field(default_factory=list)
    metadata: dict = Field(default_factory=dict)

    @model_validator(mode="after")
    def _validate_graph(self) -> GraphDef:
        # Unique node IDs
        node_ids = [n.id for n in self.nodes]
        dupes = {nid for nid in node_ids if node_ids.count(nid) > 1}
        if dupes:
            raise ValueError(f"Graph has duplicate node IDs: {dupes}")

        # Auto-generate missing edge IDs (FEAT-034 / API-02)
        seen_pairs: dict[str, int] = {}
        new_edges: list[EdgeDef] = []
        for edge in self.edges:
            if not edge.id:
                pair_key = f"{edge.source}__{edge.target}"
                count = seen_pairs.get(pair_key, 0) + 1
                seen_pairs[pair_key] = count
                auto_id = pair_key if count == 1 else f"{pair_key}__{count}"
                edge = edge.model_copy(update={"id": auto_id})
            new_edges.append(edge)

        # Replace edges list with auto-ID'd version (Pydantic frozen workaround)
        object.__setattr__(self, "edges", new_edges)

        # Unique edge IDs (after auto-gen)
        edge_ids = [e.id for e in self.edges]
        edge_dupes = {eid for eid in edge_ids if edge_ids.count(eid) > 1}
        if edge_dupes:
            raise ValueError(f"Graph has duplicate edge IDs: {edge_dupes}")

        # Edges reference existing nodes
        node_id_set = set(node_ids)
        for edge in self.edges:
            if edge.source not in node_id_set:
                raise ValueError(
                    f"Edge '{edge.id}' references non-existent node '{edge.source}' as source"
                )
            if edge.target not in node_id_set:
                raise ValueError(
                    f"Edge '{edge.id}' references non-existent node '{edge.target}' as target"
                )

        return self

    def get_node(self, node_id: str) -> NodeDef | None:
        """Find node by ID. Returns None if not found."""
        for node in self.nodes:
            if node.id == node_id:
                return node
        return None

    def get_outgoing_edges(self, node_id: str) -> list[EdgeDef]:
        """Get all edges originating from a node."""
        return [e for e in self.edges if e.source == node_id]
