"""Transcribe tool — transcribes audio via LLM provider."""

from __future__ import annotations

from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext


class TranscribeTool(BaseTool):
    spec = ToolSpec(
        tool_type="ai/transcribe",
        version="1.0.0",
        display_name="Transcribe Audio",
        description="Transcribes audio file to text",
        category="ai",
        icon="mic",
        intents=[
            "convertir audio o grabaciones a texto",
            "transcripcion de reuniones, entrevistas, podcasts, notas de voz",
        ],
        inputs=[
            ToolInput(name="audio_key", type="string", required=True,
                       description="Storage key of audio file"),
        ],
        outputs=[
            ToolOutput(name="text", type="string"),
            ToolOutput(name="duration_seconds", type="number"),
        ],
        config=[
            ConfigField(name="model", type="string", default="whisper-1"),
            ConfigField(name="language", type="string", default="auto"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        # In production, this would download audio from storage and send to transcription API.
        # For now, uses LLM as mock transcription.
        result = await context.llm.call(
            model=config.get("model", "whisper-1"),
            prompt=f"transcribe:{inputs['audio_key']}",
        )
        text = result.response if hasattr(result, "response") else result.get("text", "")
        return {
            "text": text,
            "duration_seconds": 0.0,
        }
