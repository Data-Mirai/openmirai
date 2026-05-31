<!--
BLUEPRINT SEED — SCHEMA.md
Responsable: → blueprint/agents/03-ENTITIES.md

Estructura esperada:
1. Entidades (por entidad: campos, relaciones, índices, RLS, notas N+1)
2. Enums (derivados de FLUJOS.md cuando hay máquinas de estado)

Reglas:
- NO SQL literal (CREATE TABLE, ALTER, etc.). Todo declarativo en tablas markdown
- Enums derivan de FLUJOS.md — si FLUJOS cambia, enum cambia
- Referencias: → DOMINIO.md#rol-X y → FLUJOS.md#maquina-X
- RLS: operación + rol + condición (si el DB engine lo soporta; si no, policy en código)
- Índices y notas N+1 se agregan durante optimización, no al crear
- PKs siempre UUID (gen_random_uuid() o equivalente según engine)
- No DELETE físico — soft delete via campo status
-->

# SCHEMA.md

## Storage Overview

OpenMirai uses **bundled SQLite (rusqlite)** with WAL (Write-Ahead Logging) for concurrent reads. No external database, no Docker, no Postgres.

**Key Features:**
- All tables use `IF NOT EXISTS` for idempotent migrations (schema version v1)
- WAL mode enabled for better concurrency (REGLA-504)
- Foreign keys enabled with `ON DELETE CASCADE`
- User-defined tables created dynamically via `data/db_write` tool

---

## 1. Core Engine Tables

### graphs {#entidad-graphs}

Stores YAML graph definitions executed by the engine.

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| id | TEXT | No | _(required)_ | PK: graph identifier |
| name | TEXT | No | _(required)_ | Human-readable graph name |
| version | TEXT | No | `'1.0'` | Semantic version of the graph |
| nodes | TEXT | No | `'[]'` | JSON array of node definitions (NodeDef) |
| edges | TEXT | No | `'[]'` | JSON array of edge definitions (source → target) |
| metadata | TEXT | No | `'{}'` | JSON object: custom graph metadata |
| created_at | REAL | No | _(required)_ | Unix timestamp (seconds) of creation |
| updated_at | REAL | No | _(required)_ | Unix timestamp (seconds) of last update |

**Relaciones:**
| Campo FK | Entidad relacionada | Tipo | Descripción |
|---|---|---|---|
| — | agents | 1:N | A graph may be referenced by many agents |

**Índices:**
- _(None at v1 — optimized during performance tuning phase)_

**Notas:**
- Each row holds the complete node/edge list as JSON; no separate node or edge tables.

---

### agents {#entidad-agents}

Represents a managed or scheduled agent wrapping a graph and adding triggers/scheduling.

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| id | TEXT | No | _(required)_ | PK: agent identifier (UUID format) |
| name | TEXT | No | _(required)_ | Human-readable agent name |
| graph_id | TEXT | No | _(required)_ | FK → graphs.id; `ON DELETE CASCADE` |
| status | TEXT | No | `'disabled'` | Current agent state: `'enabled'` \| `'disabled'` (→ AgentStatus enum) |
| triggers | TEXT | No | `'[]'` | JSON array of trigger definitions (webhook, schedule, etc.) |
| metadata | TEXT | No | `'{}'` | JSON object: custom agent metadata |
| created_at | REAL | No | _(required)_ | Unix timestamp (seconds) of creation |
| updated_at | REAL | No | _(required)_ | Unix timestamp (seconds) of last update |

**Índices:**
| Nombre | Columns | Propósito |
|---|---|---|
| idx_agents_status | status | Fast filter by agent status (enabled/disabled) |

**Notas:**
- Agent state machine is: `disabled` ↔ `enabled` (two-state)
- `triggers` can include webhook URLs, cron expressions, event subscriptions

---

### sessions {#entidad-sessions}

Persists execution records of agent runs (a single session = one graph execution).

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| id | TEXT | No | _(required)_ | PK: session identifier (UUID format) |
| agent_id | TEXT | No | _(required)_ | FK → agents.id; `ON DELETE CASCADE` |
| agent_name | TEXT | No | _(required)_ | Denormalized copy of agent name at session start |
| graph_id | TEXT | No | _(required)_ | Copy of graph_id for convenience (not a FK) |
| status | TEXT | No | `'running'` | Session state: `'running'` \| `'completed'` \| `'failed'` (→ SessionStatus enum) |
| trace | TEXT | No | `'[]'` | JSON array of execution trace entries (node visits, tool calls) |
| transcript | TEXT | No | `'[]'` | JSON array of LLM messages (prompt/response pairs) |
| state | TEXT | No | `'{}'` | JSON object: final execution state (node outputs) |
| error | TEXT | Yes | NULL | Error message if status = `'failed'` |
| started_at | REAL | No | _(required)_ | Unix timestamp (seconds) of session start |
| finished_at | REAL | Yes | NULL | Unix timestamp (seconds) of session completion |
| duration_ms | REAL | Yes | NULL | Total execution duration in milliseconds |

