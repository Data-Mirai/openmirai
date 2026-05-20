"""Storage Write tool — writes files to S3-compatible storage."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class StorageWriteTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/storage_write",
        version="1.0.0",
        display_name="Storage Write",
        description="Writes file to S3-compatible storage",
        category="data",
        icon="upload",
        intents=[
            "guardar archivos generados (reportes, exports, backups)",
            "subir documentos o contenido al storage",
            "persistir resultados como archivos descargables",
        ],
        inputs=[
            ToolInput(name="key", type="string", required=True),
            ToolInput(name="content", type="string", required=True),
        ],
        outputs=[
            ToolOutput(name="written", type="boolean"),
            ToolOutput(name="key", type="string"),
        ],
        config=[
            ConfigField(name="content_type", type="string", default=None),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        key = inputs["key"]
        content = inputs["content"]
        data = content.encode("utf-8") if isinstance(content, str) else content
        await context.storage.put(
            key, data, content_type=config.get("content_type")
        )
        return {"written": True, "key": key}
