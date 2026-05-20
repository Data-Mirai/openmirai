# FEAT-007 — MCP Client + Server

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-070

---

## Problem Statement

**Tipo**: Feature nueva (2 capacidades de integracion bidireccional)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine ejecuta grafos agentivos internamente pero esta aislado del ecosistema de herramientas externas en dos direcciones:

1. **Sin acceso a tools externas**: Existen 200+ MCP servers publicos (GitHub, Slack, Notion, Google Drive, Brave Search, filesystem, databases) que exponen tools estandarizadas. Hoy el usuario que necesita interactuar con GitHub desde un agente tiene que construir un tool custom con web_scrape o HTTP manual. Es ineficiente, fragil y reinventa lo que ya existe.

2. **Agentes encerrados en la UI**: Los agentes de Data Mirai solo se pueden invocar desde la UI web local. Claude Code, Cursor, OpenClaw, Windsurf y cualquier app MCP-compatible no pueden ejecutar agentes de Data Mirai como tools. El valor de los agentes queda atrapado dentro de la aplicacion — no hay interoperabilidad.

MCP (Model Context Protocol) de Anthropic es el estandar emergente para interoperabilidad de tools entre AI systems. Implementar ambos lados (client + server) posiciona a Data Mirai como ciudadano de primera clase en el ecosistema AI.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Conectar MCP servers externos (GitHub, Slack, filesystem, etc.) y usar sus tools dentro de grafos de Data Mirai como cualquier otro nodo
2. Descubrir tools disponibles de cada MCP server conectado y verlas en el catalogo del editor visual
3. Ejecutar tools MCP desde nodos del grafo con inputs/outputs mapeados via data_map
4. Exponer sus agentes de Data Mirai como tools MCP para que Claude Code, Cursor u otros clientes MCP los invoquen
5. Gestionar API keys para acceso externo a sus agentes via MCP
6. Copiar configuracion MCP lista para pegar en Claude Code o Cursor

---

## Features

### 7.1 — MCP Client: Consumir Tools de MCP Servers Externos

**Problema**: El usuario quiere que su agente cree un issue en GitHub, envie un mensaje en Slack, o lea un archivo de Google Drive. Hoy tiene que usar web_scrape con APIs REST manuales: construir URLs, manejar auth, parsear responses. Hay 200+ MCP servers que ya exponen estas capacidades como tools estandarizadas con schemas tipados, pero Data Mirai no puede consumirlas.

**Solucion**: Implementar un MCP client que se conecta a MCP servers configurados, descubre sus tools via el protocolo estandar, y las expone como nodos disponibles en el editor visual. Un nuevo tool `mcp/tool_call` ejecuta cualquier tool de un server conectado.

**Arquitectura**:
- `MCPClientManager`: singleton que gestiona conexiones a MCP servers
  - `connect(server_config) -> MCPConnection` — establece conexion stdio o SSE
  - `disconnect(server_id)` — cierra conexion limpiamente
  - `list_connections() -> list[MCPConnection]` — conexiones activas
  - `get_connection(server_id) -> MCPConnection` — conexion especifica
- `MCPConnection`: wrapper sobre una conexion MCP activa
  - `discover_tools() -> list[MCPToolDef]` — listar tools disponibles del server
  - `call_tool(tool_name, arguments) -> MCPToolResult` — ejecutar tool
  - `get_status() -> ConnectionStatus` — estado de la conexion (connected/error/disconnected)
  - `ping() -> bool` — health check
- `MCPToolDef`: `{ name: str, description: str, input_schema: dict, server_id: str, server_name: str }`
- `MCPToolResult`: `{ content: list[dict], is_error: bool, error_message: str | None }`
- `ConnectionStatus`: enum (connected, disconnected, error, connecting)
- Transports soportados:
  - `StdioTransport` — lanza proceso local (command + args), comunica via stdin/stdout. Para MCP servers locales (filesystem, sqlite, git).
  - `SSETransport` — conecta via HTTP SSE a URL remota. Para MCP servers remotos.
- Tool `mcp/tool_call` (nuevo tool builtin):
  - Inputs: `server_id` (referencia al server conectado), `tool_name` (nombre del tool en el server), `arguments` (dict con argumentos segun schema del tool)
  - Outputs: `result` (contenido retornado por el tool), `is_error` (bool), `error_message` (str | None)
  - En el editor visual: el usuario selecciona server -> tool -> mapea arguments via data_map
