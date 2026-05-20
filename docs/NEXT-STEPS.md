# Data Mirai Engine — Próximos pasos

Estado actual: motor funcional (267 tests), runtime con scheduler/webhooks, 11 pantallas Cosmic Glass conectadas al API, schema de DB diseñado.

---

## PROMPT 1: YAML Agent Definition + Export/Import

### Contexto

Los agentes actualmente se definen via API (JSON). Necesitamos que un agente pueda expresarse como YAML — leerlo, editarlo, exportarlo, importarlo, versionarlo. Esto es como Anthropic define agentes en Claude Console.

### Qué implementar

1. **AgentSpec dataclass** en `src/datamirai_engine/core/agent_spec.py`:

```yaml
# Formato YAML de un agente
name: document-pipeline
description: Procesa documentos, transcribe audio, genera embeddings
version: v3

graph:
  nodes:
    - id: t1
      block_type: trigger/webhook
      config:
        method: POST
        path: /webhooks/document-pipeline
    - id: n1
      block_type: ai/transcribe
      config:
        model: whisper-large-v3
    - id: n2
      block_type: ai/embeddings
    - id: n3
      block_type: data/db_write
      config:
        table: documents
  edges:
    - id: e1
      source: t1
      target: n1
    - id: e2
      source: n1
      target: n2
    - id: e3
      source: n2
      target: n3

triggers:
  - type: webhook
    path: /webhooks/document-pipeline
    auth: api_key
  - type: schedule
    interval_seconds: 3600

config:
  max_iterations: 50
  retry:
    max_retries: 3
    backoff: exponential
    on_failure: route_to_error
  timeout_ms: 60000

resources:
  - docs-s3
  - pgvector
  - anthropic-prod

metadata:
  created_by: mateo
  tags: [documents, transcription, embeddings]
```

2. **Serialización bidireccional**:
   - `AgentSpec.from_yaml(yaml_string)` → parsea y valida
   - `AgentSpec.to_yaml()` → genera YAML limpio
   - `AgentSpec.from_dict(dict)` / `AgentSpec.to_dict()` → JSON compatible
   - `AgentSpec.to_graph()` → retorna `GraphDef` listo para ejecutar
   - Validación: nodos referencian block_types registrados, edges referencian nodos existentes

3. **API endpoints**:
   - `GET /api/agents/{id}/spec` → retorna YAML o JSON según Accept header
   - `PUT /api/agents/{id}/spec` → actualiza agente desde YAML/JSON
   - `POST /api/agents/import` → crea agente desde YAML
   - `POST /api/agents/{id}/version` → crea nueva versión del agente

4. **Frontend: modal de edición YAML** (como el de Claude Console):
   - En `/agents/[id]`, botón "Editar" abre modal
   - Tabs: YAML | JSON (toggle entre formatos)
   - Editor de texto con syntax highlighting mono
   - Botón "Save new version" → POST /api/agents/{id}/version
   - Preview de cambios antes de guardar

5. **CLI: agentes desde archivos YAML**:
   ```bash
   # Cargar agente desde archivo
   datamirai agent load agent.yaml

   # Exportar agente a archivo
   datamirai agent export document-pipeline > agent.yaml

   # Listar agentes
   datamirai agent list
   ```

6. **Tests**:
   - Parsear YAML → AgentSpec → validar
   - AgentSpec → YAML → AgentSpec roundtrip
   - AgentSpec → GraphDef → ejecutar con GraphRunner
   - API: import YAML → agente creado → export YAML → igual
   - Versioning: v1 → edit → v2 → rollback v1

### Archivos a crear/modificar

```
CREAR:
  src/datamirai_engine/core/agent_spec.py     — AgentSpec dataclass + YAML serialization
  tests/core/test_agent_spec.py             — tests de serialización + validación

MODIFICAR:
  src/datamirai_engine/server/app.py          — endpoints spec/import/version
  src/datamirai_engine/cli.py                 — comandos agent load/export/list
  src/datamirai_engine/__init__.py            — exportar AgentSpec
  editor/src/app/agents/[id]/page.tsx       — modal YAML editor
  editor/src/lib/api.ts                     — métodos getAgentSpec, importAgent, etc.
  pyproject.toml                            — agregar pyyaml como dependencia
```

### Dependencia nueva
```
pyyaml >= 6.0
```

---

## PROMPT 2: Persistencia Postgres + Sesiones con Transcript

### Contexto

Todo está in-memory — se pierde al reiniciar. El schema SQL ya existe en `src/datamirai_engine/db/schema.sql`. Necesitamos conectar el server a Postgres real y enriquecer las sesiones para que tengan transcript legible (como Claude Console), no solo trace técnico.

### Qué implementar

1. **Database connection layer** en `src/datamirai_engine/db/`:
   - `connection.py` — pool de conexiones asyncpg
   - `repository.py` — repositorio con métodos CRUD para cada tabla
   - Inicialización: `datamirai db init` ejecuta `schema.sql`
   - Config via env vars: `DATABASE_URL=postgresql://...`

