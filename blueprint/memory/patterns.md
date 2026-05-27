# Patrones aprobados

Patrones y convenciones que el proyecto adopta. Todo agente los lee antes de implementar.

## Testing

### REGLA: Zero mocks fuera de unit tests

**Mocks SOLO están permitidos dentro de `#[cfg(test)]` en unit tests aislados.**

Todo lo demás — server, CLI, integration tests, E2E — DEBE usar implementaciones reales:
- LLM: Ollama local o provider real. `MockLLMResource` prohibido en rutas de producción.
- DB: SQLite real o in-memory SQLite. No fake DBs en integration.
- MCP: Server real (o mock-mcp-server.js que ES un server real via stdio).
- Storage: filesystem real o temp dir.

Si un feature necesita un servicio externo para funcionar y no está disponible, debe **fallar con error claro**, no devolver datos falsos silenciosamente.

**Violación detectada (2026-05-27):** `server/app.rs` usaba `MockLLMResource::new()` en producción. Toda la API devolvía `"[mock response to: ...]"`. Esto hizo que streaming, universe, eval, RAG parecieran funcionales cuando eran fachada.

## Backend

### Provider resolution unificada
CLI y Server usan la misma cadena: flag > env var > auto-detect > default ollama.
Implementación: `adapter_factory::create_adapter()` + `AdapterBridgeLLMResource`.

### MCP injection
`mcp_servers` del AgentSpec se inyectan como `__mcp_servers` en config de nodos `mcp/call`.

### YAML-only para agent specs (PRD-004)
YAML es el único formato para specs. `from_json()`/`to_json()` eliminados.
`from_file()` usa `serde_yaml` para todo (parsea JSON syntax también).
HTTP API bodies siguen siendo JSON (estándar HTTP).

### Input validation: 2 capas (PRD-004)
- **Capa 1 (AgentSpec)**: `validate_agent_inputs()` en agent_spec.rs valida payload del client contra `spec.inputs`. CLI exit(1), HTTP 422.
- **Capa 2 (Node)**: `validate_node_inputs()` en tools/base.rs valida inputs resueltos contra `ToolSpec.inputs` antes de cada `tool.execute()`.
- **catch_unwind**: RegistryExecutor envuelve `tool.execute()` en catch_unwind. Panics → ToolError.

### Nested field traversal (PRD-004)
`SharedState.get_field()` soporta dot-separated paths: `trigger.payload.question`.
Backward compat: paths sin dots funcionan idéntico.

### Trigger output: payload (PRD-004)
ManualTriggerTool y WebhookTriggerTool producen `payload` como campo principal.
`user_input` y `body` se mantienen como alias de backward compat.
Injection key: `config["payload"]` (antes `mock_payload`).

## Frontend

_(Engine no tiene frontend — las interfaces son CLI y HTTP API)_
