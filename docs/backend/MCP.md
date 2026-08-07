# Model Context Protocol Integration

OpenMirai 0.7.0 implements an MCP 2024-11-05 JSON-RPC client with stdio and
HTTP transports and exposes it through the `mcp/call` built-in tool.

## Configuration

MCP servers belong under `config.mcp_servers`:

```yaml
name: mcp-example
config:
  mcp_servers:
    - name: local-tools
      transport: stdio
      command: node
      args: [./servers/tools.js]
    - name: remote-tools
      transport: http
      url: http://mcp.internal:8080
graph:
  nodes:
    - id: call
      tool_type: mcp/call
      config:
        server_name: local-tools
        tool_name: lookup
        arguments:
          key: example
  edges: []
```

The default host serializes this list into an internal `__mcp_servers` node
config value. Users should not set that internal key directly.

## Server fields

| Field | Meaning |
|---|---|
| `name` | Unique lookup name used by `mcp/call`. |
| `transport` | `stdio` by default; `http` is also implemented. |
| `command` | Required for stdio. Executed directly with `args`, not through a shell. |
| `args` | String argument list for the stdio process. |
| `url` | Required for HTTP. |
| `credential_ref` | Parsed declaration, but not resolved or applied by the default MCP manager. |

The HTTP transport supports custom headers at the library level, but
`MCPManager` currently constructs it without headers. Authenticated HTTP MCP
servers therefore require additional host wiring; `credential_ref` alone does
not send authorization.

## Call lifecycle

```text
mcp/call
 -> read injected server configs
 -> construct per-tool-call MCPManager
 -> find server by name
 -> lazily create transport
 -> initialize JSON-RPC handshake
 -> notifications/initialized
 -> tools/call(name, arguments)
 -> return raw result + joined text content
 -> close all clients / terminate stdio process
```

Although `MCPManager` can cache a client within its lifetime, `mcp/call`
creates a manager for one tool execution and calls `close_all()` afterward.
Connections and child processes are therefore not pooled across graph nodes or
runs by the default tool path.

## Tool contract

Required node config: `server_name`, `tool_name`. Arguments may arrive through
the `arguments` input or node config. Outputs include:

- `result`: raw MCP result object or null;
- `text`: newline-joined text content on success;
- `server_name` and `tool_name`;
- `success`: boolean;
- `error`: null or descriptive message.

Protocol/transport failures are returned as `success: false` tool output rather
than necessarily failing the graph node. Route explicitly on `success` if the
workflow must stop or compensate.

## Transport behavior

### Stdio

- spawn/connect timeout: 30 seconds by default;
- one JSON-RPC response line read timeout: 600 seconds;
- stderr handling belongs to the child transport/process environment;
- close waits up to five seconds, then kills the child;
- command and arguments inherit the OpenMirai process environment and OS
  permissions.

### HTTP

- sends JSON-RPC requests to the configured URL with Reqwest;
- connection is stateless on close;
- no default credential resolution or SSRF destination policy;
- use private DNS/network policy and TLS validation appropriate to the deployment.

## Security requirements

- trust and pin stdio executables/packages;
- use absolute executable paths where practical;
- do not pass secrets in arguments or AgentSpec;
- restrict environment variables inherited by child servers;
- allowlist HTTP schemes/hosts at the deployment boundary;
- prevent access to cloud metadata and internal control endpoints;
- run MCP servers inside the same or a stricter sandbox;
- treat returned content as prompt-injection-capable untrusted input;
- cap response size and execution resources at a surrounding worker boundary.

## Troubleshooting

| Symptom | Check |
|---|---|
| No servers configured | Ensure the list is under `config.mcp_servers`, not top level. |
| Server not found | `server_name` must exactly match the configuration name. |
| Stdio missing command | Set `command`; `url` is not used for stdio. |
| HTTP missing URL | Set `url`; `command` is not used for HTTP. |
| Handshake failure | Confirm server protocol/version, stdout framing, and that logs do not corrupt JSON-RPC stdout. |
| Remote 401 | Default manager does not apply `credential_ref` or custom headers. |
| Long hang | Account for the 600-second stdio read timeout and outer execution timeout differences. |

## Related documentation

- [AgentSpec](../AGENT_SPEC.md)
- [Built-in tools](BUILTIN_TOOLS.md)
- [Security](../../SECURITY.md)
- [Cloud deployment](../infra/CLOUD-DEPLOYMENT.md)
