"""MCP client — JSON-RPC 2.0 transport + high-level operations.

Implements the Model Context Protocol (2024-11-05) using only stdlib + asyncio.
Two transports: StdioTransport (subprocess) and HttpTransport (HTTP POST).

Usage::

    async with MCPClient(StdioTransport(["npx", "-y", "@some/mcp-server"])) as client:
        await client.initialize()
        tools = await client.list_tools()
        result = await client.call_tool("tool_name", {"arg": "value"})

    # Convert MCP tools to Data Mirai ToolSpec
    specs = MCPToolAdapter.to_tool_specs(tools)
"""

from __future__ import annotations

import asyncio
import json
import logging
import urllib.error
import urllib.request
from abc import ABC, abstractmethod
from typing import Any

from datamirai_engine.tools.base import ConfigField, ToolInput, ToolOutput, ToolSpec

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MCP_PROTOCOL_VERSION = "2024-11-05"
_DEFAULT_TIMEOUT: float = 30.0
_DEFAULT_RECONNECT_ATTEMPTS: int = 3
_DEFAULT_RECONNECT_DELAY: float = 1.0

# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------


class MCPError(Exception):
    """Base error for all MCP operations."""


class MCPTransportError(MCPError):
    """Transport-level failure (process crash, network error)."""


class MCPProtocolError(MCPError):
    """Server returned a JSON-RPC error response."""

    def __init__(self, code: int, message: str, data: Any = None) -> None:
        self.code = code
        self.error_message = message
        self.data = data
        super().__init__(f"MCP error {code}: {message}")


class MCPTimeoutError(MCPError):
    """Request did not complete within the configured timeout."""


# ---------------------------------------------------------------------------
# Transport ABC
# ---------------------------------------------------------------------------


class MCPTransport(ABC):
    """Abstract transport for JSON-RPC 2.0 communication with an MCP server."""

    @abstractmethod
    async def connect(self) -> None:
        """Establish the connection to the MCP server."""

    @abstractmethod
    async def send_request(
        self, method: str, params: dict[str, Any] | None = None
    ) -> Any:
        """Send a JSON-RPC request and return the parsed result."""

    @abstractmethod
    async def send_notification(
        self, method: str, params: dict[str, Any] | None = None
    ) -> None:
        """Send a JSON-RPC notification (no response expected)."""

    @abstractmethod
    async def close(self) -> None:
        """Tear down the connection and release resources."""

    @abstractmethod
    def is_connected(self) -> bool:
        """Return True if the transport is currently connected."""


# ---------------------------------------------------------------------------
# JSON-RPC helpers
# ---------------------------------------------------------------------------


def _build_request(
    method: str, params: dict[str, Any] | None, request_id: int
) -> bytes:
    """Serialize a JSON-RPC 2.0 request to bytes."""
    payload: dict[str, Any] = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
    }
    if params is not None:
        payload["params"] = params
    return json.dumps(payload).encode("utf-8")


def _build_notification(
    method: str, params: dict[str, Any] | None
) -> bytes:
    """Serialize a JSON-RPC 2.0 notification (no id) to bytes."""
    payload: dict[str, Any] = {
        "jsonrpc": "2.0",
        "method": method,
    }
    if params is not None:
        payload["params"] = params
    return json.dumps(payload).encode("utf-8")


def _parse_response(raw: bytes | str, expected_id: int) -> Any:
    """Parse and validate a JSON-RPC 2.0 response, return the result."""
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise MCPTransportError(f"Invalid JSON from server: {exc}") from exc

    if data.get("id") != expected_id:
        raise MCPProtocolError(
            -32600,
            f"Response id mismatch: expected {expected_id}, got {data.get('id')}",
        )

    if "error" in data:
        err = data["error"]
        raise MCPProtocolError(
            code=err.get("code", -1),
            message=err.get("message", "Unknown error"),
            data=err.get("data"),
        )

    return data.get("result")


# ---------------------------------------------------------------------------
# StdioTransport
# ---------------------------------------------------------------------------


