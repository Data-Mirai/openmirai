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

## Frontend

_(Engine no tiene frontend — las interfaces son CLI y HTTP API)_
