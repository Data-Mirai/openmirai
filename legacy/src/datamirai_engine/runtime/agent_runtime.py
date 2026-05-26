"""AgentRuntime — manages the lifecycle of agents within an environment.

Responsibilities:
- Register/unregister agents
- Enable/disable (registers/unregisters triggers)
- Execute agents (manual or triggered)
- Manage scheduler for scheduled agents
- Webhook routing for webhook-triggered agents
- Track sessions
- Persist to Postgres when repos are provided (optional)
"""

from __future__ import annotations

import logging
import time
import uuid
from typing import Any

from datamirai_engine.tools.registry import ToolRegistry
from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.core.graph import GraphDef
from datamirai_engine.core.runner import (
    GraphExecutionError,
    GraphRunner,
    MaxIterationsError,
    RegistryExecutor,
)
from datamirai_engine.runtime.scheduler import Scheduler

logger = logging.getLogger(__name__)


class AgentRuntime:
    """Manages running agents in an environment.

    This is the "process" described in FLUJOS.md:
    - ONE runtime manages ALL agents in an environment
    - Webhook listener routes HTTP to correct agent
    - Scheduler fires scheduled agents
    - Registry maps trigger → agent

    If repos (GraphRepo, AgentRepo, SessionRepo) are provided,
    state is persisted to Postgres. Otherwise, everything stays in memory.
    """

    def __init__(
        self,
        registry: ToolRegistry,
        context: ExecutionContext | None = None,
        *,
        graph_repo: Any | None = None,
        agent_repo: Any | None = None,
        session_repo: Any | None = None,
    ) -> None:
        self._registry = registry
        self._context = context
        self._runner = GraphRunner(executor=RegistryExecutor(registry))
        self._scheduler = Scheduler(on_fire=self._on_schedule_fire)

        # Persistence repos (optional — None = in-memory only)
        self._graph_repo = graph_repo
        self._agent_repo = agent_repo
        self._session_repo = session_repo

        # In-memory storage (always used as cache, persisted if repos present)
        self._agents: dict[str, dict[str, Any]] = {}
        self._graphs: dict[str, GraphDef] = {}
        self._sessions: list[dict[str, Any]] = []

        # Webhook routing: path → agent_id
        self._webhook_routes: dict[str, str] = {}

        self._running = False

    # --- Lifecycle ---

    async def start(self) -> None:
        self._running = True
        await self._scheduler.start()
        logger.info("AgentRuntime started")

    async def stop(self) -> None:
        self._running = False
        await self._scheduler.stop()
        logger.info("AgentRuntime stopped")

    # --- Graph management ---

    def register_graph(self, graph: GraphDef) -> None:
        self._graphs[graph.id] = graph

    async def persist_graph(self, graph: GraphDef) -> None:
        """Persist graph to DB if repo available. Call after register_graph."""
        if self._graph_repo:
            await self._graph_repo.save({
                "id": graph.id,
                "name": graph.name,
                "version": graph.version,
                "nodes": [n.model_dump() for n in graph.nodes],
                "edges": [e.model_dump() for e in graph.edges],
                "metadata": dict(graph.metadata),
            })

    def get_graph(self, graph_id: str) -> GraphDef | None:
        return self._graphs.get(graph_id)

    # --- Agent management ---

    async def deploy_agent(
        self,
        name: str,
        graph_id: str,
        triggers: list[dict[str, Any]] | None = None,
    ) -> dict[str, Any]:
        if graph_id not in self._graphs:
            raise ValueError(f"Graph '{graph_id}' not found")

        agent_id = str(uuid.uuid4())[:8]
        agent = {
            "id": agent_id,
            "name": name,
            "graph_id": graph_id,
            "status": "disabled",
            "triggers": triggers or [],
            "created_at": time.time(),
            "sessions_total": 0,
            "sessions_today": 0,
            "last_run": None,
        }
        self._agents[agent_id] = agent

        if self._agent_repo:
            await self._agent_repo.save(agent)

        logger.info("Deployed agent '%s' (%s) with graph '%s'", name, agent_id, graph_id)
        return agent

    def get_agent(self, agent_id: str) -> dict[str, Any] | None:
        return self._agents.get(agent_id)

    def list_agents(self) -> list[dict[str, Any]]:
        return list(self._agents.values())

    async def enable_agent(self, agent_id: str) -> dict[str, Any]:
        agent = self._agents.get(agent_id)
        if not agent:
            raise ValueError(f"Agent '{agent_id}' not found")

        agent["status"] = "enabled"

        # Register triggers
        for trigger in agent.get("triggers", []):
            trigger_type = trigger.get("type")

            if trigger_type == "schedule":
                self._scheduler.add(
                    agent_id,
                    interval_seconds=trigger.get("interval_seconds"),
                    cron=trigger.get("cron"),
                )

            elif trigger_type == "webhook":
                path = trigger.get("path", f"/webhooks/{agent['name']}")
                self._webhook_routes[path] = agent_id
                logger.info("Registered webhook %s → %s", path, agent_id)

        if self._agent_repo:
            await self._agent_repo.update_status(agent_id, "enabled")

        logger.info("Enabled agent '%s' (%s)", agent["name"], agent_id)
        return agent

    async def disable_agent(self, agent_id: str) -> dict[str, Any]:
        agent = self._agents.get(agent_id)
        if not agent:
            raise ValueError(f"Agent '{agent_id}' not found")

        agent["status"] = "disabled"

        # Unregister triggers
        self._scheduler.remove(agent_id)
        self._webhook_routes = {
            k: v for k, v in self._webhook_routes.items() if v != agent_id
        }

        if self._agent_repo:
            await self._agent_repo.update_status(agent_id, "disabled")

        logger.info("Disabled agent '%s' (%s)", agent["name"], agent_id)
        return agent

    async def destroy_agent(self, agent_id: str) -> None:
        agent = self._agents.pop(agent_id, None)
        if agent:
            self._scheduler.remove(agent_id)
            self._webhook_routes = {
                k: v for k, v in self._webhook_routes.items() if v != agent_id
            }
            if self._agent_repo:
                await self._agent_repo.delete(agent_id)
            logger.info("Destroyed agent '%s'", agent.get("name"))

    # --- Execution ---

    async def execute_agent(
        self,
        agent_id: str,
        trigger_data: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        agent = self._agents.get(agent_id)
        if not agent:
            raise ValueError(f"Agent '{agent_id}' not found")

        graph = self._graphs.get(agent["graph_id"])
        if not graph:
            raise ValueError(f"Graph '{agent['graph_id']}' not found for agent '{agent_id}'")

        # Find entry node (first trigger node, or first node)
        entry_node = None
        for node in graph.nodes:
            if node.tool_type.startswith("trigger/"):
                entry_node = node
                break
        if not entry_node:
            entry_node = graph.nodes[0] if graph.nodes else None
        if not entry_node:
            raise ValueError(f"No entry node found in graph '{graph.id}'")

        # Inject trigger data into entry node config
        if trigger_data:
            entry_config = dict(entry_node.config)
            entry_config["mock_payload"] = trigger_data
            # Create modified node with trigger data
            from datamirai_engine.core.graph import NodeDef
            modified_nodes = []
            for n in graph.nodes:
                if n.id == entry_node.id:
                    modified_nodes.append(NodeDef(
                        id=n.id, tool_type=n.tool_type,
                        version=n.version, config=entry_config,
                        position=n.position,
                    ))
                else:
                    modified_nodes.append(n)
            graph = GraphDef(
                id=graph.id, name=graph.name, version=graph.version,
                nodes=modified_nodes, edges=list(graph.edges),
                metadata=dict(graph.metadata),
            )

        # Execute
        session_id = str(uuid.uuid4())[:8]
        started_at = time.time()

        try:
            result = await self._runner.run(
                graph, context=self._context, entry_node_id=entry_node.id
            )
            session = {
                "id": session_id,
                "agent_id": agent_id,
                "agent_name": agent["name"],
                "graph_id": agent["graph_id"],
                "status": result.status,
                "trace": result.trace,
                "transcript": result.transcript,
                "state": result.state.snapshot(),
                "started_at": started_at,
                "finished_at": time.time(),
                "duration_ms": (time.time() - started_at) * 1000,
            }
        except (GraphExecutionError, MaxIterationsError) as e:
            session = {
                "id": session_id,
                "agent_id": agent_id,
                "agent_name": agent["name"],
                "graph_id": agent["graph_id"],
                "status": "failed",
                "error": str(e),
                "trace": [],
                "transcript": [],
                "started_at": started_at,
                "finished_at": time.time(),
                "duration_ms": (time.time() - started_at) * 1000,
            }

        self._sessions.append(session)
        agent["sessions_total"] += 1
        agent["last_run"] = time.time()

        if self._session_repo:
            await self._session_repo.save(session)

        status_emoji = "✓" if session["status"] == "completed" else "✗"
        logger.info(
            "%s Agent '%s' session %s — %s (%.0fms)",
            status_emoji, agent["name"], session_id,
            session["status"], session["duration_ms"],
        )

        return session

    # --- Webhook handling ---

    async def handle_webhook(
        self, path: str, body: dict[str, Any], headers: dict[str, str] | None = None,
    ) -> dict[str, Any] | None:
        agent_id = self._webhook_routes.get(path)
        if not agent_id:
            return None

        agent = self._agents.get(agent_id)
        if not agent or agent["status"] != "enabled":
            return None

        return await self.execute_agent(agent_id, trigger_data={
            "body": body,
            "headers": headers or {},
            "path": path,
        })

    def get_webhook_routes(self) -> dict[str, str]:
        return dict(self._webhook_routes)

    # --- Schedule callback ---

    async def _on_schedule_fire(self, agent_id: str, run_count: int) -> None:
        agent = self._agents.get(agent_id)
        if not agent or agent["status"] != "enabled":
            return
        await self.execute_agent(agent_id, trigger_data={
            "triggered_at": time.time(),
            "run_count": run_count,
            "trigger_type": "schedule",
        })

    # --- DB hydration ---

    async def load_from_db(self) -> None:
        """Load graphs and agents from database into memory. Call on startup."""
        if not self._graph_repo or not self._agent_repo:
            return

        from datamirai_engine.core.graph import EdgeDef, NodeDef

        graphs = await self._graph_repo.list_all()
        for g in graphs:
            nodes = [NodeDef(**n) for n in g.get("nodes", [])]
            edges = [EdgeDef(**e) for e in g.get("edges", [])]
            graph = GraphDef(
                id=g["id"], name=g["name"], version=g.get("version", "1.0"),
                nodes=nodes, edges=edges, metadata=g.get("metadata", {}),
            )
            self._graphs[graph.id] = graph

        agents = await self._agent_repo.list_all()
        for a in agents:
            a.setdefault("sessions_total", 0)
            a.setdefault("sessions_today", 0)
            a.setdefault("last_run", None)
            self._agents[a["id"]] = a

        logger.info("Loaded %d graphs, %d agents from database", len(graphs), len(agents))

    # --- Sessions ---

    def get_session(self, session_id: str) -> dict[str, Any] | None:
        """Get a single session by ID from in-memory cache."""
        for s in self._sessions:
            if s["id"] == session_id:
                return s
        return None

    async def get_session_from_db(self, session_id: str) -> dict[str, Any] | None:
        """Get a single session from DB. Falls back to in-memory if no repo."""
        if self._session_repo:
            return await self._session_repo.get(session_id)
        return self.get_session(session_id)

    def list_sessions(
        self, agent_id: str | None = None, limit: int = 50,
    ) -> list[dict[str, Any]]:
        sessions = self._sessions
        if agent_id:
            sessions = [s for s in sessions if s["agent_id"] == agent_id]
        return sorted(sessions, key=lambda s: s["started_at"], reverse=True)[:limit]

    # --- Status ---

    def status(self) -> dict[str, Any]:
        enabled = [a for a in self._agents.values() if a["status"] == "enabled"]
        return {
            "running": self._running,
            "agents_total": len(self._agents),
            "agents_enabled": len(enabled),
            "sessions_total": len(self._sessions),
            "scheduled_jobs": self._scheduler.list_jobs(),
            "webhook_routes": self._webhook_routes,
        }
