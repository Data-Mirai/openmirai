# FEAT-024: MCP Server Integration — Conexion a servicios externos

**Estado**: Draft
**Fecha**: 2026-05-15
**Prioridad**: Critica (prerequisito para caso de onboarding + demo YC)

---

## Problem Statement

**Tipo**: Feature nueva
**Resumen**: Los agentes necesitan conectarse a servicios externos (email, WhatsApp, CRMs, APIs) para ser utiles en casos de uso reales. MCP (Model Context Protocol) es el estandar abierto de Anthropic que resuelve esto.
**Actores afectados**: Usuarios de la plataforma (disenan agentes), agentes en ejecucion (consumen servicios)
**Contexto**: Hoy los agentes solo pueden hacer web scrape, llamar LLMs, y leer/escribir DB. No tienen forma de interactuar con servicios externos como email, mensajeria, o APIs de terceros. MCP es el protocolo estandar que todos los AI frameworks estan adoptando.
**Objetivo**: Un agente debe poder conectarse a cualquier MCP server y usar sus tools como parte de su grafo de ejecucion.

---

## Caso de uso: Onboarding Bot para fondo de trading

Un fondo de trading algoritmico necesita un agente que:

1. **Monitorea email** → detecta aplicaciones a vacante de ventas
2. **Contacta por WhatsApp** → en ES o EN segun idioma del aplicante
3. **Hace preguntas de calificacion** → por WhatsApp
4. **Clasifica al candidato** → segun respuestas
5. **Envia documentos/pruebas** → al candidato calificado
6. **Espera video de respuesta** → con deadline de 48 horas
7. **Si no responde en 48h** → descalifica automaticamente
8. **Si responde** → notifica human-in-the-loop para revision
9. **Decision final** → aceptar o rechazar

### MCP Servers requeridos:
- **WhatsApp**: whatsapp-mcp-ts (Baileys) — enviar/recibir mensajes, listar chats
- **Gmail**: gmail-mcp — leer inbox, filtrar por asunto/remitente

---

## Arquitectura

### Concepto: MCP Servers como Servicios

Los MCP servers son procesos externos (Node.js, Python) que corren al lado de la app, igual que Ollama o PostgreSQL. Se comunican via stdio o SSE.

```
┌─────────────────────────────────────────┐
│  Data Mirai App                            │
│  ┌───────────┐  ┌───────────────────┐   │
│  │ Agent     │  │ MCP Client        │   │
│  │ Runner    │──│ (Python SDK)      │   │
│  └───────────┘  └────────┬──────────┘   │
│                          │ stdio/SSE    │
├──────────────────────────┼──────────────┤
│  Servicios externos      │              │
│  ┌───────────────────────▼────────┐     │
│  │ whatsapp-mcp-ts (Node.js)     │     │
│  │ gmail-mcp (Node.js)           │     │
│  │ chatwoot-mcp (Node.js)        │     │
│  │ ... cualquier MCP server      │     │
│  └────────────────────────────────┘     │
└─────────────────────────────────────────┘
```

### Componentes a implementar

#### 1. MCP Client Runtime (framework/)
- `framework/src/datamirai_engine/mcp/client.py`
- Conecta a MCP servers via stdio (proceso hijo) o SSE (HTTP)
- Lista tools disponibles del server
- Ejecuta tools con argumentos
- Maneja lifecycle (connect, disconnect, reconnect)
- Usa SDK oficial: `mcp==1.27.1`

#### 2. Tool `mcp/call` (framework/)
- `framework/src/datamirai_engine/tools/builtin/mcp/mcp_call.py`
- Nuevo tool type para el grafo
- Config: `server_name` (cual MCP server usar), `tool_name` (cual tool del server), `arguments` (parametros)
- El server se resuelve desde la config del agente (`agent.config.mcp_servers`)

#### 3. MCP Server Registry (app/server/)
- Configuracion en la app de que MCP servers estan disponibles
- Endpoint API para listar servers configurados y sus tools
- Health check para verificar que el server esta corriendo
- UI en Settings para agregar/remover MCP servers

#### 4. MCP Server Config en AgentSpec
- Ya existe `mcp_servers: list[AgentMcpServerSpec]` en agent_spec.py
- Enriquecer con: nombre, comando, args, env, transport (stdio/sse)

---

## Reglas

- REGLA-400: MCP servers son procesos externos, no se embeben en el engine
- REGLA-401: Comunicacion via stdio (default) o SSE (alternativa)
- REGLA-402: Cada MCP server se registra con: name, command, args, env
- REGLA-403: El tool mcp/call resuelve el server por nombre desde la config del agente
- REGLA-404: Si el MCP server no esta corriendo, el tool falla con error descriptivo
- REGLA-405: Los tools del MCP server se listan dinamicamente (no hardcodeados)
- REGLA-406: El MCP client maneja reconnect automatico si el server cae
- REGLA-407: El catalogo de MCPs verificados es curado y vive en el engine
- REGLA-408: El designer de agentes conoce los MCP servers instalados y sus tools para sugerirlos