2. **Repositorios por entidad**:
   ```python
   class GraphRepository:
       async def create(self, graph: GraphDef, environment_id: str) -> dict
       async def get(self, graph_id: str) -> dict | None
       async def list(self, environment_id: str) -> list[dict]
       async def update(self, graph_id: str, data: dict) -> dict
       async def delete(self, graph_id: str) -> None

   class AgentRepository:
       async def create(self, agent_data: dict) -> dict
       async def get(self, agent_id: str) -> dict | None
       async def list(self, environment_id: str) -> list[dict]
       async def update_status(self, agent_id: str, status: str) -> dict
       async def delete(self, agent_id: str) -> None

   class SessionRepository:
       async def create(self, session_data: dict) -> dict
       async def get(self, session_id: str) -> dict | None
       async def list(self, agent_id: str | None, limit: int) -> list[dict]
       async def update(self, session_id: str, data: dict) -> dict

   # + ResourceRepository, UserRepository, SettingsRepository, MemoryRepository
   ```

3. **Sesiones con transcript** (inspirado en Claude Console):
   - Cada sesión tiene `transcript: list[TranscriptEvent]`
   - Eventos: `user_input`, `agent_action`, `tool_call`, `tool_result`, `agent_response`, `error`
   - Cada evento: `type`, `content`, `timestamp`, `duration_ms`, `tokens` (si aplica)
   - El GraphRunner emite eventos al ShortTermMemory durante ejecución
   - Al finalizar, se persiste como transcript en la sesión

   ```python
   class TranscriptEvent:
       type: str          # "trigger", "block_start", "block_end", "decision", "error"
       node_id: str
       block_type: str
       content: str       # descripción legible
       input: dict        # qué recibió
       output: dict       # qué produjo
       timestamp: float
       duration_ms: float
       tokens_used: int   # si es bloque AI
   ```

4. **Server: swap in-memory → Postgres**:
   - `create_app(database_url=None)` — si None, usa in-memory (dev sin DB)
   - Si database_url, conecta a Postgres y usa repositorios
   - Lifespan: crear pool en startup, cerrar en shutdown
   - Migrar AgentRuntime para usar repositorios en vez de dicts

5. **CLI: init de base de datos**:
   ```bash
   # Inicializar DB (ejecuta schema.sql)
   datamirai db init

   # Verificar conexión
   datamirai db status

   # Reset (development only)
   datamirai db reset
   ```

6. **Frontend: session transcript view**:
   - En `/sessions/[id]`, mostrar transcript como timeline legible:
     - Cada evento con icono, timestamp, duración
     - Bloques AI muestran tokens usados
     - Expand/collapse para ver input/output completo
     - Progress bar mostrando avance del grafo
   - Similar a la vista de Claude Console pero adaptada a grafos

### Archivos a crear/modificar

```
CREAR:
  src/datamirai_engine/db/connection.py       — asyncpg pool management
  src/datamirai_engine/db/repositories.py     — CRUD repos por entidad
  tests/db/test_repositories.py             — tests con testcontainers o fixture

MODIFICAR:
  src/datamirai_engine/db/schema.sql          — agregar campo transcript a sessions
  src/datamirai_engine/server/app.py          — usar repos en vez de dicts
  src/datamirai_engine/runtime/agent_runtime.py — persistir sessions en DB
  src/datamirai_engine/core/runner.py         — emitir TranscriptEvents
  src/datamirai_engine/memory/short_term.py   — capturar transcript events
  src/datamirai_engine/cli.py                 — comandos db init/status/reset
  src/datamirai_engine/__init__.py            — exportar TranscriptEvent
  editor/src/app/sessions/[id]/page.tsx     — transcript view
  editor/src/lib/api.ts                     — getSession endpoint
  pyproject.toml                            — agregar asyncpg como optional dep
  docker-compose.yml                        — verificar postgres config
```

### Dependencias nuevas
```
[project.optional-dependencies]
postgres = ["asyncpg>=0.30"]
all = ["datamirai-engine[server,postgres]"]
```

### Orden de implementación sugerido
```
1. connection.py + db init CLI
2. repositories.py (CRUD por entidad)
3. TranscriptEvent en runner
4. server swap in-memory → repos
5. Frontend transcript view
6. Tests con postgres real (docker-compose)
```

---

## Ideas para el futuro (no implementar ahora)

- **Chat-based agent creator**: "Describe your agent..." → genera YAML automáticamente + templates predefinidos
- **Agent templates**: blank, deep researcher, support agent, data analyst, etc.
- **Credential vaults**: gestión segura de API keys y secrets (actualmente en resources.credentials)
- **Memory stores**: UI dedicada para gestionar memoria persistente de agentes
- **Analytics**: dashboard de métricas (tokens, latencia, errores por agente)
- **MCP server exposure**: cada agente expone un endpoint MCP para que otras apps lo consuman