class StdioTransport(MCPTransport):
    """Launch MCP server as a subprocess; communicate via stdin/stdout.

    Each JSON-RPC message is a single line terminated by ``\\n``.
    """

    def __init__(
        self,
        command: list[str],
        *,
        env: dict[str, str] | None = None,
        timeout: float = _DEFAULT_TIMEOUT,
    ) -> None:
        if not command:
            raise ValueError("command must be a non-empty list")
        self._command = command
        self._env = env
        self._timeout = timeout
        self._process: asyncio.subprocess.Process | None = None
        self._request_id = 0
        self._lock = asyncio.Lock()

    # -- lifecycle -----------------------------------------------------------

    async def connect(self) -> None:
        if self._process is not None and self._process.returncode is None:
            logger.debug("StdioTransport already connected")
            return

        logger.info("Starting MCP subprocess: %s", " ".join(self._command))
        self._process = await asyncio.create_subprocess_exec(
            *self._command,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=self._env,
        )
        logger.info("MCP subprocess started (pid=%s)", self._process.pid)

    async def close(self) -> None:
        proc = self._process
        if proc is None:
            return

        logger.info("Closing MCP subprocess (pid=%s)", proc.pid)

        if proc.stdin and not proc.stdin.is_closing():
            proc.stdin.close()
            try:
                await proc.stdin.wait_closed()
            except Exception:  # noqa: BLE001
                pass

        # Give the process a short window to exit cleanly before killing.
        try:
            await asyncio.wait_for(proc.wait(), timeout=5.0)
        except asyncio.TimeoutError:
            logger.warning("MCP subprocess did not exit; killing (pid=%s)", proc.pid)
            proc.kill()
            await proc.wait()

        self._process = None

    def is_connected(self) -> bool:
        return self._process is not None and self._process.returncode is None

    # -- messaging -----------------------------------------------------------

    async def send_request(
        self, method: str, params: dict[str, Any] | None = None
    ) -> Any:
        proc = self._process
        if proc is None or proc.returncode is not None:
            raise MCPTransportError("Subprocess is not running")

        async with self._lock:
            self._request_id += 1
            req_id = self._request_id

            payload = _build_request(method, params, req_id)
            logger.debug("stdio >>> %s", payload.decode())

            assert proc.stdin is not None
            assert proc.stdout is not None

            proc.stdin.write(payload + b"\n")
            await proc.stdin.drain()

            try:
                raw_line = await asyncio.wait_for(
                    proc.stdout.readline(), timeout=self._timeout
                )
            except asyncio.TimeoutError:
                raise MCPTimeoutError(
                    f"Timed out waiting for response to {method} (id={req_id})"
                )

            if not raw_line:
                stderr_tail = ""
                if proc.stderr:
                    try:
                        stderr_data = await asyncio.wait_for(
                            proc.stderr.read(4096), timeout=1.0
                        )
                        stderr_tail = stderr_data.decode(errors="replace")
                    except asyncio.TimeoutError:
                        pass
                raise MCPTransportError(
                    f"Subprocess closed stdout unexpectedly. stderr: {stderr_tail}"
                )

            logger.debug("stdio <<< %s", raw_line.decode().rstrip())
            return _parse_response(raw_line, req_id)

    async def send_notification(
        self, method: str, params: dict[str, Any] | None = None
    ) -> None:
        proc = self._process
        if proc is None or proc.returncode is not None:
            raise MCPTransportError("Subprocess is not running")

        async with self._lock:
            payload = _build_notification(method, params)
            logger.debug("stdio >>> (notification) %s", payload.decode())

            assert proc.stdin is not None
            proc.stdin.write(payload + b"\n")
            await proc.stdin.drain()


# ---------------------------------------------------------------------------
# HttpTransport
# ---------------------------------------------------------------------------