- Discovery cache: al conectar un server, sus tools se cachean en memoria. Refresh manual o automatico cada 5 minutos. El cache se invalida al reconectar.

**Entidades nuevas**:

Tabla `mcp_server_connection` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | NULL |
| transport | TEXT | NOT NULL (stdio / sse) |
| command | TEXT | NULL (para stdio: comando a ejecutar, ej: "npx @modelcontextprotocol/server-github") |
| args | TEXT | JSON array, NULL (argumentos del comando stdio) |
| url | TEXT | NULL (para SSE: URL del server remoto) |
| env_vars | TEXT | JSON object, NULL (variables de entorno para stdio, ej: {"GITHUB_TOKEN": "vault:xxx"}) |
| headers | TEXT | JSON object, NULL (headers para SSE, ej: auth headers) |
| auto_connect | BOOLEAN | DEFAULT true (conectar automaticamente al arrancar server) |
| status | TEXT | DEFAULT 'disconnected' (connected/disconnected/error) |
| last_error | TEXT | NULL |
| tools_cache | TEXT | JSON array, NULL (cache de tools descubiertas) |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

**Contratos API**:
- `GET /api/mcp/servers` — listar servers configurados con status actual. Response: `{ servers: MCPServerConnection[] }`
- `POST /api/mcp/servers` — crear/configurar server. Body: `{ name, description?, transport, command?, args?, url?, env_vars?, headers?, auto_connect? }`. Response: `{ server: MCPServerConnection }`
- `GET /api/mcp/servers/{id}` — detalle de server. Response: `{ server: MCPServerConnection }`
- `PATCH /api/mcp/servers/{id}` — actualizar config. Body: campos parciales. Response: `{ server: MCPServerConnection }`
- `DELETE /api/mcp/servers/{id}` — eliminar server (desconecta primero si activo). Response: `204`
- `POST /api/mcp/servers/{id}/connect` — conectar al server. Response: `{ status: "connected", tools_count: int }`
- `POST /api/mcp/servers/{id}/disconnect` — desconectar. Response: `{ status: "disconnected" }`
- `POST /api/mcp/servers/{id}/test` — test de conexion (conecta, descubre tools, desconecta). Response: `{ success: bool, tools_count: int, latency_ms: int, error?: str }`
- `GET /api/mcp/servers/{id}/tools` — listar tools del server. Response: `{ tools: MCPToolDef[] }`
- `GET /api/mcp/tools` — listar TODAS las tools de TODOS los servers conectados (para catalogo). Response: `{ tools: MCPToolDef[] }`

**Pantallas**:
- **Settings → seccion "MCP Servers"** (`/settings/mcp`):
  - Lista de servers configurados con indicador de estado (connected verde / disconnected gris / error rojo) y conteo de tools
  - Formulario para agregar server: nombre, tipo de transporte (stdio/SSE), campos dinamicos segun tipo (command+args para stdio, URL+headers para SSE), variables de entorno con soporte vault
  - Botones: Test Connection, Connect/Disconnect, Edit, Delete
  - Vista expandida de server muestra las tools descubiertas con nombre y descripcion
