# FEAT-002 — Tier 1: Fundamento (Multi-LLM + Persistencia + Busqueda)

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-020

---

## Problem Statement

**Tipo**: Feature nueva (3 capacidades fundamentales)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine tiene core funcional (308+ tests, 16 tools builtin, memoria 2 niveles) pero presenta tres carencias criticas que bloquean todo el roadmap:

1. **LLM unico**: El nodo `ai/llm_call` hoy solo funciona a traves del protocolo `LLMResource` sin adapters. Cada provider (Claude, GPT, Gemini, Ollama, Groq, OpenRouter) retorna formatos distintos — Ollama incluye bloques `<think>` que hay que limpiar, Claude retorna content blocks, OpenAI retorna choices. No hay normalizacion ni seleccion de provider.

2. **Memoria volatil**: `LongTermMemory` es una lista en RAM (`self._entries: list[dict]`). Se pierde al cerrar la app. La busqueda es substring matching basico. No hay persistencia a disco ni busqueda semantica.

3. **Sin busqueda semantica**: El tool `ai/embeddings` genera vectores pero no hay infraestructura para almacenarlos ni buscar por similitud. Sin esto no hay RAG, no hay memoria inteligente, no hay reutilizacion de conocimiento entre sesiones.

Estas tres carencias se bloquean mutuamente con el resto del roadmap: sin multi-LLM no hay embeddings confiables, sin persistencia no hay memoria util, sin busqueda no hay agentes que aprendan.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Configurar multiples LLM providers (Ollama local, Claude, GPT, Gemini, Groq, OpenRouter) y seleccionar cual usar por nodo
2. Probar conexion a cada provider desde la UI antes de usarlo
3. Conservar la memoria de largo plazo de sus agentes entre sesiones (persiste a SQLite, opcionalmente a PostgreSQL)
4. Buscar en la memoria del agente por significado (semantica) y por keywords (full-text), combinados
5. Ver y gestionar las memorias de cada agente desde la UI

---

## Features

### 1.1 — Multi-LLM Provider (Adapter Pattern)

**Problema**: Hoy el nodo `ai/llm_call` delega en `context.llm.call()` que implementa `LLMResource` protocol. No hay capa de normalizacion entre providers. Cada provider responde con formato diferente: Ollama incluye bloques `<think>` que hay que limpiar, Claude retorna content blocks con types, OpenAI retorna choices con message objects, Gemini retorna candidates. El usuario no puede elegir provider por nodo ni gestionar multiples providers.

**Solucion**: Adapter pattern donde cada provider tiene su adapter que normaliza input/output. Un registry selecciona el adapter correcto basado en configuracion. La UI permite configurar providers y seleccionar cual usar en cada nodo `ai/llm_call`.

**Arquitectura**:
- Interfaz `LLMAdapter` (abstract base class) con metodos:
  - `call(prompt, context, config) -> NormalizedResponse` — llamada sincrona
  - `stream(prompt, context, config) -> AsyncIterator[NormalizedChunk]` — streaming
  - `embed(text, model) -> list[float]` — generar embeddings
  - `list_models() -> list[ModelInfo]` — listar modelos disponibles
  - `test_connection() -> ConnectionTestResult` — verificar que el provider responde
- `NormalizedResponse`: `{ response: str, tokens_used: { input: int, output: int }, model: str, provider: str }`
- `NormalizedChunk`: `{ delta: str, done: bool, tokens_used: { input: int, output: int } | None }`
- `ModelInfo`: `{ id: str, name: str, context_window: int | None, supports_streaming: bool }`
- `ConnectionTestResult`: `{ success: bool, latency_ms: int, error: str | None }`
- Adapters concretos:
  - `OllamaAdapter` — HTTP contra localhost, strip de `<think>` tags, embeddings via `/api/embeddings`
  - `ClaudeAdapter` — Anthropic SDK, extrae texto de content blocks, embeddings via Voyage API o fallback
  - `OpenAIAdapter` — OpenAI SDK, extrae choices[0].message.content, embeddings via `/v1/embeddings`
  - `GeminiAdapter` — Google GenAI SDK, extrae candidates[0].content, embeddings via embedding-001
  - `GroqAdapter` — API compatible OpenAI, misma extraccion, sin embeddings propios (fallback a otro provider)
  - `OpenRouterAdapter` — API compatible OpenAI con header X-Title, sin embeddings propios (fallback)
