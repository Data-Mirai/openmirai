# Data Mirai Engine

Motor open source de ejecucion de grafos agentivos. Alternativa a LangGraph y Google ADK.

Compilado en **Rust** — portable via FFI, WASM o CLI. El agente es un JSON, el motor es un binario.

## Estructura

```
rust/          Motor Rust (crate principal + CLI)
  engine/      Crate: datamirai-engine
  cli/         CLI: mirai run, mirai validate
legacy/        Paquete Python (referencia historica, no usar)
docs/          Documentacion tecnica y PRDs
blueprint/     Blueprint Agents (gestion de proyecto)
```

## Quick Start

```bash
# Compilar
cd rust && cargo build --release

# Validar un agente
mirai validate agent.json

# Ejecutar un agente
mirai run agent.json
mirai run agent.yaml --input '{"query": "hola"}'
```

## Agente = JSON

```json
{
  "name": "mi-agente",
  "version": "v1",
  "graph": {
    "nodes": [
      {"id": "trigger", "tool_type": "trigger/manual"},
      {"id": "llm", "tool_type": "ai/llm_call", "config": {"model": "gemma4"}}
    ],
    "edges": [
      {"source": "trigger", "target": "llm"}
    ]
  }
}
```

Edge IDs son opcionales (auto-generados). data_map es opcional para flujos lineales.

## Tools builtin

| Categoria | Tools |
|-----------|-------|
| Trigger | webhook, manual, schedule, event |
| Logic | condition, switch, loop, merge, wait, human_input |
| AI | llm_call, transcribe, embeddings |
| Data | db_read, db_write, storage_read, storage_write |
| Filesystem | read_file, write_file, edit_file, glob, grep, tree |
| System | bash, process_list |
| Git | status, diff, log, commit |
| Output | response |
| Agent | run_agent |

## Arquitectura de 3 capas

```
Agente = JSON config (portable, versionable, language-agnostic)
Motor  = Rust binary (FFI, WASM, CLI — 553 tests)
Host   = App que abraza motor + agente (Python, Swift, Go, cualquiera)
```

## Licencia

MIT
