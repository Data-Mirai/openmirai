"""DB Read tool — reads data from relational database with query support.

FEAT-019: Generates real SQL, supports filtering, pagination, ordering.
"""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class DBReadTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/db_read",
        version="2.0.0",
        display_name="DB Read",
        description="Reads data from relational database with filtering and pagination",
        category="data",
        icon="database",
        intents=[
            "leer datos de una tabla o base de datos relacional",
            "consultar registros existentes para procesarlos o mostrarlos",
            "obtener datos de contexto para que el LLM los analice",
            "leer datos guardados por otro agente",
        ],
        inputs=[
            ToolInput(name="query_params", type="object", required=False,
                       description="Filter parameters: {column: value} for WHERE clause"),
        ],
        outputs=[
            ToolOutput(name="data", type="any"),
            ToolOutput(name="count", type="number"),
        ],
        config=[
            ConfigField(name="table", type="string", default="default",
                        description="Table to read from"),
            ConfigField(name="mode", type="select", default="all",
                        options=["one", "all"],
                        description="Read one row (by id) or all matching rows"),
            ConfigField(name="limit", type="number", default=100,
                        description="Max rows to return (mode=all)"),
            ConfigField(name="order_by", type="string", default="created_at DESC",
                        description="Column + direction for ordering (e.g. 'created_at DESC')"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        table = config.get("table", "default")
        mode = config.get("mode", "all")
        query_params = inputs.get("query_params", {}) or {}

        # Use new schema-aware query() if available
        if hasattr(context.db, "query"):
            limit = int(config.get("limit", 100)) if mode == "all" else 1
            order_by = config.get("order_by", "created_at DESC") if mode == "all" else None
            where = query_params if query_params else None

            rows = await context.db.query(
                table, where=where, limit=limit, order_by=order_by
            )

            if mode == "one":
                row = rows[0] if rows else None
                return {"data": row, "count": 1 if row else 0}
            return {"data": rows, "count": len(rows)}

        # Legacy fallback
        params = {**query_params, "__table__": table}
        if mode == "all":
            rows = await context.db.fetch_all("SELECT", params)
            return {"data": rows, "count": len(rows)}
        else:
            row = await context.db.fetch_one("SELECT", params)
            return {"data": row, "count": 1 if row else 0}