- **EditorCanvas → ToolCatalog**: nueva seccion "MCP Tools" en el catalogo. Agrupa tools por server. Al arrastrar una MCP tool al canvas, crea nodo `mcp/tool_call` pre-configurado con server_id y tool_name. NodeConfigPanel muestra los arguments segun el input_schema del tool.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-107 | Conexiones MCP se cierran limpiamente al apagar el server. Ningun proceso hijo stdio queda huerfano | MCPClientManager.shutdown() en app lifecycle |
| REGLA-108 | env_vars con prefijo `vault:` se resuelven contra el vault de credenciales (FEAT-001). Nunca se almacenan secrets en plaintext en mcp_server_connection | MCPClientManager al conectar |
| REGLA-109 | Si un MCP server se desconecta durante ejecucion de un grafo, el nodo mcp/tool_call falla con error descriptivo (server_name, tool_name, motivo). No retry automatico | mcp/tool_call error handling |
| REGLA-110 | Discovery cache se invalida al reconectar. Tools removidas del server se remueven del cache. Nodos que referencian tools eliminadas fallan con error descriptivo al ejecutar | MCPConnection.discover_tools |
| REGLA-111 | Max 20 MCP servers configurados simultaneamente. Limite practico para evitar leak de procesos stdio | POST /api/mcp/servers validation |
| REGLA-112 | Timeout de 30s para conexion inicial. Timeout de 60s para tool execution. Ambos configurables por server | MCPConnection config |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/mcp/client.py` — MCPClientManager + MCPConnection + MCPToolDef + MCPToolResult
- CREAR `framework/src/datamirai_engine/mcp/transports.py` — StdioTransport + SSETransport
- CREAR `framework/src/datamirai_engine/tools/builtin/mcp/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/tools/builtin/mcp/tool_call.py` — tool mcp/tool_call
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/__init__.py` — registrar categoria mcp
- MODIFICAR `app/server/datamirai_app/database.py` — tabla mcp_server_connection + migracion
- CREAR `app/server/datamirai_app/routes/mcp_servers.py` — endpoints CRUD + connect/disconnect + test + tools
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de mcp_servers
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints MCP
- CREAR `app/web/src/app/settings/mcp/page.tsx` — pagina UI de gestion de MCP servers
- MODIFICAR `app/web/src/components/editor/ToolCatalog.tsx` — seccion MCP Tools con tools de servers conectados
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — config panel para nodos mcp/tool_call con arguments dinamicos

---

### 7.2 — MCP Server: Exponer Agentes como Tools MCP

**Problema**: El usuario construyo agentes utiles en Data Mirai (scraper de noticias, analizador de documentos, generador de reportes) pero solo puede invocarlos desde la UI web. Claude Code no puede decir "ejecuta mi agente de scraping" porque no hay forma de exponer agentes como tools MCP. El valor queda encerrado.

**Solucion**: Implementar un MCP server que expone cada agente habilitado como un tool MCP estandar. Claude Code, Cursor y cualquier cliente MCP pueden descubrir y ejecutar agentes de Data Mirai. Soportar transporte stdio (para integracion local con Claude Code/Cursor) y SSE (para acceso remoto). Auth via API keys.

**Arquitectura**:
- `Data MiraiMCPServer`: implementa el protocolo MCP server usando `mcp` SDK de Anthropic
  - Tools expuestas dinamicamente:
    - `datamirai_list_agents` — retorna lista de agentes habilitados con descripcion
    - `datamirai_execute_{agent_slug}` — una tool por agente habilitado. Input: argumentos del trigger del agente. Output: resultado de la sesion
    - `datamirai_get_session_status` — consultar estado de una sesion en progreso (para ejecuciones largas)
  - El server consulta la DB para listar agentes con status `enabled`
  - Al ejecutar: crea session, ejecuta grafo, retorna resultado
- `MCPServerManager`: gestiona el lifecycle del MCP server
  - `start_stdio()` — inicia server en modo stdio (lee stdin, escribe stdout)
  - `start_sse(host, port)` — inicia server HTTP con SSE
  - `stop()` — shutdown graceful
- `APIKeyManager`: gestion de API keys para acceso externo
  - Keys se almacenan hasheadas (bcrypt). Plaintext solo se retorna en creacion.
  - Cada request SSE valida API key en header `Authorization: Bearer {key}`
  - Requests stdio no requieren auth (confianza local implicita)
- Config snippet generator: produce JSON listo para copiar en:
  - Claude Code (`~/.claude/mcp.json` format)
  - Cursor (`.cursor/mcp.json` format)
  - Formato generico MCP

**Entidades nuevas**:

Tabla `mcp_api_key` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL (nombre descriptivo: "Claude Code local", "Cursor trabajo") |
| key_hash | TEXT | NOT NULL (bcrypt hash de la API key) |
| key_prefix | TEXT | NOT NULL (primeros 8 chars para identificacion visual: "tuc_abc1...") |
| is_active | BOOLEAN | DEFAULT true |
| last_used_at | TEXT | NULL |
| expires_at | TEXT | NULL (null = no expira) |
| created_at | TEXT | NOT NULL |

