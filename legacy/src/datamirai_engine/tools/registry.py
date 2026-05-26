"""ToolRegistry — Catalog of available tools with autodiscovery."""

from __future__ import annotations

import importlib
import inspect
import logging
import sys

from datamirai_engine.tools.base import BaseTool, ToolSpec

logger = logging.getLogger(__name__)


class ToolRegistry:
    """Registry of available tool classes, indexed by tool_type.

    Supports manual registration and autodiscovery from Python modules.
    """

    def __init__(self) -> None:
        self._tools: dict[str, type[BaseTool]] = {}

    def register(self, tool_cls: type[BaseTool], *, replace: bool = False) -> None:
        """Register a tool class. Raises ValueError if already registered (unless replace=True)."""
        tool_type = tool_cls.spec.tool_type
        if tool_type in self._tools and not replace:
            raise ValueError(
                f"Tool '{tool_type}' is already registered. "
                f"Use replace=True to override."
            )
        self._tools[tool_type] = tool_cls

    def get(self, tool_type: str) -> type[BaseTool] | None:
        """Get tool class by tool_type. Returns None if not found."""
        return self._tools.get(tool_type)

    def list_all(self) -> list[ToolSpec]:
        """Return specs of all registered tools."""
        return [cls.spec for cls in self._tools.values()]

    def list_by_category(self, category: str) -> list[ToolSpec]:
        """Return specs of tools in a given category."""
        return [cls.spec for cls in self._tools.values() if cls.spec.category == category]

    def categories(self) -> set[str]:
        """Return set of all registered categories."""
        return {cls.spec.category for cls in self._tools.values()}

    def discover(self, module_path: str) -> int:
        """Autodiscover BaseTool subclasses in a Python module.

        Imports the module and scans for classes that:
        1. Are subclasses of BaseTool
        2. Are not BaseTool itself
        3. Have a `spec` attribute

        Returns count of newly registered tools.
        """
        if module_path in sys.modules:
            module = sys.modules[module_path]
        else:
            module = importlib.import_module(module_path)

        count = 0
        for _name, obj in inspect.getmembers(module, inspect.isclass):
            if (
                issubclass(obj, BaseTool)
                and obj is not BaseTool
                and hasattr(obj, "spec")
                and isinstance(obj.spec, ToolSpec)
            ):
                tool_type = obj.spec.tool_type
                if tool_type not in self._tools:
                    self._tools[tool_type] = obj
                    count += 1
                    logger.debug("Discovered tool: %s (%s)", tool_type, obj.__name__)

        return count

    _BUILTIN_MODULES = [
        "datamirai_engine.tools.builtin.trigger.triggers",
        "datamirai_engine.tools.builtin.logic.condition",
        "datamirai_engine.tools.builtin.logic.switch",
        "datamirai_engine.tools.builtin.logic.merge",
        "datamirai_engine.tools.builtin.logic.wait",
        "datamirai_engine.tools.builtin.logic.loop",
        "datamirai_engine.tools.builtin.logic.human_input",
        "datamirai_engine.tools.builtin.ai.llm_call",
        "datamirai_engine.tools.builtin.ai.transcribe",
        "datamirai_engine.tools.builtin.ai.embeddings",
        "datamirai_engine.tools.builtin.data.db_read",
        "datamirai_engine.tools.builtin.data.db_write",
        "datamirai_engine.tools.builtin.data.storage_read",
        "datamirai_engine.tools.builtin.data.storage_write",
        "datamirai_engine.tools.builtin.output.response",
        "datamirai_engine.tools.builtin.agent.run_agent",
    ]

    @classmethod
    def default(cls) -> ToolRegistry:
        """Create registry with all builtin tools pre-registered.

        FEAT-034 / API-06: convenience factory equivalent to manual discover().

            registry = ToolRegistry.default()
            # All builtin tools ready to use
        """
        registry = cls()
        for module_path in cls._BUILTIN_MODULES:
            try:
                registry.discover(module_path)
            except (ImportError, Exception) as e:
                logger.debug("Skipping builtin %s: %s", module_path, e)
        return registry
