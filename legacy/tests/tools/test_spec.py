"""Tests for ToolSpec, BaseTool, and runtime validation."""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import ValidationError

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext

# --- Concrete test blocks ---


class EchoTool(BaseTool):
    spec = ToolSpec(
        tool_type="test/echo",
        version="1.0.0",
        display_name="Echo",
        description="Returns inputs as outputs",
        category="test",
        inputs=[ToolInput(name="message", type="string", required=True)],
        outputs=[ToolOutput(name="echo", type="string")],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        return {"echo": inputs["message"]}


class ConfigurableTool(BaseTool):
    spec = ToolSpec(
        tool_type="test/configurable",
        version="1.0.0",
        display_name="Configurable",
        description="Tool with config options",
        category="test",
        inputs=[ToolInput(name="value", type="number", required=True)],
        outputs=[ToolOutput(name="result", type="number")],
        config=[
            ConfigField(name="multiplier", type="number", default=1.0),
            ConfigField(name="mode", type="select", default="normal", options=["normal", "turbo"]),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        mult = config.get("multiplier", 1.0)
        if config.get("mode") == "turbo":
            mult *= 2
        return {"result": inputs["value"] * mult}


class OptionalInputTool(BaseTool):
    spec = ToolSpec(
        tool_type="test/optional",
        version="1.0.0",
        display_name="Optional Input",
        description="Tool with optional input",
        category="test",
        inputs=[
            ToolInput(name="required_field", type="string", required=True),
            ToolInput(name="optional_field", type="string", required=False, default="fallback"),
        ],
        outputs=[ToolOutput(name="combined", type="string")],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        req = inputs["required_field"]
        opt = inputs.get("optional_field", "fallback")
        return {"combined": f"{req}-{opt}"}


# --- ToolSpec tests ---


class TestToolSpec:
    def test_create_minimal(self):
        spec = ToolSpec(
            tool_type="ai/llm_call",
            version="1.0.0",
            display_name="LLM Call",
            description="Calls an LLM",
            category="ai",
        )
        assert spec.tool_type == "ai/llm_call"
        assert spec.inputs == []
        assert spec.outputs == []
        assert spec.config == []
        assert spec.icon is None

    def test_create_full(self):
        spec = ToolSpec(
            tool_type="ai/llm_call",
            version="2.0.0",
            display_name="LLM Call",
            description="Calls an LLM",
            category="ai",
            icon="brain",
            inputs=[
                ToolInput(name="prompt", type="string", required=True, description="The prompt"),
            ],
            outputs=[
                ToolOutput(name="response", type="string"),
                ToolOutput(name="tokens", type="number"),
            ],
            config=[
                ConfigField(
                    name="model", type="select", default="claude", options=["claude", "gpt"]
                ),
                ConfigField(name="temperature", type="number", default=0.7),
            ],
        )
        assert len(spec.inputs) == 1
        assert len(spec.outputs) == 2
        assert len(spec.config) == 2
        assert spec.icon == "brain"

    def test_tool_type_required(self):
        with pytest.raises(ValidationError):
            ToolSpec(version="1.0.0", display_name="X", description="X", category="x")

    def test_version_required(self):
        with pytest.raises(ValidationError):
            ToolSpec(tool_type="a/b", display_name="X", description="X", category="x")

    def test_serialization_roundtrip(self):
        spec = EchoTool.spec
        data = spec.model_dump()
        restored = ToolSpec.model_validate(data)
        assert restored.tool_type == spec.tool_type
        assert len(restored.inputs) == len(spec.inputs)

    def test_json_roundtrip(self):
        spec = EchoTool.spec
        json_str = spec.model_dump_json()
        restored = ToolSpec.model_validate_json(json_str)
        assert restored == spec


# --- BaseTool tests ---


class TestBaseTool:
    @pytest.mark.asyncio
    async def test_echo_block_executes(self):
        tool = EchoTool()
        result = await tool.run({"message": "hello"}, {}, None)
        assert result == {"echo": "hello"}

    @pytest.mark.asyncio
    async def test_configurable_block_default_config(self):
        tool = ConfigurableTool()
        result = await tool.run({"value": 10}, {}, None)
        assert result == {"result": 10.0}

    @pytest.mark.asyncio
    async def test_configurable_block_custom_config(self):
        tool = ConfigurableTool()
        result = await tool.run({"value": 10}, {"multiplier": 3.0}, None)
        assert result == {"result": 30.0}

    @pytest.mark.asyncio
    async def test_configurable_block_turbo_mode(self):
        tool = ConfigurableTool()
        result = await tool.run({"value": 5}, {"multiplier": 2.0, "mode": "turbo"}, None)
        assert result == {"result": 20.0}

    def test_block_has_spec(self):
        tool = EchoTool()
        assert tool.spec.tool_type == "test/echo"
        assert tool.spec.version == "1.0.0"


# --- Runtime validation tests ---


class TestRuntimeValidation:
    @pytest.mark.asyncio
    async def test_missing_required_input_raises(self):
        tool = EchoTool()
        with pytest.raises(ValueError, match=r"Missing required input.*message"):
            await tool.run({}, {}, None)

    @pytest.mark.asyncio
    async def test_optional_input_uses_default(self):
        tool = OptionalInputTool()
        result = await tool.run({"required_field": "hello"}, {}, None)
        assert result == {"combined": "hello-fallback"}

    @pytest.mark.asyncio
    async def test_optional_input_provided(self):
        tool = OptionalInputTool()
        result = await tool.run(
            {"required_field": "hello", "optional_field": "world"}, {}, None
        )
        assert result == {"combined": "hello-world"}

    @pytest.mark.asyncio
    async def test_config_defaults_applied(self):
        tool = ConfigurableTool()
        # No config provided — defaults should be applied
        result = await tool.run({"value": 10}, {}, None)
        assert result == {"result": 10.0}

    @pytest.mark.asyncio
    async def test_validate_outputs(self):
        """Tool output should contain declared output fields."""
        tool = EchoTool()
        result = await tool.run({"message": "test"}, {}, None)
        # Verify output has declared fields
        for output_def in tool.spec.outputs:
            assert output_def.name in result
