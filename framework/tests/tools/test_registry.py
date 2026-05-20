"""Tests for ToolRegistry + autodiscovery."""

from __future__ import annotations

import pytest

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec
from datamirai_engine.tools.registry import ToolRegistry

# --- Test blocks for registry ---


class AlphaTool(BaseTool):
    spec = ToolSpec(
        tool_type="test/alpha",
        version="1.0.0",
        display_name="Alpha",
        description="First test block",
        category="test",
        inputs=[ToolInput(name="x", type="number", required=True)],
        outputs=[ToolOutput(name="y", type="number")],
    )

    async def execute(self, inputs, config, context):
        return {"y": inputs["x"] * 2}


class BetaTool(BaseTool):
    spec = ToolSpec(
        tool_type="test/beta",
        version="1.0.0",
        display_name="Beta",
        description="Second test block",
        category="test",
    )

    async def execute(self, inputs, config, context):
        return {}


class AlphaToolV2(BaseTool):
    spec = ToolSpec(
        tool_type="test/alpha",
        version="2.0.0",
        display_name="Alpha v2",
        description="Updated alpha",
        category="test",
    )

    async def execute(self, inputs, config, context):
        return {"y": 0}


# --- Tests ---


class TestToolRegistry:
    def test_register_block(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        assert registry.get("test/alpha") is AlphaTool

    def test_register_multiple(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        registry.register(BetaTool)
        assert len(registry.list_all()) == 2

    def test_get_nonexistent_returns_none(self):
        registry = ToolRegistry()
        assert registry.get("test/nonexistent") is None

    def test_duplicate_registration_raises(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        with pytest.raises(ValueError, match="already registered"):
            registry.register(AlphaToolV2)

    def test_duplicate_registration_with_replace(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        registry.register(AlphaToolV2, replace=True)
        tool_cls = registry.get("test/alpha")
        assert tool_cls is AlphaToolV2
        assert tool_cls.spec.version == "2.0.0"

    def test_list_all_returns_specs(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        registry.register(BetaTool)
        specs = registry.list_all()
        tool_types = {s.tool_type for s in specs}
        assert tool_types == {"test/alpha", "test/beta"}

    def test_list_by_category(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        registry.register(BetaTool)
        specs = registry.list_by_category("test")
        assert len(specs) == 2

    def test_list_by_category_empty(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        assert registry.list_by_category("ai") == []

    @pytest.mark.asyncio
    async def test_instantiate_and_run(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        tool_cls = registry.get("test/alpha")
        tool = tool_cls()
        result = await tool.run({"x": 5}, {}, None)
        assert result == {"y": 10}

    def test_discover_from_module(self):
        """Test autodiscovery finds BaseTool subclasses in a module."""
        registry = ToolRegistry()
        # Discover from this test module — should find AlphaTool, BetaTool, AlphaToolV2
        registry.discover(__name__)
        # AlphaToolV2 has same tool_type as AlphaTool, so one will win
        # (first found wins without replace)
        assert len(registry.list_all()) >= 2

    def test_categories(self):
        registry = ToolRegistry()
        registry.register(AlphaTool)
        registry.register(BetaTool)
        assert registry.categories() == {"test"}
