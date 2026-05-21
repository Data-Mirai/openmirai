"""ToolSpec and BaseTool — Interface and ABC for all tools."""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any

from pydantic import BaseModel, Field

from datamirai_engine.core.context import ExecutionContext


class ToolInput(BaseModel):
    """Declares an input port for a tool."""

    name: str
    type: str  # string | number | boolean | object | array | any
    required: bool = True
    default: Any = None
    description: str = ""

    model_config = {"frozen": True}


class ToolOutput(BaseModel):
    """Declares an output port for a tool."""

    name: str
    type: str
    description: str = ""

    model_config = {"frozen": True}


class ConfigField(BaseModel):
    """Declares a configurable field shown in editor panel."""

    name: str
    type: str  # string | number | boolean | select | slider | object
    default: Any = None
    description: str = ""
    options: list[Any] | None = None  # for select type
    required: bool = False  # if True, graph cannot be saved/executed without this field

    model_config = {"frozen": True}


class ToolSpec(BaseModel):
    """Declarative identity and contract for a tool.

    Defines what a tool is, what it accepts, what it produces,
    and how it can be configured in the editor.
    """

    tool_type: str  # category/name format (e.g., "ai/llm_call")
    version: str
    display_name: str
    description: str
    category: str
    icon: str | None = None
    intents: list[str] = Field(default_factory=list)
    inputs: list[ToolInput] = Field(default_factory=list)
    outputs: list[ToolOutput] = Field(default_factory=list)
    config: list[ConfigField] = Field(default_factory=list)

    model_config = {"frozen": True}


class BaseTool(ABC):
    """Abstract base class for all tools.

    Subclasses MUST define a class-level `spec` attribute and implement `execute()`.
    Call `run()` to execute with input validation and config defaults.
    """

    spec: ToolSpec

    async def run(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        """Validate inputs, apply config defaults, execute, return output."""
        validated_inputs = self._validate_inputs(inputs)
        merged_config = self._apply_config_defaults(config)
        return await self.execute(validated_inputs, merged_config, context)

    @abstractmethod
    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        """Execute tool logic. Implemented by each concrete tool."""
        ...

    def _validate_inputs(self, inputs: dict[str, Any]) -> dict[str, Any]:
        """Check required inputs are present. Apply defaults for optional."""
        validated = dict(inputs)
        missing = []

        for input_def in self.spec.inputs:
            if input_def.name not in validated:
                if input_def.required:
                    missing.append(input_def.name)
                elif input_def.default is not None:
                    validated[input_def.name] = input_def.default

        if missing:
            raise ValueError(
                f"Missing required input(s) for tool '{self.spec.tool_type}': "
                f"{', '.join(missing)}"
            )

        return validated

    def _apply_config_defaults(self, config: dict[str, Any]) -> dict[str, Any]:
        """Merge user config with defaults from spec."""
        merged = {}
        for field in self.spec.config:
            merged[field.name] = config.get(field.name, field.default)
        # Include any extra config keys not in spec
        for key, value in config.items():
            if key not in merged:
                merged[key] = value
        return merged
