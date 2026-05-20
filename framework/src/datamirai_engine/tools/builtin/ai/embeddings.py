"""Embeddings tool — generates vector embeddings from text."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class EmbeddingsTool(BaseTool):
    spec = ToolSpec(
        tool_type="ai/embeddings",
        version="1.0.0",
        display_name="Generate Embeddings",
        description="Generates vector embedding from text input",
        category="ai",
        icon="hash",
        intents=[
            "generar vectores para busqueda semantica o similitud",
            "indexar documentos en base de datos vectorial",
            "preparar datos para RAG (retrieval augmented generation)",
        ],
        inputs=[
            ToolInput(name="text", type="string", required=True),
        ],
        outputs=[
            ToolOutput(name="embedding", type="array"),
            ToolOutput(name="dimension", type="number"),
        ],
        config=[
            ConfigField(name="model", type="string", default=None),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        embedding = await context.llm.embed(
            inputs["text"],
            model=config.get("model"),
        )
        return {
            "embedding": embedding,
            "dimension": len(embedding),
        }
