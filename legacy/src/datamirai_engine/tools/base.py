"""ToolSpec and BaseTool — Interface and ABC for all tools."""

from __future__ import annotations

import functools
import inspect
from abc import ABC, abstractmethod
from typing import Any, get_type_hints

from pydantic import BaseModel, Field

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.core.enums import ConfigFieldType, DataType


class ToolInput(BaseModel):
    """Declares an input port for a tool."""

    name: str
    type: str | DataType = DataType.STRING
    required: bool = True
    default: Any = None
    description: str = ""

    model_config = {"frozen": True}


class ToolOutput(BaseModel):
    """Declares an output port for a tool."""

    name: str
    type: str | DataType = DataType.STRING
    description: str = ""

    model_config = {"frozen": True}


class ConfigField(BaseModel):
    """Declares a configurable field shown in editor panel."""

    name: str
    type: str | ConfigFieldType = ConfigFieldType.STRING
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


# --- @tool decorator (FEAT-034 / API-05) ---

_PYTHON_TYPE_TO_DATATYPE: dict[type, DataType] = {
    str: DataType.STRING,
    int: DataType.NUMBER,
    float: DataType.NUMBER,
    bool: DataType.BOOLEAN,
    dict: DataType.OBJECT,
    list: DataType.ARRAY,
}


def _infer_data_type(python_type: type) -> DataType:
    """Map a Python type hint to a DataType enum."""
    origin = getattr(python_type, "__origin__", None)
    if origin is list:
        return DataType.ARRAY
    if origin is dict:
        return DataType.OBJECT
    return _PYTHON_TYPE_TO_DATATYPE.get(python_type, DataType.ANY)


def tool(
    fn: Any = None,
    *,
    tool_type: str | None = None,
    description: str | None = None,
    category: str = "custom",
    version: str = "1.0.0",
) -> Any:
    """Decorator that turns a plain function into a registrable BaseTool.

    Usage:
        @tool
        def get_weather(city: str) -> str:
            '''Gets weather for a city.'''
            return f"Sunny in {city}"

        # Or with explicit params:
        @tool(tool_type="custom/weather", description="Gets weather")
        def get_weather(city: str) -> str:
            return f"Sunny in {city}"

    The resulting object is a BaseTool subclass that can be registered:
        registry.register(get_weather)  # or registry.register(type(get_weather))
    """

    def _wrap(func: Any) -> type[BaseTool]:
        sig = inspect.signature(func)
        hints = get_type_hints(func)
        return_type = hints.pop("return", str)

        # Build inputs from parameters
        inputs: list[ToolInput] = []
        for param_name, param in sig.parameters.items():
            if param_name in ("self", "config", "context"):
                continue
            param_type = hints.get(param_name, str)
            has_default = param.default is not inspect.Parameter.empty
            inputs.append(ToolInput(
                name=param_name,
                type=_infer_data_type(param_type),
                required=not has_default,
                default=param.default if has_default else None,
                description="",
            ))

        # Build output
        out_type = _infer_data_type(return_type)
        outputs = [ToolOutput(name="result", type=out_type)]

        func_name = func.__name__
        resolved_tool_type = tool_type or f"{category}/{func_name}"
        resolved_description = description or func.__doc__ or func_name

        spec = ToolSpec(
            tool_type=resolved_tool_type,
            version=version,
            display_name=func_name.replace("_", " ").title(),
            description=resolved_description.strip(),
            category=category,
            inputs=inputs,
            outputs=outputs,
        )

        is_async = inspect.iscoroutinefunction(func)

        class _DecoratedTool(BaseTool):
            nonlocal spec
            _spec = spec
            # Class-level spec required by BaseTool
            __qualname__ = f"{func_name}_Tool"

            async def execute(self, inputs_dict, config, context):
                # Pass only known params
                kwargs = {k: v for k, v in inputs_dict.items() if k in sig.parameters}
                if is_async:
                    result = await func(**kwargs)
                else:
                    result = func(**kwargs)
                if isinstance(result, dict):
                    return result
                return {"result": result}

        _DecoratedTool.spec = spec
        _DecoratedTool.__name__ = f"{func_name}_Tool"

        # Preserve function metadata
        functools.update_wrapper(_DecoratedTool, func, updated=[])

        return _DecoratedTool

    if fn is not None:
        # Called as @tool without parentheses
        return _wrap(fn)
    # Called as @tool(...) with parentheses
    return _wrap
