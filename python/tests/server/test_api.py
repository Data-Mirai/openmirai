"""Tests for FastAPI server — all BLOCK-013 tickets."""

from __future__ import annotations

import pytest
from httpx import ASGITransport, AsyncClient

from datamirai_engine.server.app import create_app


@pytest.fixture
def app():
    return create_app()


@pytest.fixture
async def client(app):
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


# --- TICK-043: Health endpoint ---


class TestHealth:
    @pytest.mark.asyncio
    async def test_health(self, client):
        resp = await client.get("/health")
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "ok"
        assert "version" in data


# --- TICK-044: CRUD de grafos ---


class TestGraphCRUD:
    @pytest.mark.asyncio
    async def test_create_graph(self, client):
        resp = await client.post("/api/graphs", json={
            "name": "My Graph",
            "nodes": [
                {"id": "n1", "tool_type": "trigger/webhook"},
                {"id": "n2", "tool_type": "ai/llm_call"},
            ],
            "edges": [
                {"id": "e1", "source": "n1", "target": "n2"},
            ],
        })
        assert resp.status_code == 201
        data = resp.json()
        assert "id" in data
        assert data["name"] == "My Graph"

    @pytest.mark.asyncio
    async def test_list_graphs(self, client):
        await client.post("/api/graphs", json={"name": "G1"})
        await client.post("/api/graphs", json={"name": "G2"})
        resp = await client.get("/api/graphs")
        assert resp.status_code == 200
        assert len(resp.json()) >= 2

    @pytest.mark.asyncio
    async def test_get_graph(self, client):
        create = await client.post("/api/graphs", json={"name": "G1"})
        graph_id = create.json()["id"]
        resp = await client.get(f"/api/graphs/{graph_id}")
        assert resp.status_code == 200
        assert resp.json()["id"] == graph_id

    @pytest.mark.asyncio
    async def test_get_graph_not_found(self, client):
        resp = await client.get("/api/graphs/nonexistent")
        assert resp.status_code == 404

    @pytest.mark.asyncio
    async def test_update_graph(self, client):
        create = await client.post("/api/graphs", json={"name": "Old"})
        graph_id = create.json()["id"]
        resp = await client.put(f"/api/graphs/{graph_id}", json={"name": "New"})
        assert resp.status_code == 200
        assert resp.json()["name"] == "New"

    @pytest.mark.asyncio
    async def test_delete_graph(self, client):
        create = await client.post("/api/graphs", json={"name": "ToDelete"})
        graph_id = create.json()["id"]
        resp = await client.delete(f"/api/graphs/{graph_id}")
        assert resp.status_code == 204
        get_resp = await client.get(f"/api/graphs/{graph_id}")
        assert get_resp.status_code == 404


# --- TICK-045: Gestión de agentes ---


class TestAgentManagement:
    @pytest.mark.asyncio
    async def test_deploy_agent(self, client):
        create = await client.post("/api/graphs", json={"name": "G1"})
        graph_id = create.json()["id"]
        resp = await client.post("/api/agents", json={
            "graph_id": graph_id,
            "name": "Agent 1",
        })
        assert resp.status_code == 201
        data = resp.json()
        assert data["graph_id"] == graph_id
        assert data["status"] == "disabled"

    @pytest.mark.asyncio
    async def test_list_agents(self, client):
        create = await client.post("/api/graphs", json={"name": "G1"})
        graph_id = create.json()["id"]
        await client.post("/api/agents", json={"graph_id": graph_id, "name": "A1"})
        resp = await client.get("/api/agents")
        assert resp.status_code == 200
        assert len(resp.json()) >= 1

    @pytest.mark.asyncio
    async def test_enable_disable_agent(self, client):
        create = await client.post("/api/graphs", json={"name": "G1"})
        graph_id = create.json()["id"]
        agent = await client.post("/api/agents", json={
            "graph_id": graph_id, "name": "A1"
        })
        agent_id = agent.json()["id"]

        resp = await client.patch(f"/api/agents/{agent_id}", json={"status": "enabled"})
        assert resp.status_code == 200
        assert resp.json()["status"] == "enabled"

        resp = await client.patch(f"/api/agents/{agent_id}", json={"status": "disabled"})
        assert resp.json()["status"] == "disabled"


# --- TICK-046: Ejecución y sesiones ---


class TestExecution:
    @pytest.mark.asyncio
    async def test_execute_graph(self, client):
        create = await client.post("/api/graphs", json={
            "name": "Exec Test",
            "nodes": [{"id": "n1", "tool_type": "trigger/manual"}],
        })
        graph_id = create.json()["id"]
        resp = await client.post(f"/api/graphs/{graph_id}/execute", json={
            "entry_node_id": "n1",
        })
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] in ("completed", "failed")

    @pytest.mark.asyncio
    async def test_list_sessions(self, client):
        create = await client.post("/api/graphs", json={
            "name": "G1",
            "nodes": [{"id": "n1", "tool_type": "trigger/manual"}],
        })
        graph_id = create.json()["id"]
        # Deploy agent and execute via runtime (sessions tracked here)
        agent = await client.post("/api/agents", json={
            "graph_id": graph_id, "name": "test-agent",
        })
        agent_id = agent.json()["id"]
        await client.post(f"/api/agents/{agent_id}/execute", json={})
        resp = await client.get("/api/sessions")
        assert resp.status_code == 200
        assert len(resp.json()) >= 1