- `LLMAdapterRegistry`: mapa de provider_name → adapter class. Metodo `get_adapter(provider_config) -> LLMAdapter`. Singleton.
- `LLMAdapterFactory`: instancia adapters con config concreta (api_key, base_url, model). Cache por provider_id para reutilizar conexiones.

**Entidades nuevas**:

Tabla `llm_provider_config` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| provider | TEXT | NOT NULL (ollama / claude / openai / gemini / groq / openrouter) |
| display_name | TEXT | NOT NULL |
| api_key_credential_id | TEXT | FK credential, NULL (Ollama no necesita) |
| base_url | TEXT | NULL (override para self-hosted o proxy) |
| default_model | TEXT | NOT NULL |
| embedding_model | TEXT | NULL (modelo para embeddings, si soporta) |
| is_default | BOOLEAN | DEFAULT false |
| max_timeout_seconds | INTEGER | DEFAULT 60 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

**Contratos API**:
- `GET /api/llm-providers` — listar providers configurados. Response: `{ providers: LLMProviderConfig[] }`
- `POST /api/llm-providers` — crear/configurar provider. Body: `{ provider, display_name, api_key?, base_url?, default_model, embedding_model? }`. Response: `{ provider: LLMProviderConfig }`
- `GET /api/llm-providers/{id}` — detalle de provider. Response: `{ provider: LLMProviderConfig }`
- `PATCH /api/llm-providers/{id}` — actualizar config. Body: campos parciales. Response: `{ provider: LLMProviderConfig }`
- `DELETE /api/llm-providers/{id}` — eliminar provider. Guard: no eliminar si es el unico provider activo. Response: `204`
- `POST /api/llm-providers/{id}/test` — probar conexion con prompt de prueba. Response: `{ success, latency_ms, error?, model_used }`
- `GET /api/llm-providers/{id}/models` — listar modelos disponibles del provider. Response: `{ models: ModelInfo[] }`

