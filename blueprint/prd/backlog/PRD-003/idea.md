# MCP Client Real — Conectar mcp/call con MCPClient existente

| Campo | Valor |
|-------|-------|
| **ID** | PRD-003 |
| **Fecha** | 2026-05-27 |
| **Estado** | backlog |
| **Branch** | prd/PRD-003 |
| **Target** | v0.2.0 (incluir en PRD-002) |

---

## Diagrama General

```
┌──────────────────────────────────────────────────────────────┐
│                     AGENT SPEC (YAML/JSON)                   │
│                                                              │
│  config:                                                     │
│    mcp_servers:                                              │
│      - name: "gaboos"                                        │
│        transport: "stdio"                                    │
│        command: "node"                                        │
│        args: ["mcp-server.js"]                               │
│                                                              │
│  graph:                                                      │
│    nodes:                                                    │
│      - id: create_task                                       │
│        tool_type: mcp/call                                   │
│        config:                                               │
│          server_name: "gaboos"                               │
│          tool_name: "create_task"                            │
└───────────────────────┬──────────────────────────────────────┘
                        │
                        ▼
┌──────────────────────────────────────────────────────────────┐
│                    ENGINE EXECUTION                           │
│                                                              │
│  1. GraphRunner inicia                                       │
│  2. Nodo mcp/call detecta server_name="gaboos"              │
│  3. MCP Manager busca/crea connection al server              │
│     ┌────────────────────────────────────────┐               │
│     │ MCPManager (lazy, por ejecución)       │               │
│     │                                        │               │
│     │ connections: HashMap<name, MCPClient>   │               │
│     │                                        │               │
│     │ get_or_create("gaboos") →              │               │
│     │   StdioTransport::new(["node","mcp"])  │               │
│     │   MCPClient::new(transport)            │               │
│     │   client.initialize()                  │               │
│     │   return &client                       │               │
│     └────────────────────────────────────────┘               │
│  4. client.call_tool("create_task", arguments)               │
│  5. JSON-RPC → stdin → server procesa → stdout → response    │
│  6. Parsear resultado → output del nodo                      │
│  7. Al terminar graph: MCPManager.close_all()                │
└──────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: feature (conectar dos partes existentes)
- **Resumen**: El tool `mcp/call` devuelve placeholder "MCP client not configured". El `MCPClient` con `StdioTransport` y `HttpTransport` ya existe y funciona (12 tests). Falta el puente: un `MCPManager` que crea/reutiliza connections por server_name, y actualizar `mcp/call` para usarlo.
- **Actores**: Engine (crate), MCP Server (externo), Agent (JSON config)
- **Flujos tocados**: mcp/call tool execution, agent config parsing, graph lifecycle (cleanup)
- **Qué cambia**:
  - HOY: mcp/call → `success: false, "MCP client not configured"`
  - DESPUÉS: mcp/call → spawn server → initialize → call_tool → resultado real

---

## Actores y Permisos

| Actor | Capacidad | Acción | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | configurar_mcp | Definir mcp_servers en AgentSpec | Config de servidores |
| Agent | ejecutar_mcp_tool | Invocar tools en MCP servers | Resultado del tool call |
| MCP Server | proveer_tools | Exponer tools via JSON-RPC | Tools disponibles |

---

## Entidades

### McpServerConfig (ya existe en AgentSpec, formalizar)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| name | texto | sí | Nombre único del server (referenciado por mcp/call) |
| transport | enum [stdio, http] | sí | Tipo de transporte |
| command | texto | sí (stdio) | Comando para spawn (ej: "node") |
| args | lista de texto | no | Argumentos del comando |
| url | texto | sí (http) | URL del server HTTP |
| env | json | no | Variables de ambiente para el proceso |
| timeout_secs | número | no | Timeout por tool call (default 600) |

### McpConnection (nueva, runtime-only)

| Campo | Tipo lógico | Requerido | Descripción |
|-------|-------------|-----------|-------------|
| server_name | texto | sí | Nombre del server |
| estado | enum [disconnected, connecting, connected, error] | sí | Estado de la conexión |
| client | referencia → MCPClient | sí | Client activo |
| available_tools | lista de json | no | Tools del server (post-initialize) |

---

## Ciclos de Vida

### McpConnection
Estados: DISCONNECTED | CONNECTING | CONNECTED | ERROR

| Desde | Hacia | Condición | Efecto |
|-------|-------|-----------|--------|
| DISCONNECTED | CONNECTING | Primer mcp/call para este server | Spawn process / connect HTTP |
| CONNECTING | CONNECTED | initialize() exitoso | list_tools(), guardar available_tools |
| CONNECTING | ERROR | Spawn falla o initialize falla | Log error |
| CONNECTED | ERROR | Tool call falla por transport error | Intentar reconectar 1 vez |
| CONNECTED | DISCONNECTED | Graph execution termina | close() + kill process |
| ERROR | CONNECTING | Retry automático (1 intento) | Re-spawn |

---

## Reglas de Negocio

### regla-mcp-lazy-connect
- **Invariante**: MCP servers se conectan lazy — solo al primer `mcp/call` que referencia ese server_name, no al cargar el agent spec.
- **Cuándo se verifica**: al ejecutar nodo mcp/call
- **Si se viola**: N/A (es optimización de recursos)

### regla-mcp-reuse-connection
- **Invariante**: Si un agente tiene 5 nodos `mcp/call` al mismo server, se reutiliza UNA sola conexión. No spawne 5 procesos.
- **Cuándo se verifica**: al ejecutar mcp/call
- **Si se viola**: Memory leak + process leak

### regla-mcp-cleanup
- **Invariante**: Al terminar la ejecución del graph (completed, failed, interrupted), TODOS los MCP servers spawneados se cierran (close + kill).
- **Cuándo se verifica**: al finalizar GraphRunner.run()
- **Si se viola**: Procesos zombie

### regla-mcp-server-not-found
- **Invariante**: Si mcp/call referencia un server_name que no existe en config.mcp_servers, error inmediato.
- **Cuándo se verifica**: al ejecutar mcp/call
- **Si se viola**: Error: "MCP server '{name}' not found in agent config"

---

## Patrones de Diseño

### Factory → MCPManager crea connections
- **Aplica a**: McpConnection lifecycle
- **Por qué**: Según transport type (stdio/http), crea diferente transporte
- **Participantes**: MCPManager (factory), StdioTransport/HttpTransport (productos), MCPClient (wrapper)

### Repository → MCPManager como cache de connections
- **Aplica a**: McpConnection reuse
- **Por qué**: get_or_create pattern — busca en cache, si no existe crea
- **Participantes**: MCPManager (repo), HashMap<String, MCPClient> (storage)

---

## Operaciones

### crear_mcp_manager
- **Actor**: Engine (interno, al iniciar graph execution)
- **Input**: lista de McpServerConfig del AgentSpec
- **Output exitoso**: MCPManager configurado (sin conexiones activas — lazy)

### obtener_connection
- **Actor**: Engine (desde mcp/call tool)
- **Input**: server_name (texto)
- **Output exitoso**: &MCPClient conectado e inicializado
- **Errores posibles**:
  - Server no definido en config → "MCP server '{name}' not found in agent config"
  - Spawn falla → "Failed to spawn MCP server '{name}': {error}"
  - Initialize falla → "MCP handshake failed for '{name}': {error}"

### ejecutar_mcp_call
- **Actor**: Agent (via mcp/call tool)
- **Input**: server_name, tool_name, arguments
- **Output exitoso**: `{result, server_name, tool_name, success: true}`
- **Errores posibles**:
  - Tool no existe en server → "Tool '{tool}' not found on MCP server '{server}'"
  - Server error → "MCP call failed: {jsonrpc_error}"
  - Timeout → "MCP call timed out after {n}s"

### cerrar_todas_connections
- **Actor**: Engine (al terminar graph execution)
- **Input**: MCPManager
- **Output exitoso**: Todos los procesos terminados limpiamente

---

## Interfaces

### CLI (sin cambios)
`mirai run agent.yaml` — si el agent tiene mcp_servers, se conecta automáticamente.

### Agent Spec formato (ya existe, sin cambios)
```yaml
config:
  mcp_servers:
    - name: "gaboos"
      transport: "stdio"
      command: "node"
      args: ["mcp-server.js"]
