"""AgentSpec — declarative agent definition with YAML/JSON serialization.

An AgentSpec is the complete, self-contained definition of an agent:
graph (nodes + edges), triggers, config, resources, and metadata.

One YAML file = one agent. Export it, send it to another server, import it.
"""

from __future__ import annotations

import uuid
from typing import Any

import yaml
from pydantic import BaseModel, Field, model_validator

from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef


# --- Sub-models ---


class AgentNodeSpec(BaseModel):
    """Node within an agent spec. Maps to NodeDef."""

    id: str
    tool_type: str
    version: str = "1.0.0"
    config: dict[str, Any] = Field(default_factory=dict)
    position: dict[str, float] = Field(default_factory=lambda: {"x": 0.0, "y": 0.0})


class AgentEdgeSpec(BaseModel):
    """Edge within an agent spec. Maps to EdgeDef."""

    id: str
    source: str
    target: str
    condition: dict[str, Any] | None = None
    data_map: dict[str, str] | None = None


class AgentGraphSpec(BaseModel):
    """Inline graph definition inside an agent spec."""

    nodes: list[AgentNodeSpec] = Field(default_factory=list)
    edges: list[AgentEdgeSpec] = Field(default_factory=list)


class AgentRetryConfig(BaseModel):
    """Retry policy at agent level."""

    max_retries: int = 3
    backoff: str = "exponential"
    on_failure: str = "stop"


class AgentHookSpec(BaseModel):
    """Hook definition within agent config."""

    handler: str  # function name or module.path
    filter: str | None = None  # glob pattern for tool_type (e.g., "ai/*")


class AgentMcpServerSpec(BaseModel):
    """MCP server reference within agent config."""

    name: str
    transport: str = "stdio"  # stdio | http
    command: str | None = None  # for stdio
    args: list[str] = Field(default_factory=list)  # for stdio
    url: str | None = None  # for http
    credential_ref: str | None = None  # vault credential reference


class AgentConfig(BaseModel):
    """Top-level agent configuration."""

    max_iterations: int = 50
    retry: AgentRetryConfig = Field(default_factory=AgentRetryConfig)
    timeout_ms: int = 60000
    hooks: dict[str, list[AgentHookSpec]] = Field(default_factory=dict)
    mcp_servers: list[AgentMcpServerSpec] = Field(default_factory=list)
    vault_refs: list[dict[str, str]] = Field(default_factory=list)


class AgentTriggerSpec(BaseModel):
    """Trigger definition within an agent spec."""

    type: str
    path: str | None = None
    method: str | None = None
    auth: str | None = None
    interval_seconds: int | None = None
    cron: str | None = None
    event_type: str | None = None
    source: str | None = None
    input_form: list[dict[str, Any]] | None = None

    def to_runtime_dict(self) -> dict[str, Any]:
        """Convert to the dict format AgentRuntime expects."""
        d: dict[str, Any] = {"type": self.type}
        for field_name in (
            "path", "method", "auth", "interval_seconds",
            "cron", "event_type", "source", "input_form",
        ):
            val = getattr(self, field_name)
            if val is not None:
                d[field_name] = val
        return d


# --- Main AgentSpec ---


class AgentSpec(BaseModel):
    """Complete, self-contained agent definition.

    Includes the graph inline (nodes + edges), triggers, config,
    resource references, and metadata. Serializes to/from YAML and JSON.
    """

    name: str
    description: str = ""
    version: str = "v1"
    agent_type: str = "managed"  # managed | live
    system_prompt: str = ""
    graph: AgentGraphSpec = Field(default_factory=AgentGraphSpec)
    triggers: list[AgentTriggerSpec] = Field(default_factory=list)
    config: AgentConfig = Field(default_factory=AgentConfig)
    resources: list[str] = Field(default_factory=list)
    metadata: dict[str, Any] = Field(default_factory=dict)

    @model_validator(mode="after")
    def _validate_graph_refs(self) -> AgentSpec:
        """Validate that edges reference existing nodes."""
        node_ids = {n.id for n in self.graph.nodes}

        # Duplicate node IDs
        all_ids = [n.id for n in self.graph.nodes]
        dupes = {nid for nid in all_ids if all_ids.count(nid) > 1}
        if dupes:
            raise ValueError(f"Duplicate node IDs: {dupes}")

        # Duplicate edge IDs
        edge_ids = [e.id for e in self.graph.edges]
        edge_dupes = {eid for eid in edge_ids if edge_ids.count(eid) > 1}
        if edge_dupes:
            raise ValueError(f"Duplicate edge IDs: {edge_dupes}")

        # Edge references
        for edge in self.graph.edges:
            if edge.source not in node_ids:
                raise ValueError(
                    f"Edge '{edge.id}' references non-existent source node '{edge.source}'"
                )
            if edge.target not in node_ids:
                raise ValueError(
                    f"Edge '{edge.id}' references non-existent target node '{edge.target}'"
                )

        return self

    # --- Serialization ---

    def to_dict(self) -> dict[str, Any]:
        """Serialize to a plain dict (JSON-compatible)."""
        return self.model_dump(exclude_none=True)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> AgentSpec:
        """Create from a plain dict."""
        return cls.model_validate(data)

    def to_yaml(self) -> str:
        """Serialize to YAML string."""
        return yaml.dump(
            self.to_dict(),
            default_flow_style=False,
            sort_keys=False,
            allow_unicode=True,
        )

    @classmethod
    def from_yaml(cls, yaml_string: str) -> AgentSpec:
        """Parse YAML string into AgentSpec."""
        data = yaml.safe_load(yaml_string)
        if not isinstance(data, dict):
            raise ValueError("YAML must be a mapping at the top level")
        return cls.from_dict(data)

    # --- Conversion to runtime types ---

    def to_graph(self, graph_id: str | None = None) -> GraphDef:
        """Convert the inline graph to a GraphDef ready for execution."""
        gid = graph_id or str(uuid.uuid4())[:8]
        nodes = [
            NodeDef(
                id=n.id,
                tool_type=n.tool_type,
                version=n.version,
                config=n.config,
                position=n.position,
            )
            for n in self.graph.nodes
        ]
        edges = [
            EdgeDef(
                id=e.id,
                source=e.source,
                target=e.target,
                condition=e.condition,
                data_map=e.data_map,
            )
            for e in self.graph.edges
        ]
        return GraphDef(
            id=gid,
            name=self.name,
            version=self.version,
            nodes=nodes,
            edges=edges,
            metadata=dict(self.metadata),
        )

    def to_triggers_list(self) -> list[dict[str, Any]]:
        """Convert triggers to the list[dict] format AgentRuntime expects."""
        return [t.to_runtime_dict() for t in self.triggers]