class HttpTransport(MCPTransport):
    """Connect to an MCP server over HTTP POST (JSON-RPC over HTTP).

    Uses ``urllib.request`` from stdlib so no external HTTP library is needed.
    All network I/O is offloaded to a thread via ``asyncio.to_thread``.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        timeout: float = _DEFAULT_TIMEOUT,
    ) -> None:
        if not url:
            raise ValueError("url must not be empty")
        self._url = url
        self._headers = headers or {}
        self._timeout = timeout
        self._request_id = 0
        self._connected = False
        self._lock = asyncio.Lock()

    # -- lifecycle -----------------------------------------------------------

    async def connect(self) -> None:
        """Verify that the server endpoint is reachable."""
        logger.info("Verifying MCP HTTP server at %s", self._url)
        try:
            # Simple connectivity check — send an empty OPTIONS-like GET.
            # Some MCP servers respond 405 which is fine; we just want TCP ok.
            req = urllib.request.Request(self._url, method="GET")
            for k, v in self._headers.items():
                req.add_header(k, v)
            await asyncio.to_thread(
                urllib.request.urlopen, req, timeout=self._timeout
            )
        except urllib.error.HTTPError:
            # HTTP errors mean the server is alive.
            pass
        except urllib.error.URLError as exc:
            raise MCPTransportError(
                f"Cannot reach MCP server at {self._url}: {exc.reason}"
            ) from exc

        self._connected = True
        logger.info("MCP HTTP server reachable")

    async def close(self) -> None:
        self._connected = False

    def is_connected(self) -> bool:
        return self._connected

    # -- messaging -----------------------------------------------------------

    async def send_request(
        self, method: str, params: dict[str, Any] | None = None
    ) -> Any:
        if not self._connected:
            raise MCPTransportError("HttpTransport is not connected")

        async with self._lock:
            self._request_id += 1
            req_id = self._request_id

        payload = _build_request(method, params, req_id)
        logger.debug("http >>> %s", payload.decode())

        req = urllib.request.Request(
            self._url,
            data=payload,
            method="POST",
        )
        req.add_header("Content-Type", "application/json")
        for k, v in self._headers.items():
            req.add_header(k, v)

        try:
            resp = await asyncio.to_thread(
                urllib.request.urlopen, req, timeout=self._timeout
            )
            raw = resp.read()
        except urllib.error.HTTPError as exc:
            body = exc.read().decode(errors="replace")
            raise MCPTransportError(
                f"HTTP {exc.code} from MCP server: {body}"
            ) from exc
        except urllib.error.URLError as exc:
            raise MCPTransportError(
                f"Network error reaching MCP server: {exc.reason}"
            ) from exc
        except TimeoutError:
            raise MCPTimeoutError(
                f"HTTP request timed out after {self._timeout}s for {method}"
            )

        logger.debug("http <<< %s", raw.decode().rstrip())
        return _parse_response(raw, req_id)

    async def send_notification(
        self, method: str, params: dict[str, Any] | None = None
    ) -> None:
        if not self._connected:
            raise MCPTransportError("HttpTransport is not connected")

        payload = _build_notification(method, params)
        logger.debug("http >>> (notification) %s", payload.decode())

        req = urllib.request.Request(
            self._url,
            data=payload,
            method="POST",
        )
        req.add_header("Content-Type", "application/json")
        for k, v in self._headers.items():
            req.add_header(k, v)

        try:
            await asyncio.to_thread(
                urllib.request.urlopen, req, timeout=self._timeout
            )
        except Exception:  # noqa: BLE001
            # Notifications are fire-and-forget — log but don't raise.
            logger.warning("Failed to send notification %s", method, exc_info=True)


# ---------------------------------------------------------------------------
# MCPClient
# ---------------------------------------------------------------------------


class MCPClient:
    """High-level async client for Model Context Protocol servers.

    Wraps a transport and provides typed helpers for the standard MCP
    operations: ``initialize``, ``list_tools``, ``call_tool``.

    Supports reconnection on transport failure (configurable).

    Can be used as an async context manager::

        async with MCPClient(transport) as client:
            await client.initialize()
            ...
    """

    def __init__(
        self,
        transport: MCPTransport,
        *,
        reconnect_attempts: int = _DEFAULT_RECONNECT_ATTEMPTS,
        reconnect_delay: float = _DEFAULT_RECONNECT_DELAY,
    ) -> None:
        self._transport = transport
        self._reconnect_attempts = reconnect_attempts
        self._reconnect_delay = reconnect_delay
        self._initialized = False
        self._server_capabilities: dict[str, Any] = {}
        self._server_info: dict[str, Any] = {}

    # -- context manager -----------------------------------------------------

    async def __aenter__(self) -> MCPClient:
        await self.connect()
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.close()

    # -- lifecycle -----------------------------------------------------------

    async def connect(self) -> None:
        """Connect the underlying transport."""
        await self._transport.connect()

    async def close(self) -> None:
        """Close the transport and release resources."""
        self._initialized = False
        await self._transport.close()

    @property
    def is_connected(self) -> bool:
        return self._transport.is_connected()

    @property
    def is_initialized(self) -> bool:
        return self._initialized

    @property
    def server_capabilities(self) -> dict[str, Any]:
        return dict(self._server_capabilities)

    @property
    def server_info(self) -> dict[str, Any]:
        return dict(self._server_info)

    # -- reconnection --------------------------------------------------------

    async def _ensure_connected(self) -> None:
        """Reconnect if the transport dropped. Raises after exhausting retries."""
        if self._transport.is_connected():
            return

        last_error: Exception | None = None
        for attempt in range(1, self._reconnect_attempts + 1):
            logger.warning(
                "Transport disconnected — reconnect attempt %d/%d",
                attempt,
                self._reconnect_attempts,
            )
            try:
                await self._transport.close()
                await self._transport.connect()

                # Re-initialize after reconnect so the server knows us again.
                if self._initialized:
                    await self._send_initialize()

                logger.info("Reconnected on attempt %d", attempt)
                return
            except Exception as exc:  # noqa: BLE001
                last_error = exc
                if attempt < self._reconnect_attempts:
                    await asyncio.sleep(self._reconnect_delay * attempt)

        raise MCPTransportError(
            f"Failed to reconnect after {self._reconnect_attempts} attempts"
        ) from last_error

    # -- MCP operations ------------------------------------------------------

    async def initialize(self) -> dict[str, Any]:
        """Send the MCP ``initialize`` handshake.

        Returns the full result dict from the server.
        """
        await self._ensure_connected()
        result = await self._send_initialize()
        self._initialized = True
        return result

    async def _send_initialize(self) -> dict[str, Any]:
        result = await self._transport.send_request(
            "initialize",
            {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "datamirai-engine",
                    "version": "0.1.0",
                },
            },
        )

        self._server_capabilities = result.get("capabilities", {})
        self._server_info = result.get("serverInfo", {})

        logger.info(
            "MCP initialized — server=%s, protocol=%s",
            self._server_info.get("name", "unknown"),
            result.get("protocolVersion", "unknown"),
        )

        # Per spec, client MUST send initialized notification after init.
        await self._transport.send_notification("notifications/initialized")
        return result

    async def list_tools(self) -> list[dict[str, Any]]:
        """Request the list of tools from the MCP server.

        Returns a list of raw MCP tool definitions, each with keys
        ``name``, ``description``, and ``inputSchema``.
        """
        if not self._initialized:
            raise MCPError("Client not initialized — call initialize() first")

        await self._ensure_connected()
        result = await self._transport.send_request("tools/list")
        tools: list[dict[str, Any]] = result.get("tools", [])
        logger.info("MCP server exposes %d tool(s)", len(tools))
        return tools

    async def call_tool(
        self, name: str, arguments: dict[str, Any] | None = None
    ) -> list[dict[str, Any]]:
        """Execute a tool on the MCP server.

        Parameters
        ----------
        name:
            Tool name as returned by ``list_tools``.
        arguments:
            Arguments to pass to the tool (matching its ``inputSchema``).

        Returns
        -------
        list[dict]:
            Content blocks from the response. Each block has at minimum
            ``type`` (e.g. ``"text"``) and a corresponding value key.
        """
        if not self._initialized:
            raise MCPError("Client not initialized — call initialize() first")

        await self._ensure_connected()
        result = await self._transport.send_request(
            "tools/call",
            {
                "name": name,
                "arguments": arguments or {},
            },
        )

        content: list[dict[str, Any]] = result.get("content", [])
        is_error = result.get("isError", False)
        if is_error:
            text_parts = [
                c.get("text", "") for c in content if c.get("type") == "text"
            ]
            raise MCPError(
                f"Tool '{name}' returned an error: {' '.join(text_parts)}"
            )

        return content


# ---------------------------------------------------------------------------
# MCPClientManager — manages multiple named MCP server connections
# ---------------------------------------------------------------------------


class MCPClientManager:
    """Manages connections to multiple MCP servers simultaneously.

    Used as ``context.mcp_client`` during graph execution so that the
    ``mcp/call`` tool can route requests to the correct server by name.

    Usage::

        manager = MCPClientManager()
        await manager.connect("my-server", command=["python3", "server.py"])
        tools = await manager.list_tools("my-server")
        result = await manager.call_tool("my-server", "echo", {"message": "hi"})
        await manager.disconnect("my-server")
    """

    def __init__(self) -> None:
        self._clients: dict[str, MCPClient] = {}

    async def connect(
        self,
        server_name: str,
        *,
        command: list[str] | None = None,
        args: list[str] | None = None,
        env: dict[str, str] | None = None,
        url: str | None = None,
        headers: dict[str, str] | None = None,
        timeout: float = _DEFAULT_TIMEOUT,
    ) -> dict[str, Any]:
        """Connect to an MCP server. Returns server info after handshake.

        For stdio: provide ``command`` (and optionally ``args``).
        For HTTP: provide ``url``.
        """
        if server_name in self._clients and self._clients[server_name].is_connected:
            logger.info("Server '%s' already connected — reusing", server_name)
            return self._clients[server_name].server_info

        # Build transport
        if command:
            full_command = [command] if isinstance(command, str) else list(command)
            if args:
                full_command.extend(args)
            transport: MCPTransport = StdioTransport(full_command, env=env, timeout=timeout)
        elif url:
            transport = HttpTransport(url, headers=headers, timeout=timeout)
        else:
            raise MCPError("Either command or url must be provided")

        client = MCPClient(transport=transport)
        await client.connect()
        await client.initialize()

        self._clients[server_name] = client
        logger.info("MCPClientManager: connected to '%s'", server_name)
        return client.server_info

    async def disconnect(self, server_name: str) -> None:
        """Disconnect from an MCP server."""
        client = self._clients.pop(server_name, None)
        if client:
            await client.close()
            logger.info("MCPClientManager: disconnected '%s'", server_name)

    async def disconnect_all(self) -> None:
        """Disconnect from all MCP servers."""
        for name in list(self._clients):
            await self.disconnect(name)

    def is_connected(self, server_name: str) -> bool:
        """Check if a server is connected."""
        client = self._clients.get(server_name)
        return client is not None and client.is_connected

    async def list_tools(self, server_name: str) -> list[dict[str, Any]]:
        """List tools available on a connected server."""
        client = self._clients.get(server_name)
        if not client:
            raise MCPError(f"Server '{server_name}' is not connected")
        return await client.list_tools()

    async def call_tool(
        self, server_name: str, tool_name: str, arguments: dict[str, Any] | None = None
    ) -> list[dict[str, Any]]:
        """Execute a tool on a connected MCP server."""
        client = self._clients.get(server_name)
        if not client:
            raise MCPError(f"Server '{server_name}' is not connected")
        return await client.call_tool(tool_name, arguments)

    def list_servers(self) -> list[dict[str, Any]]:
        """Return status of all managed servers."""
        return [
            {
                "name": name,
                "connected": client.is_connected,
                "initialized": client.is_initialized,
                "server_info": client.server_info,
            }
            for name, client in self._clients.items()
        ]


# ---------------------------------------------------------------------------
# MCPToolAdapter — MCP tool defs -> Data Mirai ToolSpec
# ---------------------------------------------------------------------------

# Mapping from JSON Schema types to ToolInput type strings.
_JSON_SCHEMA_TYPE_MAP: dict[str, str] = {
    "string": "string",
    "number": "number",
    "integer": "number",
    "boolean": "boolean",
    "object": "object",
    "array": "array",
}


class MCPToolAdapter:
    """Convert MCP tool definitions to Data Mirai ``ToolSpec`` instances.

    This is a stateless utility class — all methods are classmethods.
    """

    @classmethod
    def to_tool_specs(cls, mcp_tools: list[dict[str, Any]]) -> list[ToolSpec]:
        """Convert a list of raw MCP tool definitions to ``ToolSpec`` list.

        Parameters
        ----------
        mcp_tools:
            List of dicts as returned by ``MCPClient.list_tools()``.

        Returns
        -------
        list[ToolSpec]:
            One ``ToolSpec`` per MCP tool.
        """
        return [cls._convert_tool(t) for t in mcp_tools]

    @classmethod
    def _convert_tool(cls, mcp_tool: dict[str, Any]) -> ToolSpec:
        name: str = mcp_tool.get("name", "unknown")
        description: str = mcp_tool.get("description", "")
        input_schema: dict[str, Any] = mcp_tool.get("inputSchema", {})

        inputs = cls._extract_inputs(input_schema)

        return ToolSpec(
            tool_type=f"mcp/{name}",
            version="0.1.0",
            display_name=name,
            description=description,
            category="mcp",
            inputs=inputs,
            outputs=[
                ToolOutput(
                    name="content",
                    type="array",
                    description="MCP content blocks returned by the tool",
                ),
            ],
            config=[
                ConfigField(
                    name="mcp_tool_name",
                    type="string",
                    default=name,
                    description="Original MCP tool name (read-only)",
                ),
            ],
        )

    @classmethod
    def _extract_inputs(cls, schema: dict[str, Any]) -> list[ToolInput]:
        """Parse a JSON Schema ``properties`` object into ``ToolInput`` list."""
        properties: dict[str, Any] = schema.get("properties", {})
        required_set: set[str] = set(schema.get("required", []))

        inputs: list[ToolInput] = []
        for prop_name, prop_def in properties.items():
            raw_type = prop_def.get("type", "any")
            # Handle union types (e.g. ["string", "null"])
            if isinstance(raw_type, list):
                non_null = [t for t in raw_type if t != "null"]
                raw_type = non_null[0] if non_null else "any"

            mapped_type = _JSON_SCHEMA_TYPE_MAP.get(raw_type, "any")
            inputs.append(
                ToolInput(
                    name=prop_name,
                    type=mapped_type,
                    required=prop_name in required_set,
                    default=prop_def.get("default"),
                    description=prop_def.get("description", ""),
                )
            )

        return inputs