---

## Catalogo de MCP Servers Verificados

La app incluye un catalogo curado de MCP servers verificados (fuente: smithery.ai).
El usuario navega el catalogo, selecciona un server, y con un click lo instala/configura.

### Estructura del catalogo

Cada entrada del catalogo tiene:
- `id`: identificador unico (ej: "whatsapp-baileys")
- `name`: nombre para mostrar (ej: "WhatsApp (Baileys)")
- `description`: que hace
- `category`: comunicacion | productividad | datos | busqueda | finanzas | desarrollo
- `source`: URL del repo o smithery.ai
- `command`: comando para ejecutar (ej: "npx whatsapp-mcp-ts")
- `install_command`: comando para instalar dependencias
- `requires`: prerequisitos (ej: "Node.js 18+")
- `auth_type`: "qr" | "api_key" | "oauth" | "none"
- `verified`: true si viene de smithery.ai o repos oficiales
- `tools_preview`: lista de tools que expone (para el designer)

### Catalogo inicial (MVP)

| ID | Nombre | Categoria | Auth |
|----|--------|-----------|------|
| whatsapp-baileys | WhatsApp (Baileys) | comunicacion | QR scan |
| gmail | Gmail | comunicacion | OAuth/API key |
| slack | Slack | comunicacion | API key |
| notion | Notion | productividad | API key |
| google-sheets | Google Sheets | productividad | OAuth |
| google-drive | Google Drive | productividad | OAuth |
| brave-search | Brave Search | busqueda | API key |
| github | GitHub | desarrollo | API key |
| filesystem | Filesystem | desarrollo | none |
| supabase | Supabase | datos | API key |

### Flujo de usuario

1. Va a Settings → seccion "MCP Servers"
2. Ve servers instalados + boton "Explorar catalogo"
3. Catalogo muestra servers verificados con descripcion y categoria
4. Click "Instalar" → ejecuta install_command + registra el server
5. Si requiere auth (API key, QR) → muestra formulario de configuracion
6. Server listo → aparece en el designer como tools disponibles

### Designer awareness

El system prompt del designer de agentes se enriquece con:
- Lista de MCP servers instalados y sus tools
- Cuando el usuario pide algo que requiere un servicio externo (email, WhatsApp, Slack),
  el designer sugiere el MCP server correcto y configura el nodo mcp/call

---

## Scope del MVP (para demo YC)

### Incluido:
- [x] MCP Client runtime con conexion stdio
- [x] Tool mcp/call funcional en grafos
- [x] WhatsApp MCP server (Baileys) como servicio
- [x] Gmail MCP server como servicio
- [x] Configuracion de MCP servers en la app
- [x] Catalogo curado de MCP servers verificados en la UI
- [x] Designer conoce MCP servers instalados y sugiere nodos mcp/call
- [x] Agente de onboarding disenado y ejecutable
- [x] Tests E2E Playwright validando el flujo

### Excluido (post-MVP):
- SSE transport (solo stdio en MVP)
- Auto-discovery de MCP servers
- MCP server development kit

---

## Tests E2E esperados

### API
1. GET /api/mcp/catalog — devuelve catalogo curado con 10+ servers
2. GET /api/mcp/servers — lista MCP servers instalados/configurados
3. GET /api/mcp/servers/:name/tools — lista tools de un server especifico
4. POST /api/mcp/servers/:name/call — ejecuta tool y recibe resultado
5. POST /api/mcp/servers — instalar/registrar un MCP server nuevo
6. DELETE /api/mcp/servers/:name — desinstalar un MCP server

### UI — Settings
7. Settings muestra seccion "MCP Servers" con servers instalados
8. Boton "Explorar catalogo" abre catalogo con categorias
9. Catalogo muestra servers verificados con badge de verificacion
10. Click "Instalar" registra el server y aparece en la lista de instalados

### UI — Designer
11. Designer muestra tools de MCP servers instalados en el catalogo de herramientas
12. Designer sugiere MCP server cuando usuario pide funcionalidad externa
13. Nodo mcp/call se configura con server_name + tool_name
14. Agente con nodos mcp/call se guarda correctamente

### E2E — Integracion real
15. WhatsApp MCP — enviar mensaje a numero real y verificar entrega
16. Gmail MCP — leer inbox y listar emails recientes

---

## Dependencias

- Python SDK: `mcp==1.27.1` (ya instalado)
- Node.js: para correr MCP servers (whatsapp-mcp-ts, gmail-mcp)
- npm packages: `@anthropic-ai/sdk`, `@whiskeysockets/baileys`
- Enums: ToolType.MCP_CALL (agregar a enums.py)
