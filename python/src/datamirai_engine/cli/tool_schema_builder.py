"""tool_schema_builder — converts BaseTool blocks into OpenAI-format
tool schemas and handles execution of tool calls from the LLM.
"""

from __future__ import annotations

import os
from typing import Any

from datamirai_engine.tools.base import BaseTool, ToolSpec
from datamirai_engine.tools.registry import ToolRegistry


# ---------------------------------------------------------------------------
# BaseTool → OpenAI function-calling JSON Schema
# ---------------------------------------------------------------------------

_TYPE_MAP = {
    "string": "string",
    "number": "number",
    "boolean": "boolean",
    "object": "object",
    "array": "array",
    "any": "string",
}


def _spec_to_openai_schema(spec: ToolSpec) -> dict[str, Any]:
    """Convert a ToolSpec into an OpenAI-compatible tool definition."""
    properties: dict[str, Any] = {}
    required: list[str] = []

    for inp in spec.inputs:
        prop: dict[str, Any] = {"type": _TYPE_MAP.get(inp.type, "string")}
        if inp.description:
            prop["description"] = inp.description
        properties[inp.name] = prop
        if inp.required:
            required.append(inp.name)

    for cfg in spec.config:
        if cfg.name in properties:
            continue
        prop = {"type": _TYPE_MAP.get(cfg.type, "string")}
        if cfg.description:
            prop["description"] = cfg.description
        if cfg.default is not None:
            prop["default"] = cfg.default
        if cfg.options:
            prop["enum"] = cfg.options
        properties[cfg.name] = prop

    return {
        "type": "function",
        "function": {
            "name": spec.tool_type.replace("/", "_"),
            "description": spec.description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        },
    }


# ---------------------------------------------------------------------------
# Expose ALL registered tools dynamically (minus graph-only exclusions)
# ---------------------------------------------------------------------------

_EXCLUDED_TOOL_TYPES = frozenset({
    "trigger/webhook",
    "trigger/manual",
    "trigger/schedule",
    "trigger/heartbeat",
    "output/response",
})

# Core tools for local models (small context, limited tool handling)
_CORE_TOOL_TYPES = frozenset({
    "filesystem/read_file",
    "filesystem/write_file",
    "filesystem/edit_file",
    "filesystem/glob",
    "filesystem/grep",
    "filesystem/list_dir",
    "filesystem/tree",
    "filesystem/mkdir",
    "filesystem/delete",
    "system/bash",
    "git/status",
    "git/diff",
    "git/log",
    "git/commit",
})


def build_tool_schemas(
    registry: ToolRegistry,
    mode: str = "core",
) -> list[dict[str, Any]]:
    """Build OpenAI-format tool schemas.

    Modes:
      - "core": Only essential coding tools (14 tools). Best for local models.
      - "all": All registered tools minus exclusions. For large/cloud models.
    """
    schemas: list[dict[str, Any]] = []
    for spec in registry.list_all():
        if spec.tool_type in _EXCLUDED_TOOL_TYPES:
            continue
        if mode == "core" and spec.tool_type not in _CORE_TOOL_TYPES:
            continue
        schemas.append(_spec_to_openai_schema(spec))
    return schemas


def get_tool_name_map(registry: ToolRegistry) -> dict[str, str]:
    """Map function names (underscores) → tool_type (slashes)."""
    mapping: dict[str, str] = {}
    for spec in registry.list_all():
        if spec.tool_type in _EXCLUDED_TOOL_TYPES:
            continue
        fn_name = spec.tool_type.replace("/", "_")
        mapping[fn_name] = spec.tool_type
    return mapping


# ---------------------------------------------------------------------------
# Tool executor
# ---------------------------------------------------------------------------

async def execute_tool(
    registry: ToolRegistry,
    tool_type: str,
    arguments: dict[str, Any],
    cwd: str | None = None,
) -> dict[str, Any]:
    """Execute a tool by type with the given arguments."""
    tool_cls = registry.get(tool_type)
    if tool_cls is None:
        return {"error": f"Unknown tool: {tool_type}"}

    tool: BaseTool = tool_cls()
    spec = tool.spec

    input_names = {i.name for i in spec.inputs}
    config_names = {c.name for c in spec.config}

    inputs: dict[str, Any] = {}
    config: dict[str, Any] = {}

    for key, value in arguments.items():
        if key in input_names:
            inputs[key] = value
        elif key in config_names:
            config[key] = value
        else:
            inputs[key] = value

    if cwd:
        for key in ("path", "source", "destination"):
            if key in inputs and inputs[key] and not os.path.isabs(inputs[key]):
                inputs[key] = os.path.join(cwd, inputs[key])
        if "cwd" not in config and "cwd" in config_names:
            config["cwd"] = cwd

    try:
        result = await tool.execute(inputs, config, context=None)
        return result
    except Exception as exc:
        return {"error": f"{type(exc).__name__}: {exc}"}