**Índices:**
| Nombre | Columns | Propósito |
|---|---|---|
| idx_sessions_agent | agent_id | Filter sessions by agent |
| idx_sessions_started | started_at DESC | Sort sessions by recency |
| idx_sessions_status | status | Filter by running/completed/failed |

**Notas:**
- `trace` and `transcript` grow with graph complexity; no truncation at v1
- `state` contains all node outputs for debugging/audit
- No separate event/transcript tables; all ephemera stored in JSON for now

---

### _schema_version {#entidad-schema_version}

Tracks which DDL version has been applied to this database.

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| version | INTEGER | No | _(required)_ | PK: schema version number (1-based) |
| applied_at | REAL | No | _(required)_ | Unix timestamp (seconds) when this version was applied |

**Notas:**
- Exactly one row per applied migration
- Current version = 1 (initial standalone engine schema)
- Used by migrations.rs to decide which DDL to run on startup

---

## 2. Memory Backend Tables

### long_term_memory {#entidad-long_term_memory}

Persistent agent memory: facts, observations, reflections, and lessons learned (with FTS5).

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| id | TEXT | No | _(required)_ | PK: entry identifier (UUID) |
| entry_type | TEXT | No | _(required)_ | Type of memory: `'decision'` \| `'learning'` \| `'pattern'` \| `'error_resolution'` |
| content | TEXT | No | _(required)_ | Main text of the memory entry |
| tags | TEXT | No | `'[]'` | JSON array of tag strings for filtering |
| session_id | TEXT | Yes | NULL | FK (informational): originating session, if any |
| created_at | REAL | No | _(required)_ | Unix timestamp (seconds) when memory was stored |
| metadata | TEXT | No | `'{}'` | JSON object: custom metadata (confidence, source, etc.) |

**Índices:**
- **memory_fts** (virtual FTS5 table)
  - Columns indexed: `content`, `tags`, `entry_type`
  - Content table: long_term_memory
  - Enables full-text search via `WHERE memory_fts MATCH 'query'`

**Triggers:**
- **memory_ai**: After INSERT → auto-populate memory_fts
- **memory_ad**: After DELETE → auto-remove from memory_fts

**Notas:**
- FTS5 allows substring and phrase search ("learning*", "\"best practice\"")
- `entry_type` derives from agent's memory reflection logic (memory/long_term.rs)
- Search results ordered by `created_at DESC` (newest first)
- No UPDATE trigger on memory_fts; updates must be delete + re-insert

---

### memory_fts {#entidad-memory_fts}

Virtual FTS5 table for full-text search over `long_term_memory`.

**Columns (FTS5):**
| Columna | Descripción |
|---|---|
| content | Searchable memory content |
| tags | Tag strings (for filtering by tag) |
| entry_type | Entry type (for filtering by type) |

**Notas:**
- Virtual table; no direct writes (triggers handle sync)
- Supports FTS5 operators: `AND`, `OR`, phrase quotes, prefix `*`

---

## 3. Vector Store / Search Tables

### vector_docs {#entidad-vector_docs}

Stores documents for text-based search (not true vectors; FTS5-based).

**Campos:**
| Campo | Tipo | Nullable | Default | Descripción |
|---|---|---|---|---|
| id | TEXT | No | _(required)_ | PK: document identifier |
| text | TEXT | No | _(required)_ | Document content (searchable) |
| metadata | TEXT | No | `'{}'` | JSON object: custom metadata (source, title, url, etc.) |

**Índices:**
- **vector_fts** (virtual FTS5 table)
  - Columns indexed: `text`, `id` (unindexed, for retrieval)
  - Content table: vector_docs
  - Enables full-text search on document text

**Triggers:**
- **vdocs_ai**: After INSERT → auto-populate vector_fts
- **vdocs_ad**: After DELETE → auto-remove from vector_fts
- **vdocs_au**: After UPDATE → auto-refresh vector_fts

**Notas:**
- Used by `search/hybrid` nodes for RAG and general text search
- Search returns top-k by FTS5 rank (normalized to 0–1 similarity score)
- Useful for Wikipedia-like indexed content, blog posts, docs

---

### vector_fts {#entidad-vector_fts}

Virtual FTS5 table for full-text search over `vector_docs`.

**Columns (FTS5):**
| Columna | Descripción |
|---|---|
| text | Searchable document text |
| id | Document ID (unindexed for direct retrieval) |

---

## 4. User-Defined Tables (Dynamic)

The `data/db_write` tool allows graphs to write arbitrary data to custom tables on the fly. The engine automatically creates tables as needed and infers schema from the first write.

**Behavior:**
- Table is created on first insert with columns matching the data object keys
- Column types are inferred from the JSON value types (string → TEXT, number → REAL, boolean → INTEGER, object → TEXT as JSON)
- Subsequent writes to the same table must have compatible column types
- No automatic system columns; users define their data structure freely
- Write modes: `insert` (default) or `upsert` (via `mode` config field)