**Pantallas**:
- **Settings → seccion "LLM Providers"**: lista de providers configurados con indicador de estado (connected/error). Formulario para agregar nuevo provider: selector de tipo (Ollama/Claude/GPT/etc), campos dinamicos segun tipo (API key para cloud, base_url para Ollama). Boton "Test Connection". Indicador de cual es el default. Accion para eliminar (con confirmacion).
- **EditorCanvas → nodo ai/llm_call → NodeConfigPanel**: selector de provider (dropdown con providers configurados) + selector de modelo (dropdown que se llena al seleccionar provider via endpoint `/models`). Campos temperature/max_tokens se mantienen.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-30 | Todo adapter retorna NormalizedResponse. Nunca datos raw del provider al consumidor | Adapter base class con validacion en metodo abstracto |
| REGLA-31 | API keys nunca en logs ni responses. Se almacenan referenciando credential_id del vault (FEAT-001). En responses se muestran como `***` | Serializer + vault |
| REGLA-32 | Si un provider no responde en timeout (configurable, default 60s), el nodo falla con error descriptivo incluyendo provider y modelo | Adapter base con asyncio.wait_for |
| REGLA-33 | OllamaAdapter SIEMPRE strip de `<think>...</think>` tags antes de retornar, incluyendo nested y malformados (tag abierto sin cerrar) | OllamaAdapter._clean_response |
| REGLA-34 | Exactamente 1 provider puede ser is_default=true. Al marcar uno, los demas se desmarcan automaticamente | Service (transaccion atomica UPDATE) |
| REGLA-35 | Si un adapter no soporta embeddings (Groq, OpenRouter), el metodo embed() lanza NotImplementedError. El sistema cae al embedding provider configurado como fallback | LLMAdapterFactory.get_embedding_adapter |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/llm/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/llm/adapter.py` — ABC LLMAdapter + NormalizedResponse + NormalizedChunk + ModelInfo + ConnectionTestResult
- CREAR `framework/src/datamirai_engine/llm/registry.py` — LLMAdapterRegistry + LLMAdapterFactory
- CREAR `framework/src/datamirai_engine/llm/adapters/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/llm/adapters/ollama.py` — OllamaAdapter
- CREAR `framework/src/datamirai_engine/llm/adapters/claude.py` — ClaudeAdapter
- CREAR `framework/src/datamirai_engine/llm/adapters/openai_adapter.py` — OpenAIAdapter (nombre evita colision con paquete openai)
- CREAR `framework/src/datamirai_engine/llm/adapters/gemini.py` — GeminiAdapter
- CREAR `framework/src/datamirai_engine/llm/adapters/groq.py` — GroqAdapter
- CREAR `framework/src/datamirai_engine/llm/adapters/openrouter.py` — OpenRouterAdapter
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/ai/llm_call.py` — usar LLMAdapterFactory en vez de context.llm directo. Leer provider_id de config del nodo. Fallback a default provider.
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/ai/embeddings.py` — usar LLMAdapterFactory.get_embedding_adapter. Fallback chain si adapter actual no soporta embeddings.
- MODIFICAR `framework/src/datamirai_engine/core/context.py` — agregar propiedad `llm_adapters: LLMAdapterFactory` al ExecutionContext
- CREAR `app/server/datamirai_app/routes/llm_providers.py` — endpoints CRUD + test + models
- MODIFICAR `app/server/datamirai_app/database.py` — tabla llm_provider_config + migracion
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de llm_providers
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para todos los endpoints de llm-providers
- CREAR `app/web/src/app/settings/providers/page.tsx` — pagina UI de gestion de providers
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — agregar selector provider + modelo para nodos ai/llm_call

---

### 1.2 — Persistencia de LongTermMemory (SQLite + PostgreSQL)

**Problema**: `LongTermMemory` hoy es in-memory (`self._entries: list[dict]`). Se pierde al cerrar la app. El metodo `search()` es substring matching contra summary, tags y decisions — no hay busqueda semantica. Para un agente que debe aprender entre sesiones, esto es inutil. La clase `SharedLog` tiene el mismo problema.

**Solucion**: Implementar `MemoryBackend` interface con dos implementaciones: `SQLiteMemoryBackend` (default para instalacion local, cero config) y `PostgresMemoryBackend` (opt-in cuando el usuario tiene PostgreSQL con pgvector). Ambos soportan busqueda semantica — SQLite via sqlite-vec extension, PostgreSQL via pgvector extension. Un factory selecciona el backend segun configuracion.

**Arquitectura**:
- Interfaz `MemoryBackend` (ABC) con metodos:
  - `save_learning(agent_id, session_id, summary, decisions, learnings, tags) -> str` — retorna memory_id
  - `search(agent_id, query, limit, embedding?) -> list[MemoryEntry]` — busqueda hibrida
  - `get_recent(agent_id, limit) -> list[MemoryEntry]` — ultimas memorias
  - `delete(memory_id) -> bool` — borrar memoria especifica
  - `save_log_entry(agent_id, session_id, message, metadata) -> str` — para SharedLog
  - `get_log(agent_id, limit) -> list[LogEntry]` — leer log
- `MemoryEntry`: `{ id, agent_id, session_id, summary, decisions, learnings, tags, embedding, score?, created_at }`
- `LogEntry`: `{ id, agent_id, session_id, message, metadata, created_at }`
- `SQLiteMemoryBackend`:
  - Tabla `agent_memory` para learnings
  - Tabla `agent_memory_fts` (FTS5 virtual table) para full-text
  - sqlite-vec para vector search (embedding como BLOB serializado)
  - Embeddings se generan al guardar learning usando el LLM adapter de 1.1
- `PostgresMemoryBackend`:
  - Tabla `agent_memory` con columna `embedding vector(dimensions)` (pgvector)
  - pg_trgm para full-text search
  - Indice HNSW para vector search eficiente
- `MemoryBackendFactory`:
  - Si `DATABASE_URL` presente y apunta a PostgreSQL → `PostgresMemoryBackend`
  - Si no → `SQLiteMemoryBackend` (usa misma DB SQLite de la app)
  - Configurable por entorno para testing

**Entidades nuevas**:

Tabla `agent_memory` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id), NOT NULL |
| session_id | TEXT | FK sessions(id), NOT NULL |
| summary | TEXT | NOT NULL |
| decisions | TEXT | JSON array, DEFAULT '[]' |
| learnings | TEXT | JSON array, DEFAULT '[]' |
| tags | TEXT | JSON array, DEFAULT '[]' |
| embedding | BLOB | NULL (vector serializado por sqlite-vec) |
| embedding_model | TEXT | NULL (modelo usado para generar el embedding) |
| created_at | TEXT | NOT NULL |

Virtual table `agent_memory_fts` USING fts5(summary, tags, content=agent_memory, content_rowid=rowid)

Tabla `agent_log` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id), NOT NULL |
| session_id | TEXT | FK sessions(id), NOT NULL |
| message | TEXT | NOT NULL |
| metadata | TEXT | JSON, DEFAULT '{}' |
| created_at | TEXT | NOT NULL |

Tabla `agent_memory` en PostgreSQL (equivalente):

| Columna | Tipo | Constraint |
|---|---|---|
| id | UUID | PK DEFAULT gen_random_uuid() |
| agent_id | UUID | FK agents(id), NOT NULL |
| session_id | UUID | FK sessions(id), NOT NULL |
| summary | TEXT | NOT NULL |
| decisions | JSONB | DEFAULT '[]' |
| learnings | JSONB | DEFAULT '[]' |
| tags | JSONB | DEFAULT '[]' |
| embedding | vector(1536) | NULL (dimension configurable segun modelo) |
| embedding_model | TEXT | NULL |
| created_at | TIMESTAMPTZ | NOT NULL DEFAULT now() |

Indice: `CREATE INDEX idx_agent_memory_embedding ON agent_memory USING hnsw (embedding vector_cosine_ops)`

**Contratos API**:
- `GET /api/agents/{id}/memory` — listar memorias del agente paginado. Query params: `page`, `limit`, `sort` (recent/oldest). Response: `{ memories: MemoryEntry[], total: int, page: int }`
- `GET /api/agents/{id}/memory/search?q={query}` — busqueda hibrida (vector + FTS). Query params: `q` (required), `limit`. Response: `{ results: MemorySearchResult[], query: str }`. Donde `MemorySearchResult` extiende `MemoryEntry` con `{ score, vector_score, fts_score }`
- `GET /api/agents/{id}/memory/{memoryId}` — detalle de una memoria. Response: `{ memory: MemoryEntry }`
- `DELETE /api/agents/{id}/memory/{memoryId}` — borrar memoria especifica. Response: `204`
- `GET /api/agents/{id}/log` — leer log del agente. Query params: `limit`, `session_id?`. Response: `{ entries: LogEntry[] }`

**Pantallas**:
- **AgentDetail → tab "Memoria"** (`/agents/[id]` con tab activo):
  - Lista de memorias del agente ordenadas por fecha (mas recientes primero)
  - Cada memoria muestra: summary (texto principal), tags como badges, fecha relativa
  - Click en memoria expande: decisions, learnings, session de origen
  - Campo de busqueda semantica arriba de la lista — al escribir, reemplaza lista con resultados de busqueda mostrando score
  - Boton para borrar memoria individual (con confirmacion)
  - Indicador de total de memorias y espacio usado
- **AgentDetail → tab "Log"**: lista cronologica de log entries del SharedLog, filtrable por session

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-36 | MemoryBackend es intercambiable. El engine no sabe si es SQLite o Postgres. La interfaz es identica | Factory pattern + ABC |
| REGLA-37 | Embeddings se generan usando el LLM adapter configurado (feature 1.1). Si es Ollama, usa modelo de embeddings local. Si es OpenAI/Claude, usa su API de embeddings | MemoryBackend.save_learning invoca LLMAdapterFactory.get_embedding_adapter |
| REGLA-38 | Si no hay embedding provider configurado o falla, la memoria se guarda SIN embedding. Busqueda cae a FTS puro (keyword). Warning en log, no error | Fallback graceful en save_learning |
| REGLA-39 | Dimension del vector depende del modelo de embedding usado. Se almacena embedding_model junto al vector para saber con que fue generado | Campo embedding_model en tabla |
| REGLA-40 | Migracion de SQLite a PostgreSQL es manual. No hay sync automatico. El usuario exporta e importa | Documentacion, no codigo |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/memory/backend.py` — ABC MemoryBackend + MemoryEntry + LogEntry dataclasses
- CREAR `framework/src/datamirai_engine/memory/sqlite_backend.py` — SQLiteMemoryBackend con sqlite-vec + FTS5
- CREAR `framework/src/datamirai_engine/memory/postgres_backend.py` — PostgresMemoryBackend con pgvector + pg_trgm
- CREAR `framework/src/datamirai_engine/memory/factory.py` — MemoryBackendFactory
- MODIFICAR `framework/src/datamirai_engine/memory/long_term.py` — refactor: LongTermMemory delega a MemoryBackend. Constructor recibe backend. Mantiene firma publica backward-compatible.
- MODIFICAR `framework/src/datamirai_engine/memory/short_term.py` — sin cambios funcionales, solo import del nuevo backend si aplica para transcript persistence
- MODIFICAR `framework/src/datamirai_engine/core/context.py` — MemoryResource protocol actualizado para exponer backend
- MODIFICAR `app/server/datamirai_app/database.py` — tablas agent_memory + agent_memory_fts + agent_log + migracion (SCHEMA_VERSION 3)
- CREAR `app/server/datamirai_app/routes/memory.py` — endpoints de memoria y log
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de memory
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de memoria
- CREAR `app/web/src/app/agents/[id]/memory/page.tsx` — pagina UI de memoria del agente (o tab dentro de agent detail)
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab de memoria y log

