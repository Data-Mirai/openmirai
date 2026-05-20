"""FastAPI application — API for graphs, agents, execution, sessions.

Integrates AgentRuntime for real agent lifecycle management:
- Schedule triggers fire automatically
- Webhook triggers route HTTP to correct agent
- Enable/disable registers/unregisters triggers
- Postgres persistence when DATABASE_URL is set
"""

from __future__ import annotations

import contextlib
import logging
import uuid
from contextlib import asynccontextmanager
from typing import Any

from fastapi import FastAPI, HTTPException, Request
from pydantic import BaseModel, Field

from datamirai_engine import __version__
from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.agent_spec import AgentSpec
from datamirai_engine.core.graph import EdgeDef, GraphDef, NodeDef
from datamirai_engine.runtime.agent_runtime import AgentRuntime

logger = logging.getLogger(__name__)

# --- Request/Response models ---


class GraphCreateRequest(BaseModel):
    name: str
    nodes: list[dict[str, Any]] = Field(default_factory=list)
    edges: list[dict[str, Any]] = Field(default_factory=list)
    metadata: dict[str, Any] = Field(default_factory=dict)


class GraphUpdateRequest(BaseModel):
    name: str | None = None
    nodes: list[dict[str, Any]] | None = None
    edges: list[dict[str, Any]] | None = None
    metadata: dict[str, Any] | None = None


class AgentCreateRequest(BaseModel):
    graph_id: str
    name: str
    triggers: list[dict[str, Any]] = Field(default_factory=list)


class AgentPatchRequest(BaseModel):
    status: str | None = None


class ExecuteRequest(BaseModel):
    entry_node_id: str | None = None
    trigger_data: dict[str, Any] = Field(default_factory=dict)


# --- Registry setup ---


def _create_registry() -> ToolRegistry:
    registry = ToolRegistry()
    modules = [
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
        "datamirai_engine.tools.builtin.trigger.triggers",
        # filesystem
        "datamirai_engine.tools.builtin.filesystem.read_file",
        "datamirai_engine.tools.builtin.filesystem.write_file",
        "datamirai_engine.tools.builtin.filesystem.edit_file",
        "datamirai_engine.tools.builtin.filesystem.glob_files",
        "datamirai_engine.tools.builtin.filesystem.grep_files",
        "datamirai_engine.tools.builtin.filesystem.list_dir",
        "datamirai_engine.tools.builtin.filesystem.tree",
        "datamirai_engine.tools.builtin.filesystem.file_info",
        "datamirai_engine.tools.builtin.filesystem.move",
        "datamirai_engine.tools.builtin.filesystem.copy",
        "datamirai_engine.tools.builtin.filesystem.delete",
        "datamirai_engine.tools.builtin.filesystem.mkdir",
        # system
        "datamirai_engine.tools.builtin.system.bash",
        "datamirai_engine.tools.builtin.system.process_list",
        # git
        "datamirai_engine.tools.builtin.git.status",
        "datamirai_engine.tools.builtin.git.diff",
        "datamirai_engine.tools.builtin.git.log",
        "datamirai_engine.tools.builtin.git.commit",
    ]
    for module in modules:
        with contextlib.suppress(Exception):
            registry.discover(module)
    return registry


# --- App factory ---