### Configuration

The `data/db_write` node accepts:
- **table** (string, required): Target table name
- **mode** (string, optional): Write mode; `'insert'` or `'upsert'` (default: `'insert'`)

### Example: Custom `events` Table

A graph with a `data/db_write` node configured as:
```yaml
config:
  table: events
  mode: upsert
```

With input:
```json
{
  "data": {
    "title": "API call failed",
    "count": 3,
    "payload": {"status": 500}
  }
}
```

Results in a created table `events` (if not exists) with columns:
```
id (TEXT) | title (TEXT) | count (REAL) | payload (TEXT)
```

**Type Mapping (SQLite):**
| JSON Type | SQLite Type | Example |
|---|---|---|
| string | TEXT | `"hello"` → TEXT |
| number (int) | REAL | `42` → REAL |
| number (float) | REAL | `3.14` → REAL |
| boolean | INTEGER | `true` → 1, `false` → 0 |
| object | TEXT | `{"a": 1}` → TEXT (JSON string) |
| null | NULL | omit or NULL |

**Notas:**
- No explicit schema definition before insert (schema-on-write)
- Type inference is per-column on first non-null value
- ID generation handled by the engine (UUID)

---

## 5. Enums {#enums}

### AgentStatus {#enum-agent_status}

**Usado en:** → agents.status  
**Fuente:** Agent lifecycle (agents table)

| Valor | Descripción |
|---|---|
| enabled | Agent is active and can be triggered |
| disabled | Agent is paused; will not respond to triggers |

---

### SessionStatus {#enum-session_status}

**Usado en:** → sessions.status  
**Fuente:** Execution lifecycle (sessions table)

| Valor | Descripción |
|---|---|
| running | Session is currently executing |
| completed | Session finished successfully |
| failed | Session encountered a fatal error |

---

### MemoryEntryType {#enum-memory_entry_type}

**Usado en:** → long_term_memory.entry_type  
**Fuente:** Agent reflection logic (memory/long_term.rs)

| Valor | Descripción |
|---|---|
| decision | Decision made by the agent during execution |
| learning | Fact or insight learned from execution |
| pattern | Recurring pattern or best practice identified |
| error_resolution | Successful resolution of an error or failure |

---

## 6. Constraints & Integrity

### Foreign Keys

- agents.graph_id → graphs.id with `ON DELETE CASCADE`
  - Deleting a graph removes all its agents
- sessions.agent_id → agents.id with `ON DELETE CASCADE`
  - Deleting an agent removes all its sessions

### Soft Delete

Engine does not use physical `DELETE` in migration-managed tables (graphs, agents, sessions). Future versions may implement soft delete via a `status` field set to `'deleted'` rather than physical row removal.

---

## 7. WAL & Concurrency

SQLite is initialized with:
- Core DB (`sqlite_db.rs`):
```
PRAGMA journal_mode=WAL;        -- Write-Ahead Logging (concurrent reads)
PRAGMA busy_timeout=10000;       -- Wait up to 10s for locks
PRAGMA foreign_keys=ON;          -- Enforce referential integrity
```

- Memory backend (`memory/sqlite_backend.rs`):
```
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
```

- Vector store (`simple_vector.rs`):
```
PRAGMA journal_mode=WAL;
```

**Implications:**
- Multiple readers can proceed concurrently
- Writers serialize but do not block readers (WAL advantage)
- Temporary `.wal` and `.shm` files appear in the database directory

---

## 8. Migration Strategy

**Current state:** Schema v1, single forward migration.

**Migration file:** engine/src/db/migrations.rs

**Flow:**
1. On engine startup, read `_schema_version`
2. Apply all migrations with version > current
3. Update `_schema_version` with new version and timestamp
4. All DDL uses `IF NOT EXISTS` for idempotency

**Adding a new table:**
1. Increment `SCHEMA_VERSION` in migrations.rs
2. Create a new `Migration` struct with updated DDL
3. Push to `MIGRATIONS` array (must be ordered by version)
4. Deploy; migration runs automatically on next engine start

---

## 9. Performance Notes

**N+1 Risks (pre-optimization):**
- sessions.trace and sessions.transcript stored as JSON; no separate indexing
  - Mitigation: queries do not filter on trace/transcript content (yet)
  - If needed: consider JSON1 functions or extract to separate tables

**Unindexed Searches (pre-optimization):**
- Querying by graph_id: not indexed (agents.graph_id has no index, only sessions references)
- Mitigation: `ON DELETE CASCADE` keeps referential integrity; add index if frequent

**Recommended Indices (future phase):**
- agents(graph_id) — for "list all agents for this graph"
- long_term_memory(session_id) — for "recall memories from a specific session"
- vector_docs metadata filtering (if metadata becomes searchable)
