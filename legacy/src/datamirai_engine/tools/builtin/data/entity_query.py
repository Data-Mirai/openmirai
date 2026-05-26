"""Entity Query tool — queries entities by type, status, and filters.

FEAT-025: Generic entity tracking system. Queries entities stored
by entity_upsert.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

logger = logging.getLogger(__name__)

_ENTITY_TABLE_PREFIX = "entities_"


class EntityQueryTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/entity_query",
        version="1.0.0",
        display_name="Entity Query",
        description="Queries entities by type, status, and filters",
        category="data",
        icon="database",
        intents=[
            "consultar entidades por tipo y estado",
            "listar prospectos, tickets, ordenes por estado",
            "buscar entidades que cumplan ciertos criterios",
            "obtener la lista de leads en un estado especifico",
            "filtrar entidades por campos de datos",
        ],
        inputs=[
            ToolInput(name="status", type="string", required=False,
                      description="Filter by entity status"),
            ToolInput(name="query_params", type="object", required=False,
                      description="Additional filters on data fields"),
            ToolInput(name="limit", type="number", required=False, default=100,
                      description="Maximum number of results"),
        ],
        outputs=[
            ToolOutput(name="entities", type="array", description="List of matching entities"),
            ToolOutput(name="count", type="number", description="Number of entities found"),
        ],
        config=[
            ConfigField(name="entity_type", type="string", default="entity",
                        description="Type of entity to query (prospect, ticket, order, etc.)"),
            ConfigField(name="default_status", type="string", default="",
                        description="Default status filter (empty = all statuses)"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        entity_type = config.get("entity_type", "entity")
        table = f"{_ENTITY_TABLE_PREFIX}{entity_type}"
        default_status = config.get("default_status", "")

        status = inputs.get("status") or default_status
        limit = inputs.get("limit", 100)
        if isinstance(limit, str):
            try:
                limit = int(limit)
            except ValueError:
                limit = 100

        # Ensure table exists (may not have been created yet)
        columns = [
            {"name": "status", "type": "text", "nullable": True},
            {"name": "data", "type": "json", "nullable": True},
            {"name": "updated_at", "type": "timestamp", "nullable": True},
        ]
        if hasattr(context.db, "ensure_table"):
            await context.db.ensure_table(table, columns)

        # Build query
        conditions = []
        params: dict[str, Any] = {"limit": limit}

        if status:
            conditions.append("status = :status")
            params["status"] = status

        where = f" WHERE {' AND '.join(conditions)}" if conditions else ""
        query = f"SELECT * FROM {table}{where} ORDER BY created_at DESC LIMIT :limit"

        try:
            rows = await context.db.fetch_all(query, params)
        except Exception as e:
            logger.warning("Entity query failed (table may not exist): %s", e)
            return {"entities": [], "count": 0}

        # Parse data JSON field
        entities = []
        for row in rows:
            entity = dict(row)
            if isinstance(entity.get("data"), str):
                try:
                    entity["data"] = json.loads(entity["data"])
                except (json.JSONDecodeError, TypeError):
                    pass
            entity["entity_type"] = entity_type
            entities.append(entity)

        return {"entities": entities, "count": len(entities)}
