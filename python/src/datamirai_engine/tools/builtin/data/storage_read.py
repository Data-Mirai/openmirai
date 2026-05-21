"""Storage Read tool — reads files from S3-compatible storage."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class StorageReadTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/storage_read",
        version="1.0.0",
        display_name="Storage Read",
        description="Reads file from S3-compatible storage or generates presigned URL",
        category="data",
        icon="download",
        intents=[
            "leer archivos almacenados (documentos, imagenes, CSVs)",
            "generar URL temporal para descargar un archivo",
            "obtener contenido de archivos para procesamiento",
        ],
        inputs=[
            ToolInput(name="key", type="string", required=True),
        ],
        outputs=[
            ToolOutput(name="content", type="string"),
            ToolOutput(name="found", type="boolean"),
            ToolOutput(name="url", type="string"),
        ],
        config=[
            ConfigField(name="mode", type="select", default="read",
                        options=["read", "presign"]),
            ConfigField(name="expires_in", type="number", default=3600),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        key = inputs["key"]
        mode = config.get("mode", "read")

        if mode == "presign":
            try:
                url = await context.storage.presign(
                    key, expires_in=int(config.get("expires_in", 3600))
                )
                return {"content": None, "found": True, "url": url}
            except FileNotFoundError:
                return {"content": None, "found": False, "url": None}

        try:
            data = await context.storage.get(key)
            content = data.decode("utf-8") if isinstance(data, bytes) else data
            return {"content": content, "found": True, "url": None}
        except FileNotFoundError:
            return {"content": None, "found": False, "url": None}