**Contratos API**:
- `GET /api/mcp/server/status` — estado del MCP server (running/stopped, transport, port). Response: `{ running: bool, transport: str | null, port: int | null, tools_count: int }`
- `POST /api/mcp/server/start` — iniciar MCP server SSE. Body: `{ port?: int }`. Response: `{ status: "running", port: int }`
- `POST /api/mcp/server/stop` — detener MCP server. Response: `{ status: "stopped" }`
- `GET /api/mcp/server/config` — generar config snippet. Query params: `format` (claude_code / cursor / generic). Response: `{ config: object, format: str, instructions: str }`
- `GET /api/mcp/api-keys` — listar API keys (sin hash, solo prefix + metadata). Response: `{ keys: MCPApiKey[] }`
- `POST /api/mcp/api-keys` — crear API key. Body: `{ name, expires_at? }`. Response: `{ key: MCPApiKey, plaintext_key: str }` (unica vez que se retorna plaintext)
- `DELETE /api/mcp/api-keys/{id}` — revocar API key. Response: `204`

**Pantallas**:
- **Settings → seccion "MCP Server"** (`/settings/mcp` tab server):
  - Status del server (running/stopped) con toggle para iniciar/detener
  - Puerto configurable para modo SSE
  - Lista de agentes expuestos como tools (con toggle por agente para habilitar/deshabilitar exposicion)
  - Seccion "API Keys": lista de keys activas (nombre, prefix, last_used, created), boton crear nueva key (muestra plaintext una sola vez con boton copiar), boton revocar
  - Seccion "Configuracion": tabs con snippets para Claude Code, Cursor, generico. Boton copiar al clipboard. Instrucciones paso a paso.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-113 | Solo agentes con status `enabled` se exponen como tools MCP. Agentes disabled/draft no son visibles para clientes MCP | MCP server tool list filter |
