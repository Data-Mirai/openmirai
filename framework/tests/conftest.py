"""Shared pytest fixtures for Data Mirai Engine tests."""

from __future__ import annotations

import pytest
from httpx import ASGITransport, AsyncClient

from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.context import AuthContext
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.core.runner import GraphRunner, RegistryExecutor
from datamirai_engine.resources.context import SimpleExecutionContext
from datamirai_engine.resources.db import InMemoryDBResource
from datamirai_engine.resources.llm import MockLLMResource
from datamirai_engine.resources.storage import InMemoryStorageResource
from datamirai_engine.resources.vector import InMemoryVectorResource
from datamirai_engine.server.app import create_app


@pytest.fixture
def context():
    """Fresh SimpleExecutionContext with in-memory resources."""
    return SimpleExecutionContext(
        db=InMemoryDBResource(),
        vector=InMemoryVectorResource(),
        storage=InMemoryStorageResource(),
        llm=MockLLMResource(),
        auth=AuthContext(user_id="test-user", role="OWNER"),
    )


@pytest.fixture
def registry():
    """ToolRegistry with all builtin tools discovered."""
    reg = ToolRegistry()
    for module in [
        "datamirai_engine.tools.builtin.logic.condition",
        "datamirai_engine.tools.builtin.logic.switch",
        "datamirai_engine.tools.builtin.logic.loop",
        "datamirai_engine.tools.builtin.logic.merge",
        "datamirai_engine.tools.builtin.logic.wait",
        "datamirai_engine.tools.builtin.ai.llm_call",
        "datamirai_engine.tools.builtin.ai.transcribe",
        "datamirai_engine.tools.builtin.ai.embeddings",
        "datamirai_engine.tools.builtin.data.db_read",
        "datamirai_engine.tools.builtin.data.db_write",
        "datamirai_engine.tools.builtin.data.storage_read",
        "datamirai_engine.tools.builtin.data.storage_write",
    ]:
        reg.discover(module)
    return reg


@pytest.fixture
def runner(registry):
    """GraphRunner with RegistryExecutor wired to all builtin tools."""
    return GraphRunner(executor=RegistryExecutor(registry))


@pytest.fixture
def app():
    """Fresh FastAPI app instance."""
    return create_app()


@pytest.fixture
async def api_client(app):
    """Async HTTP client for API testing."""
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield client


def make_linear_graph(
    *tool_types: str, graph_id: str = "test-graph", name: str = "Test"
) -> GraphDef:
    """Helper: create a linear graph from a list of block types."""
    nodes = [
        NodeDef(id=f"n{i}", tool_type=bt)
        for i, bt in enumerate(tool_types, 1)
    ]
    edges = [
        EdgeDef(id=f"e{i}", source=f"n{i}", target=f"n{i+1}")
        for i in range(1, len(nodes))
    ]
    return GraphDef(id=graph_id, name=name, nodes=nodes, edges=edges)