def create_app() -> FastAPI:
    registry = _create_registry()
    runtime = AgentRuntime(registry=registry)
    _pool_holder: dict[str, Any] = {}  # mutable container for lifespan closure

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        # Try to connect to Postgres if DATABASE_URL is set
        pool = None
        try:
            from datamirai_engine.db.connection import close_pool, create_pool, get_database_url
            from datamirai_engine.db.repositories import AgentRepo, GraphRepo, SessionRepo

            dsn = get_database_url()
            if dsn:
                pool = await create_pool(dsn)
                runtime._graph_repo = GraphRepo(pool)
                runtime._agent_repo = AgentRepo(pool)
                runtime._session_repo = SessionRepo(pool)
                _pool_holder["pool"] = pool
                await runtime.load_from_db()
                logger.info("Postgres persistence enabled")
        except Exception as e:
            logger.warning("Postgres not available, running in-memory: %s", e)
            pool = None

        await runtime.start()
        yield
        await runtime.stop()

        if pool:
            await close_pool(pool)

    app = FastAPI(title="Data Mirai Engine", version=__version__, lifespan=lifespan)

    # --- Health ---

    @app.get("/health")
    async def health():
        return {
            "status": "ok",
            "version": __version__,
            "runtime": runtime.status(),
        }

    # --- Graphs CRUD ---

    @app.post("/api/graphs", status_code=201)
    async def create_graph(req: GraphCreateRequest):
        graph_id = str(uuid.uuid4())[:8]
        nodes = [NodeDef(**n) for n in req.nodes] if req.nodes else []
        edges = [EdgeDef(**e) for e in req.edges] if req.edges else []
        graph = GraphDef(
            id=graph_id, name=req.name,
            nodes=nodes, edges=edges, metadata=req.metadata,
        )
        runtime.register_graph(graph)
        await runtime.persist_graph(graph)
        return graph.model_dump()

    @app.get("/api/graphs")
    async def list_graphs():
        return [
            g.model_dump() for g in
            [runtime.get_graph(gid) for gid in runtime._graphs]
            if g is not None
        ]

    @app.get("/api/graphs/{graph_id}")
    async def get_graph(graph_id: str):
        graph = runtime.get_graph(graph_id)
        if not graph:
            raise HTTPException(status_code=404, detail="Graph not found")
        return graph.model_dump()

    @app.put("/api/graphs/{graph_id}")
    async def update_graph(graph_id: str, req: GraphUpdateRequest):
        old = runtime.get_graph(graph_id)
        if not old:
            raise HTTPException(status_code=404, detail="Graph not found")
        nodes = [NodeDef(**n) for n in req.nodes] if req.nodes else list(old.nodes)
        edges = [EdgeDef(**e) for e in req.edges] if req.edges else list(old.edges)
        graph = GraphDef(
            id=graph_id,
            name=req.name or old.name,
            nodes=nodes, edges=edges,
            metadata=req.metadata if req.metadata is not None else dict(old.metadata),
        )
        runtime.register_graph(graph)
        await runtime.persist_graph(graph)
        return graph.model_dump()

    @app.delete("/api/graphs/{graph_id}", status_code=204)
    async def delete_graph(graph_id: str):
        if not runtime.get_graph(graph_id):
            raise HTTPException(status_code=404, detail="Graph not found")
        runtime._graphs.pop(graph_id, None)
        if runtime._graph_repo:
            await runtime._graph_repo.delete(graph_id)

    # --- Agents ---

    @app.post("/api/agents", status_code=201)
    async def create_agent(req: AgentCreateRequest):
        try:
            return await runtime.deploy_agent(
                name=req.name,
                graph_id=req.graph_id,
                triggers=req.triggers,
            )
        except ValueError as e:
            raise HTTPException(status_code=404, detail=str(e)) from None

    @app.get("/api/agents")
    async def list_agents():
        return runtime.list_agents()

    @app.get("/api/agents/{agent_id}")
    async def get_agent(agent_id: str):
        agent = runtime.get_agent(agent_id)
        if not agent:
            raise HTTPException(status_code=404, detail="Agent not found")
        return agent

    @app.patch("/api/agents/{agent_id}")
    async def patch_agent(agent_id: str, req: AgentPatchRequest):
        if req.status not in ("enabled", "disabled"):
            raise HTTPException(status_code=400, detail="Invalid status")
        try:
            if req.status == "enabled":
                return await runtime.enable_agent(agent_id)
            else:
                return await runtime.disable_agent(agent_id)
        except ValueError as e:
            raise HTTPException(status_code=404, detail=str(e)) from None

    @app.delete("/api/agents/{agent_id}", status_code=204)
    async def delete_agent(agent_id: str):
        await runtime.destroy_agent(agent_id)

    # --- Agent Spec (YAML/JSON definition) ---

    @app.get("/api/agents/{agent_id}/spec")
    async def get_agent_spec(agent_id: str, request: Request):
        agent = runtime.get_agent(agent_id)
        if not agent:
            raise HTTPException(status_code=404, detail="Agent not found")

        graph = runtime.get_graph(agent["graph_id"])
        if not graph:
            raise HTTPException(status_code=404, detail="Graph not found for agent")

        # Build AgentSpec from runtime state
        spec = AgentSpec(
            name=agent["name"],
            description=agent.get("description", ""),
            version=agent.get("version", "v1"),
            graph={
                "nodes": [n.model_dump() for n in graph.nodes],
                "edges": [e.model_dump() for e in graph.edges],
            },
            triggers=[t for t in agent.get("triggers", [])],
            metadata=agent.get("metadata", {}),
        )

        accept = request.headers.get("accept", "application/json")
        if "yaml" in accept or "x-yaml" in accept:
            from fastapi.responses import Response
            return Response(content=spec.to_yaml(), media_type="application/x-yaml")
        return spec.to_dict()

    @app.put("/api/agents/{agent_id}/spec")
    async def update_agent_spec(agent_id: str, request: Request):
        agent = runtime.get_agent(agent_id)
        if not agent:
            raise HTTPException(status_code=404, detail="Agent not found")

        content_type = request.headers.get("content-type", "application/json")
        body = await request.body()
        body_str = body.decode("utf-8")

        try:
            if "yaml" in content_type or "x-yaml" in content_type:
                spec = AgentSpec.from_yaml(body_str)
            else:
                import json
                spec = AgentSpec.from_dict(json.loads(body_str))
        except Exception as e:
            raise HTTPException(status_code=400, detail=f"Invalid spec: {e}") from None

        # Update graph
        graph = spec.to_graph(graph_id=agent["graph_id"])
        runtime.register_graph(graph)
        await runtime.persist_graph(graph)

        # Update agent fields
        agent["name"] = spec.name
        agent["description"] = spec.description
        agent["version"] = spec.version
        agent["triggers"] = spec.to_triggers_list()
        agent["metadata"] = dict(spec.metadata)

        return agent

    @app.post("/api/agents/import", status_code=201)
    async def import_agent(request: Request):
        content_type = request.headers.get("content-type", "application/json")
        body = await request.body()
        body_str = body.decode("utf-8")

        try:
            if "yaml" in content_type or "x-yaml" in content_type:
                spec = AgentSpec.from_yaml(body_str)
            else:
                import json
                spec = AgentSpec.from_dict(json.loads(body_str))
        except Exception as e:
            raise HTTPException(status_code=400, detail=f"Invalid spec: {e}") from None

        # Create graph from spec
        graph = spec.to_graph()
        runtime.register_graph(graph)
        await runtime.persist_graph(graph)

        # Deploy agent
        agent = await runtime.deploy_agent(
            name=spec.name,
            graph_id=graph.id,
            triggers=spec.to_triggers_list(),
        )
        agent["description"] = spec.description
        agent["version"] = spec.version
        agent["metadata"] = dict(spec.metadata)

        return agent

    @app.post("/api/agents/{agent_id}/version", status_code=201)
    async def create_agent_version(agent_id: str):
        agent = runtime.get_agent(agent_id)
        if not agent:
            raise HTTPException(status_code=404, detail="Agent not found")

        graph = runtime.get_graph(agent["graph_id"])
        if not graph:
            raise HTTPException(status_code=404, detail="Graph not found for agent")

        # Snapshot current state as a version record
        spec = AgentSpec(
            name=agent["name"],
            description=agent.get("description", ""),
            version=agent.get("version", "v1"),
            graph={
                "nodes": [n.model_dump() for n in graph.nodes],
                "edges": [e.model_dump() for e in graph.edges],
            },
            triggers=[t for t in agent.get("triggers", [])],
            metadata=agent.get("metadata", {}),
        )

        # Store version
        versions = agent.setdefault("versions", [])
        version_num = len(versions) + 1
        version_label = f"v{version_num}"
        version_record = {
            "version": version_label,
            "spec": spec.to_dict(),
            "created_at": __import__("time").time(),
        }
        versions.append(version_record)
        agent["version"] = version_label

        return version_record

    # --- Execution ---

    @app.post("/api/agents/{agent_id}/execute")
    async def execute_agent(agent_id: str, req: ExecuteRequest):
        try:
            return await runtime.execute_agent(agent_id, trigger_data=req.trigger_data)
        except ValueError as e:
            raise HTTPException(status_code=404, detail=str(e)) from None

    @app.post("/api/graphs/{graph_id}/execute")
    async def execute_graph(graph_id: str, req: ExecuteRequest):
        """Execute a graph directly (without agent). For testing."""
        from datamirai_engine.core.runner import (  # noqa: I001
            GraphExecutionError, GraphRunner, MaxIterationsError, RegistryExecutor,
        )

        graph = runtime.get_graph(graph_id)
        if not graph:
            raise HTTPException(status_code=404, detail="Graph not found")

        entry = req.entry_node_id or (graph.nodes[0].id if graph.nodes else None)
        if not entry:
            raise HTTPException(status_code=400, detail="No entry node")

        runner = GraphRunner(executor=RegistryExecutor(registry))
        try:
            result = await runner.run(graph, context=None, entry_node_id=entry)
        except (GraphExecutionError, MaxIterationsError) as e:
            return {"id": str(uuid.uuid4())[:8], "status": "failed", "error": str(e)}

        return {
            "id": str(uuid.uuid4())[:8],
            "graph_id": graph_id,
            "status": result.status,
            "trace": result.trace,
            "state": result.state.snapshot(),
        }

    # --- Webhooks ---

    @app.post("/webhooks/{path:path}")
    async def webhook_handler(path: str, request: Request):
        try:
            body = await request.json()
        except Exception:
            body = {}
        headers = dict(request.headers)
        result = await runtime.handle_webhook(
            f"/webhooks/{path}", body=body, headers=headers,
        )
        if result is None:
            raise HTTPException(status_code=404, detail=f"No agent registered for /webhooks/{path}")
        return result

    # --- Sessions ---

    @app.get("/api/sessions")
    async def list_sessions(agent_id: str | None = None, limit: int = 50):
        return runtime.list_sessions(agent_id=agent_id, limit=limit)

    @app.get("/api/sessions/{session_id}")
    async def get_session(session_id: str):
        session = await runtime.get_session_from_db(session_id)
        if not session:
            raise HTTPException(status_code=404, detail="Session not found")
        return session

    # --- Runtime status ---

    @app.get("/api/runtime/status")
    async def runtime_status():
        return runtime.status()

    @app.get("/api/runtime/schedules")
    async def runtime_schedules():
        return runtime._scheduler.list_jobs()

    return app