| REGLA-114 | API key plaintext solo se retorna una vez (en POST response). Despues solo se almacena el hash. Si el usuario pierde la key, debe crear una nueva | Service |
| REGLA-115 | API keys revocadas rechazan requests inmediatamente (401). No se eliminan fisicamente para auditoria | Auth middleware |
| REGLA-116 | En transporte stdio, no se requiere API key (confianza local implicita). En transporte SSE, API key es obligatoria | MCP server auth layer |
| REGLA-117 | Ejecucion de agente via MCP es sincrona por default (el tool bloquea hasta que la sesion termine). Timeout configurable (default: 300s). Si excede timeout, retorna session_id para polling via `datamirai_get_session_status` | MCP tool handler |
| REGLA-118 | El slug del agente para el tool name se genera con: lowercase, reemplazar espacios por `_`, remover caracteres especiales, max 50 chars. Ejemplo: "Mi Scraper de Noticias" -> `datamirai_execute_mi_scraper_de_noticias` | Slugify util |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/mcp/server.py` — Data MiraiMCPServer + MCPServerManager
- CREAR `framework/src/datamirai_engine/mcp/auth.py` — APIKeyManager + auth middleware
- CREAR `framework/src/datamirai_engine/mcp/config_generator.py` — generador de config snippets
- MODIFICAR `app/server/datamirai_app/database.py` — tabla mcp_api_key + migracion
- CREAR `app/server/datamirai_app/routes/mcp_server.py` — endpoints de MCP server management + API keys
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint mcp_server, lifecycle hooks para auto-start
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints MCP server
- MODIFICAR `app/web/src/app/settings/mcp/page.tsx` — tab server dentro de la pagina MCP settings

---

## Dependencias entre features

```
7.1 (MCP Client) <- independiente, se implementa primero
7.2 (MCP Server) <- independiente de 7.1, puede implementarse en paralelo
```

Ambas features pueden desarrollarse en paralelo ya que son direcciones opuestas de integracion. 7.1 consume tools externas, 7.2 expone tools internas.

Dependencias externas:
- FEAT-001 (vault) — para almacenar API keys y tokens de MCP servers de forma segura
- `mcp` SDK de Anthropic — dependencia del framework para ambas features

---

## Entidades nuevas (resumen consolidado)

### mcp_server_connection
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| description | TEXT | NULL |
| transport | TEXT | NOT NULL (stdio / sse) |
| command | TEXT | NULL |
| args | TEXT | JSON array, NULL |
| url | TEXT | NULL |
| env_vars | TEXT | JSON object, NULL |
| headers | TEXT | JSON object, NULL |
| auto_connect | BOOLEAN | DEFAULT true |
| status | TEXT | DEFAULT 'disconnected' |
| last_error | TEXT | NULL |
| tools_cache | TEXT | JSON array, NULL |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### mcp_api_key
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| name | TEXT | NOT NULL |
| key_hash | TEXT | NOT NULL |
| key_prefix | TEXT | NOT NULL |
| is_active | BOOLEAN | DEFAULT true |
| last_used_at | TEXT | NULL |
| expires_at | TEXT | NULL |
| created_at | TEXT | NOT NULL |

---

## Maquinas de estado

### MCP Server Connection (client-side)

```
DISCONNECTED -> CONNECTING -> CONNECTED
CONNECTED -> DISCONNECTED (manual disconnect)
CONNECTED -> ERROR (connection lost)
ERROR -> CONNECTING (reconnect attempt)
CONNECTING -> ERROR (connection failed)
```

### MCP Server (server-side)

```
STOPPED -> STARTING -> RUNNING
RUNNING -> STOPPING -> STOPPED
RUNNING -> ERROR (crash)
ERROR -> STARTING (restart)
```

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-107 | Conexiones MCP se cierran al apagar server. Sin procesos huerfanos | MCPClientManager.shutdown |
| REGLA-108 | env_vars `vault:` se resuelven contra vault. Sin secrets en plaintext | MCPClientManager |
| REGLA-109 | Server MCP desconectado durante ejecucion -> error descriptivo, sin retry | mcp/tool_call |
| REGLA-110 | Discovery cache se invalida al reconectar. Tools eliminadas fallan descriptivo | MCPConnection |
| REGLA-111 | Max 20 MCP servers simultaneos | POST validation |
| REGLA-112 | Timeout conexion 30s, timeout execution 60s, configurables | MCPConnection |
| REGLA-113 | Solo agentes enabled se exponen como tools MCP | MCP server filter |
| REGLA-114 | API key plaintext solo una vez en response de creacion | Service |
| REGLA-115 | Keys revocadas rechazan inmediatamente (401), no se borran | Auth middleware |
| REGLA-116 | stdio sin auth, SSE requiere API key | MCP auth layer |
| REGLA-117 | Ejecucion MCP sincrona default, timeout 300s, fallback a polling | MCP tool handler |
| REGLA-118 | Slug agente: lowercase, _ por espacios, sin especiales, max 50 | Slugify util |

---

## Notas de implementacion

- **MCP SDK**: usa `mcp` (Model Context Protocol SDK de Anthropic). Dependencia opcional del framework: `pip install datamirai-engine[mcp]`. Sin el extra, los tools mcp/* no estan disponibles y el server MCP no se puede iniciar.
- **Stdio transport**: el MCPClientManager lanza procesos hijo via `asyncio.create_subprocess_exec`. Cada proceso es un MCP server local (ej: `npx @modelcontextprotocol/server-github`). El manager mantiene referencia al proceso para cleanup.
- **SSE transport**: usa `httpx` con SSE streaming para conectar a servers remotos. Compatible con cualquier MCP server que implemente el transporte SSE estandar.
- **Tool discovery caching**: al conectar, se hace un `tools/list` request y se cachea en la columna `tools_cache` de la DB. El cache se usa para mostrar tools en el catalogo sin necesidad de que el server este conectado (informativo, no ejecutable).
- **Config snippets**: el generador produce JSON valido para cada formato. Para Claude Code: `{ "mcpServers": { "datamirai": { "command": "...", "args": [...] } } }`. Para Cursor: formato equivalente segun su spec.
- **Backward compatibility**: todo es opt-in. Si el usuario no configura MCP servers, no hay overhead. La seccion MCP Tools del catalogo esta vacia hasta que se conecte algun server.
- **Proceso stdio del MCP server**: para modo stdio (integracion con Claude Code), el entry point es `python -m datamirai_engine.mcp.server` que lee stdin/escribe stdout. Para modo SSE, es un server HTTP independiente del FastAPI principal, en puerto separado.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-001.md` — vault para credenciales (env_vars de MCP servers)
- `docs/prd/draft/FEAT-002.md` — multi-LLM (agentes que se exponen via MCP usan adapters de FEAT-002)
