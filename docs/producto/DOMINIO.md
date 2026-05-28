# DOMINIO.md

## 1. Glosario del Dominio {#glosario}

| Termino | Definicion | Sinonimos |
|---|---|---|
| Agent | Unidad de trabajo autonoma definida en un archivo YAML. Contiene un grafo de nodos, triggers, config y opcionalmente contratos de entrada/salida. | workflow, pipeline |
| AgentSpec | Representacion en YAML de un agente: nombre, version, grafo, triggers, config, inputs, outputs, mcp_servers. | spec, agent definition |
| Graph | Grafo dirigido aciclico (DAG) que define el flujo de ejecucion. Compuesto por nodos y edges. | workflow graph, pipeline |
| Node | Unidad atomica de ejecucion dentro del grafo. Cada nodo tiene un `tool_type` que determina su comportamiento. | step, block |
| Edge | Conexion entre dos nodos. Puede ser incondicional, condicional (con operador + valor), o con data_map. | connection, link |
| EdgeCondition | Condicion que un edge evalua para decidir si se activa: field + operator + value. | routing condition |
| Tool | Implementacion concreta de una accion (llm_call, read_file, bash, etc.). Registrado en el ToolRegistry. | herramienta, action |
| ToolSpec | Metadatos de un tool: inputs, outputs, config_fields, tipos, requeridos. | tool schema |
| ToolRegistry | Registro central de todos los tools disponibles. Mapea `tool_type` → factory. | — |
| Trigger | Nodo de entrada del grafo que inicia la ejecucion. Tipos: manual, webhook, schedule, event, heartbeat. | entry point |
| Provider | Adaptador de LLM que conecta con un servicio especifico (Ollama, Claude, OpenAI, etc.). | LLM adapter |
| ExecutionContext | Interfaz (port) que expone recursos al runner: DB, LLM, Storage, Vector. | context |
| ExecutionState | Estado compartido thread-safe durante la ejecucion. Cada nodo escribe su output aqui. | shared state |
| ExecutionResult | Resultado final de ejecutar un grafo: status, state, trace, transcript, error. | result |
| TraceEntry | Registro de un nodo ejecutado: id, tool_type, status, duracion, retries, error. | trace item |
| Session | Instancia unica de ejecucion de un agente. Tiene ID, timestamp, resultado. | execution, run |
| Soul | Personalidad del agente definida en un archivo SOUL.md. Se inyecta como system prompt. | personality |
| Universe | Sistema de multi-agente: routing de mensajes entre agentes con estrategias (keyword, round-robin, LLM, explicit). | multi-agent router |
| data_map | Mapeo de campos entre nodos: `{target_input: "source_node.output_field"}`. Soporta dot notation anidada. | field mapping |
| MCP | Model Context Protocol — protocolo para conectar herramientas externas via stdio o HTTP. | — |
| MCPServer | Servidor externo que expone herramientas via MCP. Configurado en `mcp_servers` del AgentSpec. | tool server |
| Hook | Punto de intercepcion durante la ejecucion (7 puntos: graph start/end, pre/post block, pre/post LLM, on_error). | interceptor, middleware |
| Checkpoint | Snapshot del estado de ejecucion para pause/resume. | save point |
| Energy | Sistema de tracking de costo por operacion (tokens LLM, llamadas API). | cost tracking |
| Sandbox | Entorno aislado para ejecutar codigo no confiable (`system/sandbox_exec`). | — |
| Scanner | Detector de prompt injection con niveles de sensibilidad (Low, Medium, High). | security scanner |

## 2. Roles {#roles}

### rol-engine {#rol-engine}
**Descripcion:** El motor de ejecucion. Procesa grafos, ejecuta tools, gestiona estado.
**Capabilities:** {#capabilities-engine}
- `execute_graph` — recorrer un grafo nodo por nodo
- `resolve_inputs` — resolver data_map y templates entre nodos
- `evaluate_conditions` — evaluar condiciones de edges para routing
- `manage_state` — leer/escribir estado compartido thread-safe
- `emit_events` — emitir eventos de ejecucion (SSE)
- `apply_hooks` — ejecutar hooks en los 7 puntos de intercepcion
- `retry_with_backoff` — reintentar nodos fallidos con politica configurable
- `checkpoint_resume` — guardar y restaurar estado para pause/resume

### rol-cli {#rol-cli}
**Descripcion:** Interfaz de linea de comandos. Punto de entrada para usuarios.
**Capabilities:** {#capabilities-cli}
- `run_agent` — ejecutar un agente desde archivo YAML
- `validate_spec` — validar sintaxis de un AgentSpec
- `serve_http` — iniciar servidor HTTP
- `setup_wizard` — configurar provider y API key interactivamente
- `show_version` — mostrar version del engine

### rol-server {#rol-server}
**Descripcion:** API HTTP para integracion remota. Feature-gated (opcional).
**Capabilities:** {#capabilities-server}
- `crud_agents` — crear, leer, listar, eliminar agentes
- `crud_graphs` — crear, leer, listar, eliminar grafos
- `execute_sync` — ejecutar agente y esperar resultado
- `execute_stream` — ejecutar agente con SSE streaming
- `manage_sessions` — listar y consultar sesiones
- `authenticate` — validar X-API-Key header
- `receive_webhooks` — recibir webhooks en /webhooks/{path}

### rol-product-engineer {#rol-product-engineer}
**Descripcion:** El usuario que disena y opera agentes. Escribe YAML, ejecuta via CLI o API.
**Capabilities:** {#capabilities-pe}
- `design_agent` — escribir AgentSpec en YAML
- `run_agent` — ejecutar agentes via CLI o API
- `monitor_execution` — observar trace, transcript, SSE events
- `configure_providers` — elegir LLM provider y modelo
- `compose_agents` — crear agentes que llaman a otros agentes

## 3. Jerarquia de Roles

```
Product Engineer (humano)
  └── usa → CLI / Server (interfaz)
                └── delega → Engine (motor)
                                ├── Tools (48+ herramientas)
                                ├── Providers (7 LLM adapters)
                                ├── MCP Servers (herramientas externas)
                                └── Adapters (DB, Storage, Vector)
```

No hay herencia entre roles — cada uno tiene responsabilidades distintas. El Product Engineer interactua exclusivamente con CLI o Server, nunca con el Engine directamente.