```

### Node formato (ya existe, sin cambios)
```yaml
- id: create_task
  tool_type: mcp/call
  config:
    server_name: "gaboos"
    tool_name: "create_task"
```

---

## Matriz de Permutaciones

| Flujo | Permutación | Actor | Resultado esperado |
|---|---|---|---|
| mcp/call | Happy: stdio server, tool existe | Agent | Resultado del tool call |
| mcp/call | Happy: http server, tool existe | Agent | Resultado del tool call |
| mcp/call | Server no definido en config | Agent | Error: server not found |
| mcp/call | Server falla al spawn | Agent | Error: failed to spawn |
| mcp/call | Server falla initialize | Agent | Error: handshake failed |
| mcp/call | Tool no existe en server | Agent | Error: tool not found |
| mcp/call | Tool call timeout | Agent | Error: timed out |
| mcp/call | Multiple calls same server | Agent | Reutiliza conexión |
| mcp/call | Server crash mid-execution | Agent | Error: transport error |
| cleanup | Graph completa | Engine | Todos los servers cerrados |
| cleanup | Graph falla | Engine | Todos los servers cerrados |

---

## Escenarios GWT

TEST-053: MCP call con stdio transport exitoso
  Given: Agent con mcp_server "test" (command: "node mock-mcp.js")
  When: Nodo mcp/call ejecuta tool "echo" con arguments {"msg": "hello"}
  Then: result contiene respuesta del server, success=true

TEST-054: MCP reusa conexión para múltiples calls
  Given: Agent con 2 nodos mcp/call al mismo server "test"
  When: Ambos nodos ejecutan
  Then: Solo 1 proceso spawneado (no 2)

TEST-055: MCP server no definido en config
  Given: Agent sin mcp_servers en config
  When: Nodo mcp/call referencia server "nonexistent"
  Then: Error: "MCP server 'nonexistent' not found in agent config"

TEST-056: MCP cleanup al terminar graph
  Given: Agent con mcp_server que fue conectado
  When: Graph execution termina (completed o failed)
  Then: Proceso del MCP server terminado (no zombie)

TEST-057: MCP call con http transport
  Given: Agent con mcp_server transport="http", url="http://localhost:8080"
  When: Nodo mcp/call ejecuta
  Then: HTTP POST al server, resultado parseado

---

## Fuera de Alcance

- MCP Server implementation (el ENGINE es client, no server)
- Tool discovery dinámico (por ahora, tool_name es explícito en config)
- MCP resources/prompts (solo tools por ahora)
- SSE transport (solo stdio y http)

---

## Dependencias

- `engine/src/mcp/client.rs` — ya implementado, 12 tests
- `engine/src/tools/builtin/mcp.rs` — placeholder a reemplazar
- `engine/src/core/agent_spec.rs` — ya tiene `mcp_servers` en config
- PRD-002 W1 (server real LLM) — deseable pero no blocker
