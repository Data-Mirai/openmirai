"""DB Write tool — writes data to relational database with schema support.

FEAT-019: Generates real SQL, supports schema definition, auto-creates tables.
"""

from __future__ import annotations

import json
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class DBWriteTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/db_write",
        version="2.0.0",
        display_name="DB Write",
        description="Writes data to relational database with automatic table creation",
        category="data",
        icon="database",
        intents=[
            "guardar resultados procesados en base de datos",
            "persistir datos generados por el agente para consulta futura",
            "escribir registros nuevos o actualizar existentes",
            "guardar datos de web scrape para trazabilidad",
            "persistir analisis de LLM para auditoria",
        ],
        inputs=[
            ToolInput(name="data", type="object", required=True,
                       description="Row data to write"),
        ],
        outputs=[
            ToolOutput(name="written", type="boolean"),
            ToolOutput(name="table", type="string"),
            ToolOutput(name="row_id", type="string"),
        ],
        config=[
            ConfigField(name="table", type="string", default="default",
                        description="Target table name"),
            ConfigField(name="mode", type="select", default="insert",
                        options=["insert", "upsert"],
                        description="Write mode: insert (new rows) or upsert (update if exists)"),
            ConfigField(name="schema", type="object", default=None,
                        description="Column definitions: [{name, type, nullable}]. Types: text, integer, float, boolean, json, timestamp"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        table = config.get("table", "default")
        data = inputs.get("data", {})

        # Handle case where data is a JSON string
        if isinstance(data, str):
            try:
                data = json.loads(data)
            except (json.JSONDecodeError, TypeError):
                data = {"content": data}

        # Ensure data is a dict
        if not isinstance(data, dict):
            data = {"content": str(data)}

        # Parse schema from config
        schema = config.get("schema")
        if isinstance(schema, str):
            try:
                schema = json.loads(schema)
            except (json.JSONDecodeError, TypeError):
                schema = None

        # Use new schema-aware methods if available
        if hasattr(context.db, "ensure_table") and schema:
            columns = schema if isinstance(schema, list) else []
            await context.db.ensure_table(table, columns)

            # Filter data to only include columns defined in schema + system columns
            allowed_cols = {c["name"] for c in columns if isinstance(c, dict)}
            allowed_cols.update({"id", "created_at", "session_id", "node_id"})
            data = {k: v for k, v in data.items() if k in allowed_cols}

        if hasattr(context.db, "insert"):
            meta = {}
            if hasattr(context, "session_id"):
                meta["session_id"] = context.session_id or ""
            row = await context.db.insert(table, data, meta)
            return {"written": True, "table": table, "row_id": row.get("id", "")}

        # Legacy fallback for InMemoryDBResource without new methods
        params = {**data, "__table__": table}
        await context.db.execute("INSERT", params)
        return {"written": True, "table": table, "row_id": data.get("id", "")}
