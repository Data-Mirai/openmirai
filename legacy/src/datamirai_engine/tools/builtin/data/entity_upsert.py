"""Entity Upsert tool — creates or updates entities with state tracking.

FEAT-025: Generic entity tracking system. Entities are business objects
(prospects, tickets, orders, leads) with status lifecycle.
"""

from __future__ import annotations

import json
import logging
import uuid
from datetime import datetime, timezone
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

logger = logging.getLogger(__name__)

_ENTITY_TABLE_PREFIX = "entities_"


class EntityUpsertTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/entity_upsert",
        version="1.0.0",
        display_name="Entity Upsert",
        description="Creates or updates an entity with status tracking (prospect, ticket, order, lead, etc.)",
        category="data",
        icon="database",
        intents=[
            "crear o actualizar una entidad con estado (prospecto, ticket, orden)",
            "trackear el ciclo de vida de un objeto de negocio",
            "cambiar el estado de una entidad existente",
            "registrar un nuevo prospecto, lead, o ticket",
            "actualizar datos de una entidad existente",
        ],
        inputs=[
            ToolInput(name="data", type="object", required=False,
                      description="Entity data fields (name, email, etc.)"),
            ToolInput(name="status", type="string", required=False,
                      description="New status for the entity"),
        ],
        outputs=[
            ToolOutput(name="entity", type="object", description="The created/updated entity with id, status, timestamps"),
            ToolOutput(name="created", type="boolean", description="True if entity was created, False if updated"),
            ToolOutput(name="previous_status", type="string", description="Previous status before update (empty if new)"),
        ],
        config=[
            ConfigField(name="entity_type", type="string", default="entity",
                        description="Type of entity (prospect, ticket, order, lead, etc.)"),
            ConfigField(name="match_field", type="string", default="",
                        description="Field to match for upsert (e.g. 'email'). Empty = always create new."),
            ConfigField(name="static_data", type="object", default=None,
                        description="Static entity data (used when data is not passed via data_map)"),
            ConfigField(name="static_status", type="string", default="",
                        description="Static status to set (used when status is not passed via data_map)"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        entity_type = config.get("entity_type", "entity")
        match_field = config.get("match_field", "")
        table = f"{_ENTITY_TABLE_PREFIX}{entity_type}"

        # Data: input overrides config.static_data
        data = inputs.get("data") or config.get("static_data") or {}
        if isinstance(data, str):
            try:
                data = json.loads(data)
            except (json.JSONDecodeError, TypeError):
                data = {"content": data}
        if not isinstance(data, dict):
            data = {"content": str(data)}

        # Status: input overrides config.static_status
        status = inputs.get("status") or config.get("static_status") or "new"

        now = datetime.now(timezone.utc).isoformat()

        # Ensure table exists
        columns = [
            {"name": "status", "type": "text", "nullable": True},
            {"name": "data", "type": "json", "nullable": True},
            {"name": "updated_at", "type": "timestamp", "nullable": True},
        ]
        if hasattr(context.db, "ensure_table"):
            await context.db.ensure_table(table, columns)

        # Try to find existing entity
        existing = None
        if match_field and data.get(match_field):
            try:
                rows = await context.db.fetch_all(
                    f"SELECT * FROM {table} WHERE json_extract(data, '$.{match_field}') = :val LIMIT 1",
                    {"val": str(data[match_field])},
                )
                if rows:
                    existing = rows[0]
            except Exception:
                pass

        # Also try matching by id if data has an id field
        if not existing and data.get("id"):
            try:
                row = await context.db.fetch_one(
                    f"SELECT * FROM {table} WHERE id = :id",
                    {"id": data["id"]},
                )
                if row:
                    existing = row
            except Exception:
                pass

        if existing:
            # Update existing entity
            previous_status = existing.get("status", "")

            # Merge data
            existing_data = existing.get("data", {})
            if isinstance(existing_data, str):
                try:
                    existing_data = json.loads(existing_data)
                except (json.JSONDecodeError, TypeError):
                    existing_data = {}
            merged_data = {**existing_data, **data}

            await context.db.execute(
                f"UPDATE {table} SET status = :status, data = :data, updated_at = :updated_at WHERE id = :id",
                {
                    "id": existing["id"],
                    "status": status,
                    "data": json.dumps(merged_data),
                    "updated_at": now,
                },
            )

            entity = {
                "id": existing["id"],
                "status": status,
                "data": merged_data,
                "created_at": existing.get("created_at", ""),
                "updated_at": now,
                "entity_type": entity_type,
            }
            return {"entity": entity, "created": False, "previous_status": previous_status}

        else:
            # Create new entity
            entity_id = str(uuid.uuid4())

            if hasattr(context.db, "insert"):
                meta = {}
                if hasattr(context, "session_id"):
                    meta["session_id"] = context.session_id or ""
                row = await context.db.insert(table, {
                    "status": status,
                    "data": json.dumps(data),
                    "updated_at": now,
                }, meta)
                entity_id = row.get("id", entity_id)
            else:
                await context.db.execute(
                    f"INSERT INTO {table} (id, status, data, created_at, updated_at) VALUES (:id, :status, :data, :created_at, :updated_at)",
                    {
                        "id": entity_id,
                        "status": status,
                        "data": json.dumps(data),
                        "created_at": now,
                        "updated_at": now,
                    },
                )

            entity = {
                "id": entity_id,
                "status": status,
                "data": data,
                "created_at": now,
                "updated_at": now,
                "entity_type": entity_type,
            }
            return {"entity": entity, "created": True, "previous_status": ""}
