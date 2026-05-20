"""MCP (Model Context Protocol) client for Data Mirai Engine.

Provides transport-agnostic connectivity to MCP servers and an adapter
that converts MCP tool definitions into Data Mirai ToolSpec format.
"""

from datamirai_engine.mcp.client import (
    HttpTransport,
    MCPClient,
    MCPClientManager,
    MCPError,
    MCPToolAdapter,
    MCPTransport,
    StdioTransport,
)

__all__ = [
    "HttpTransport",
    "MCPClient",
    "MCPClientManager",
    "MCPError",
    "MCPToolAdapter",
    "MCPTransport",
    "StdioTransport",
]