---

### 1.3 — Hybrid Search (Vector + FTS)

> **Nota**: Esta feature aplica a **memoria del agente** (learnings, conclusiones). Para busqueda sobre **datos ingestados** (scrapes, análisis, reportes), ver FEAT-020.1 Knowledge Vault que implementa su propia busqueda con wiki-links, tags y backlinks. Ambos sistemas son complementarios.

**Problema**: La busqueda en memoria necesita ser tanto semantica (por significado — "errores de autenticacion" encuentra "fallo de login con OAuth") como por keywords exactos ("TIMEOUT_ERROR" encuentra exactamente ese string). Ninguna sola es suficiente. Vector search pierde keywords exactos. FTS pierde sinonimos y significado.

**Solucion**: Implementar `HybridSearchEngine` que combina vector search (cosine similarity) con full-text search (BM25/FTS5) usando weighted score fusion. El engine es agnostico al backend — funciona igual con SQLite o PostgreSQL.

**Arquitectura**:
- `HybridSearchEngine` con metodo principal: `search(query, collection, limit, weights?) -> list[SearchResult]`
  - Ejecuta vector search y FTS en paralelo
  - Normaliza scores de ambos al rango [0, 1]
  - Fusiona con formula: `score_total = (w_vec * vector_score) + (w_fts * fts_score)`
  - Deduplica resultados (mismo id de ambas fuentes)
  - Ordena por score_total descendente
  - Retorna top N
