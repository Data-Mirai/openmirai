# PRD-007 — ai/claude_code: Native Claude Code CLI Tool

| Campo | Valor |
|-------|-------|
| **ID** | PRD-007 |
| **Fecha** | 2026-05-28 |
| **Estado** | in_progress |
| **Target** | v0.4.4 |

---

## Problema

Para usar Claude Code CLI como LLM dentro de un agente, se necesita un workaround con system/bash + archivos temporales. El agente no es portable y el host tiene que inyectar el comando en runtime.

## Solución

Nuevo tool type `ai/claude_code` que habla directamente con el CLI de Claude Code instalado localmente, usando la suscripción del usuario (Max/Pro). Zero API keys, zero archivos temporales, zero inyección del host.

## Spec del nodo

```yaml
- id: think
  tool_type: ai/claude_code
  config:
    timeout_ms: 60000
    max_tokens: 2048
    system_prompt: "..."
    model: "claude-sonnet-4-20250514"
    cli_path: null
```

Inputs (via data_map): `prompt` (required), `context` (optional)
Outputs: `response`, `model`, `duration_ms`, `tokens_input`, `tokens_output`

## Implementación

1. Detectar `claude` en $PATH
2. Spawnar `claude -p` como child process
3. Prompt por stdin (seguro, no expone en ps aux)
4. Respuesta de stdout, errores de stderr
5. Timeout configurable (default 60s)
6. Estimación de tokens: len/4