- `SearchResult`: `{ id: str, content: str, score: float, vector_score: float, fts_score: float, metadata: dict }`
- Weights default: `w_vec=0.7, w_fts=0.3` (configurable por llamada)
- `VectorSearchProvider` (ABC): abstraccion sobre sqlite-vec y pgvector
  - `SQLiteVectorProvider` — usa sqlite-vec extension, cosine distance
  - `PostgresVectorProvider` — usa pgvector, operador `<=>` para cosine distance
- `FTSProvider` (ABC): abstraccion sobre FTS5 y pg_trgm
  - `SQLiteFTSProvider` — usa FTS5 con BM25 ranking
  - `PostgresFTSProvider` — usa to_tsvector/to_tsquery con ts_rank
- Fallback chain:
  1. Si hay embedding del query + hay embeddings almacenados → hybrid (vector + FTS)
  2. Si no hay embedding del query (no hay provider de embeddings) → FTS puro
  3. Si no hay FTS index → substring match basico (degradacion maxima, siempre funciona)

**Se integra con 1.2**: `HybridSearchEngine` es usado por `SQLiteMemoryBackend` y `PostgresMemoryBackend` para implementar el metodo `search()`. Los backends instancian el engine con sus providers concretos.

**Entidades**: No crea tablas nuevas. Usa las tablas de 1.2 (`agent_memory` + `agent_memory_fts` + embeddings). Los indices necesarios ya estan definidos en 1.2.

**Contratos API**: No expone endpoints propios. Es un componente interno del framework usado por los MemoryBackends. La busqueda se expone via el endpoint `GET /api/agents/{id}/memory/search` definido en 1.2.

**Pantallas**: No tiene pantallas propias. Los resultados de busqueda hibrida se ven en el tab "Memoria" de AgentDetail (definido en 1.2), donde cada resultado muestra su score desglosado (vector_score + fts_score) para transparencia.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-41 | HybridSearchEngine nunca falla. Si vector search falla, retorna solo FTS. Si FTS falla, retorna solo vector. Si ambos fallan, retorna lista vacia | Try/except por provider con fallback |
| REGLA-42 | Scores siempre normalizados a [0, 1] antes de fusion. Vector: cosine similarity ya esta en [0,1]. FTS: normalizar dividiendo por max score del batch | Normalizacion en merge step |
| REGLA-43 | Deduplicacion por id. Si un resultado aparece en ambas fuentes, se fusionan scores (no se duplica) | Set-based merge por id |
| REGLA-44 | Weights son configurables por llamada pero tienen defaults sensatos (0.7 vec, 0.3 fts). Si se pasan weights que no suman 1.0, se normalizan automaticamente | Normalizacion en constructor |
| REGLA-45 | El query embedding se genera una sola vez y se reutiliza. No se genera embedding para cada comparacion | Pre-compute en search() antes de delegar |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/search/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/search/hybrid.py` — HybridSearchEngine + SearchResult dataclass
- CREAR `framework/src/datamirai_engine/search/vector.py` — VectorSearchProvider ABC + SQLiteVectorProvider + PostgresVectorProvider
- CREAR `framework/src/datamirai_engine/search/fts.py` — FTSProvider ABC + SQLiteFTSProvider + PostgresFTSProvider
- MODIFICAR `framework/src/datamirai_engine/memory/sqlite_backend.py` — instanciar HybridSearchEngine con SQLiteVectorProvider + SQLiteFTSProvider en search()
- MODIFICAR `framework/src/datamirai_engine/memory/postgres_backend.py` — instanciar HybridSearchEngine con PostgresVectorProvider + PostgresFTSProvider en search()

---

## Dependencias entre features

```
1.1 (Multi-LLM) ← independiente, se implementa primero
1.2 (Persistencia) ← depende de 1.1 para generar embeddings al guardar memorias
1.3 (Hybrid Search) ← depende de 1.2 para las tablas, indices y backends
```

Orden de implementacion: 1.1 → 1.2 → 1.3

Dentro de 1.1, orden sugerido de adapters: OllamaAdapter (ya hay infra local) → OpenAIAdapter (mas comun en cloud) → ClaudeAdapter → GeminiAdapter → GroqAdapter → OpenRouterAdapter.

---

## Entidades nuevas (resumen consolidado)

### llm_provider_config
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| provider | TEXT | NOT NULL (ollama / claude / openai / gemini / groq / openrouter) |
| display_name | TEXT | NOT NULL |
| api_key_credential_id | TEXT | FK credential, NULL |
| base_url | TEXT | NULL |
| default_model | TEXT | NOT NULL |
| embedding_model | TEXT | NULL |
| is_default | BOOLEAN | DEFAULT false |
| max_timeout_seconds | INTEGER | DEFAULT 60 |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### agent_memory
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id), NOT NULL |
| session_id | TEXT | FK sessions(id), NOT NULL |
| summary | TEXT | NOT NULL |
| decisions | TEXT | JSON array, DEFAULT '[]' |
| learnings | TEXT | JSON array, DEFAULT '[]' |
| tags | TEXT | JSON array, DEFAULT '[]' |
| embedding | BLOB | NULL |
| embedding_model | TEXT | NULL |
| created_at | TEXT | NOT NULL |

### agent_memory_fts (virtual)
FTS5 sobre (summary, tags) de agent_memory.

### agent_log
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id), NOT NULL |
| session_id | TEXT | FK sessions(id), NOT NULL |
| message | TEXT | NOT NULL |
| metadata | TEXT | JSON, DEFAULT '{}' |
| created_at | TEXT | NOT NULL |

---

## Maquinas de estado

No se introducen nuevas maquinas de estado. Las entidades de este PRD son datos pasivos (configuraciones, memorias, logs) sin transiciones de estado complejas.

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-30 | Todo adapter retorna NormalizedResponse. Nunca datos raw del provider | Adapter base class |
| REGLA-31 | API keys nunca en logs ni responses. Se referencian via credential_id | Serializer + vault |
| REGLA-32 | Provider timeout configurable (default 60s). Nodo falla con error descriptivo | Adapter base con asyncio.wait_for |
| REGLA-33 | OllamaAdapter SIEMPRE strip `<think>` tags | OllamaAdapter._clean_response |
| REGLA-34 | Exactamente 1 provider is_default=true | Service transaccion atomica |
| REGLA-35 | Adapters sin embeddings lanzan NotImplementedError. Sistema cae a fallback | LLMAdapterFactory |
| REGLA-36 | MemoryBackend intercambiable. Engine agnostico a SQLite vs Postgres | Factory + ABC |
| REGLA-37 | Embeddings generados via LLM adapter de 1.1 | MemoryBackend.save_learning |
| REGLA-38 | Sin embedding provider → memoria se guarda sin vector. Busqueda cae a FTS | Fallback graceful |
| REGLA-39 | Dimension de vector depende del modelo. Se almacena embedding_model | Campo en tabla |
| REGLA-40 | Migracion SQLite→Postgres es manual | Documentacion |
| REGLA-41 | HybridSearch nunca falla. Fallback chain: hybrid → FTS → substring | Try/except |
| REGLA-42 | Scores normalizados a [0,1] antes de fusion | Normalizacion en merge |
| REGLA-43 | Deduplicacion por id en resultados hibridos | Set-based merge |
| REGLA-44 | Weights configurables, se normalizan si no suman 1.0 | Constructor |
| REGLA-45 | Query embedding se genera una sola vez por busqueda | Pre-compute |

---

## Notas de implementacion

- **SQLite es el default**. PostgreSQL es opt-in via `DATABASE_URL`. El usuario promedio nunca necesita Postgres para desarrollo local.
- **sqlite-vec** se instala como dependencia del framework (`pip install sqlite-vec`). Es una extension de SQLite que soporta operaciones vectoriales eficientes. Si la extension no esta disponible, el sistema funciona sin vector search (solo FTS).
- **Embeddings se generan con el LLM provider configurado**. Si es Ollama, usa modelo de embeddings local (nomic-embed-text, all-minilm, etc). Si es OpenAI, usa text-embedding-3-small. Si es Claude, usa Voyage API o fallback a otro provider.
- **Migracion SQLite→Postgres** es responsabilidad del usuario (export JSON, import en Postgres). No hay sync automatico entre backends.
- **LLM SDKs como dependencias opcionales**. El framework base solo requiere `httpx` para Ollama (HTTP directo). Los SDKs de Claude (`anthropic`), OpenAI (`openai`), Gemini (`google-genai`), Groq (`groq`) son extras opcionales: `pip install datamirai-engine[claude]`, `pip install datamirai-engine[openai]`, etc. Si el usuario configura un provider cuyo SDK no esta instalado, error descriptivo con instrucciones de instalacion.
- **Backward compatibility**: `LongTermMemory` mantiene su API publica actual (`save_learning`, `search`, `get_recent`). El constructor acepta opcionalmente un `MemoryBackend`. Si no se pasa, usa `InMemoryBackend` (comportamiento actual) para no romper tests existentes.
- **FTS5 triggers**: Se necesitan triggers de SQLite para mantener `agent_memory_fts` sincronizado con `agent_memory` en INSERT/UPDATE/DELETE.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-001.md` — MVP features (context: vault para credenciales, streaming para providers)
