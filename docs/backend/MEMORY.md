# MEMORY.md

OpenMirai has **several deliberately separate memory layers**, not one unified store. They differ in
purpose, durability, and which process owns them. The biggest source of confusion is that the word
"memory" is overloaded across three engine subsystems plus a CLI transcript store — and the layer that
the HTTP `/memory` endpoints expose is **not** the same as the long-term memory subsystem in
`engine/src/memory/`. This document maps each layer, what it persists, where it lives, and — importantly —
which layers are wired into the live execution path versus implemented-but-not-yet-invoked.

| Layer | File | Backend | Durable? | Process scope | Wired into live path? |
|-------|------|---------|----------|---------------|------------------------|
| Short-term memory | `engine/src/memory/short_term.rs` | `Vec` in RAM | No | Engine, per-execution | Component only (not auto-invoked) |
| Long-term memory (SQLite) | `engine/src/memory/sqlite_backend.rs` | SQLite + FTS5 file | **Yes** (on disk) | Engine | **No** — no live callers yet |
| Long-term memory (in-mem) | `engine/src/memory/in_memory_backend.rs` | `Arc<RwLock<Vec>>` | No | Engine | **No** — no live callers yet |
| Agent KV memory | `engine/src/runtime/agent_memory_store.rs` | `HashMap` in RAM | No (resets on restart) | Engine, per-agent | **Yes** — `/api/v1/agents/{id}/memory` |
| Engine session state | `engine/src/server/state.rs` | `HashMap` in RAM | No (FIFO-evicted) | Engine HTTP server | **Yes** — `/api/v1/sessions` |
| CLI session transcripts | `cli/src/session_storage.rs` | JSON + JSONL files | **Yes** (on disk) | CLI only | **Yes** — CLI chat sessions |

> **Read this first:** the long-term memory subsystem (`LongTermMemory`, `SqliteBackend`, `InMemoryBackend`,
> `MemoryFactory`) and the `MemoryFlusher` that is meant to populate it are fully implemented and unit-tested,
> but they currently have **no callers in the running server or CLI**. They are available building blocks, not
> an active behavior of a running OpenMirai today. See [Wiring status](#wiring-status).

---

## Memory model overview

There are three engine-side concepts and one CLI-side concept:

1. **Short-term memory** — an in-RAM trace of a single graph execution (logs, decisions, per-block metrics).
   Lives and dies with the execution.
2. **Long-term memory** — persistent learning across executions, behind a pluggable `MemoryBackend` trait with
   two implementations (SQLite-with-FTS5, and a RAM-only backend for dev/testing). Selected by `MemoryFactory`.
3. **Agent KV memory** — a per-agent key/value store (`AgentMemoryStore`) used by the live agent "play" feature,
   with an execution-scoped tier and a cycle-scoped (volatile) tier. RAM-only in V1.
4. **CLI session transcripts** — durable on-disk conversation logs the CLI writes for its interactive chat
   sessions. The engine never reads these; they are a CLI concern only.

These do not share storage. In particular, engine memory and CLI transcripts live in different processes with
different durability guarantees — see [CLI session storage vs engine memory](#cli-session-storage-vs-engine-memory).

---

## Short-term memory

**File:** `engine/src/memory/short_term.rs`

`ShortTermMemory` captures the step-by-step detail of the **current** graph execution. It is a plain
`Vec<ShortTermEntry>` held in RAM — there is no persistence and no backend. When the owning value is dropped
(execution ends), the trace is gone.

**`ShortTermEntry` fields:**

| Field | Type | Description |
|-------|------|-------------|
| `timestamp` | `f64` | Unix seconds (fractional) when the entry was recorded |
| `entry_type` | `String` | `"log"`, `"decision"`, or `"metric"` |
| `node_id` | `Option<String>` | Block/node that produced the entry, if applicable |
| `content` | `String` | Human-readable content |
| `metadata` | `HashMap<String, Value>` | Arbitrary extra data |

**API:** `add(entry_type, content, node_id, metadata)` to record; `get_all()` returns all entries in insertion
order; `get_by_type(t)` and `get_by_node(id)` filter; `clear()` empties; `len()` / `is_empty()` for size.

**Durability:** process-local RAM only.

---

## Long-term memory

**File:** `engine/src/memory/long_term.rs`

Long-term memory persists learnings/decisions/patterns/error-resolutions across executions. It is a thin facade
(`LongTermMemory`) over a pluggable backend (`MemoryBackend` trait), so the storage engine can be swapped without
changing callers.

**`LongTermEntry` fields:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | UUID — **assigned by the backend** on save (callers pass an empty string) |
| `entry_type` | `String` | `"decision"`, `"learning"`, `"pattern"`, or `"error_resolution"` |
| `content` | `String` | Human-readable content |
| `tags` | `Vec<String>` | Searchable tags |
| `session_id` | `Option<String>` | Session that produced the entry, if applicable |
| `created_at` | `f64` | Unix seconds; if left at `0.0`, the backend stamps it with the current wall clock |
| `metadata` | `HashMap<String, Value>` | Arbitrary extra data |

**`MemoryBackend` trait** (all methods `async`, fallible with `RunnerError`):

| Method | Returns | Purpose |
|--------|---------|---------|
| `save(&entry)` | `String` (assigned id) | Persist an entry |
| `search(query, limit)` | `Vec<LongTermEntry>` | Full-text search, up to `limit` matches |
| `get(id)` | `Option<LongTermEntry>` | Look up one entry by id |
| `delete(id)` | `()` | Delete by id (idempotent) |
| `list_recent(limit)` | `Vec<LongTermEntry>` | Most recent entries, newest first |

`LongTermMemory::new(Box<dyn MemoryBackend>)` wraps a backend and delegates every call to it.

---

## SQLite backend behavior

**File:** `engine/src/memory/sqlite_backend.rs`

`SqliteBackend` is the durable `MemoryBackend`, backed by `rusqlite` with an FTS5 full-text index. It is the
only memory layer (other than CLI transcripts) that writes to disk.

**Schema** (created on open via `CREATE ... IF NOT EXISTS`):

```sql
CREATE TABLE IF NOT EXISTS long_term_memory (
    id TEXT PRIMARY KEY,
    entry_type TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    session_id TEXT,
    created_at REAL NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content, tags, entry_type,
    content='long_term_memory',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS memory_ai AFTER INSERT ON long_term_memory BEGIN
    INSERT INTO memory_fts(rowid, content, tags, entry_type)
    VALUES (new.rowid, new.content, new.tags, new.entry_type);
END;

CREATE TRIGGER IF NOT EXISTS memory_ad AFTER DELETE ON long_term_memory BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, tags, entry_type)
    VALUES ('delete', old.rowid, old.content, old.tags, old.entry_type);
END;
```

**Behavior notes:**

- **Construction:** `SqliteBackend::new(path)` opens (or creates) the file, sets `PRAGMA journal_mode=WAL` and
  `PRAGMA foreign_keys=ON`, then applies the schema. `new_in_memory()` opens `:memory:` (used by tests).
- **`tags` and `metadata`** are stored as serialized JSON strings (`'[]'` / `'{}'` defaults), deserialized on read.
- **FTS sync** is automatic via the `memory_ai` / `memory_ad` triggers; there is no separate index-rebuild step.
- **Async safety:** `rusqlite` is synchronous, so every DB call runs inside `tokio::task::spawn_blocking`. The
  connection is held behind `Arc<Mutex<Connection>>`; the wrapper carries an `unsafe impl Send + Sync` justified by
  that mutex guard.
- **`save`** assigns a fresh UUID, defaults `created_at` to now when the caller passed `0.0`, and returns the id.
- **`get` / `delete`** operate by id; deleting a missing id is not an error.
- This schema is **not** documented in [SCHEMA.md](../database/SCHEMA.md) (which covers the declarative `sessions`
  table); the memory schema lives here and is created at runtime by this backend.

**Durability:** persistent on disk at the configured path.

---

## In-memory backend behavior

**File:** `engine/src/memory/in_memory_backend.rs`

`InMemoryBackend` is the default `MemoryBackend` for development and testing. It stores entries in an
`Arc<RwLock<Vec<LongTermEntry>>>`, so it is cheap to clone and safe to share across async tasks.

- **`save`** assigns a UUID and defaults `created_at` to now when `0.0`, then pushes the entry.
- **`search`** is a case-insensitive substring (`contains`) match on `content` (not FTS), sorted newest-first and
  truncated to `limit`.
- **`get` / `delete`** are by id; delete is idempotent.
- **`list_recent`** returns all entries sorted newest-first, truncated to `limit`.

**Durability:** process-local RAM only — everything is lost on restart.

---

## Factory / provider selection

**File:** `engine/src/memory/factory.rs`

`MemoryFactory` chooses the backend from an optional database path:

```rust
MemoryFactory::create(Some(path))  // → SqliteBackend (persistent)
MemoryFactory::create(None)        // → InMemoryBackend (no persistence)
MemoryFactory::in_memory()         // → InMemoryBackend (convenience)
```

The contract is simple: a `Some(path)` selects durable SQLite; `None` selects the RAM backend. There is no
environment-variable or config-file resolution layer here — selection is purely by the argument passed in.

> **Wiring caveat:** `MemoryFactory::create` / `in_memory` currently have **no callers** outside this module's own
> tests. Nothing in the running server or CLI constructs a long-term memory backend yet. See [Wiring status](#wiring-status).

---

## Agent KV memory store

**File:** `engine/src/runtime/agent_memory_store.rs` (PRD-008)

`AgentMemoryStore` is a per-agent key/value store used by the live agent **play** feature. It is **distinct from the
long-term memory subsystem above** and is the layer the HTTP `/memory` endpoints actually expose.

It maintains two maps, each `Arc<RwLock<HashMap<agent_id, HashMap<String, Value>>>>`:

- **Execution memory** (`execution_store`) — intended to persist across executions of an agent.
- **Cycle memory** (`cycle_store`) — volatile, scoped to a single play session; cleared when a new play session starts.

Writes are **filtered to the agent's declared memory keys** (keys not declared in the agent spec are dropped).

**Durability:** RAM only. The module states it explicitly — *"V1: in-memory only — resets on server restart."* Even
"execution" memory does not survive a server restart; it only outlives individual executions within one server process.

---

## Engine session state

**File:** `engine/src/server/state.rs`

The HTTP server keeps finished execution results in `state.sessions`, an `Arc<RwLock<HashMap<String, ExecutionResult>>>`.
A companion `session_order` vector enforces **FIFO eviction** once `DEFAULT_MAX_SESSIONS` (10,000) is exceeded.

**Durability:** process-local RAM. Sessions exist only for the lifetime of the server process and are evicted oldest-first
at the cap. (Note: `SCHEMA.md` describes a declarative `sessions` table, but the live server does not persist sessions to it.)

---

## CLI session storage vs engine memory

**File:** `cli/src/session_storage.rs`

This is the **only** durable, disk-backed conversation store in the live product, and it belongs entirely to the **CLI**.
The engine never reads or writes it.

**Layout:** each session lives under `~/.datamirai/sessions/<session_id>/` (the `.datamirai` directory name is kept for
backward compatibility):

- `manifest.json` — metadata: `id`, `provider`, `model`, `cwd`, `created_at`, `updated_at`, `message_count`,
  `checkpoint_count`, `status` (`"active"` | `"closed"`).
- `transcript.jsonl` — append-only log; one JSON object per line.

**Transcript entries** carry `ts`, `role`, optional `content`, and flattened `metadata`. Roles include `user`,
`assistant`, `tool_call`, `tool_result`, and `checkpoint`. Notable behaviors:

- **Tool results are truncated** to 5000 characters (with a `...(truncated)` suffix) before being written.
- **Checkpoints are transcript entries**, not separate snapshots. `create_checkpoint` appends a `checkpoint`-role entry
  recording a `message_index`; `list_checkpoints` reconstructs them by scanning the transcript.
- **`rebuild_messages`** reconstructs an LLM message history by replaying only `user` / `assistant` / `system` entries.
- Appending bumps `updated_at` and `message_count`; `list_sessions` reads manifests sorted by recency; `close_session`
  flips `status` to `"closed"`.

### Engine memory ≠ CLI transcripts

| | Engine memory | CLI session transcripts |
|---|---|---|
| Owning process | Engine (HTTP server / runtime) | CLI binary |
| What it stores | Execution traces, agent KV, learnings | Full chat conversation + tool I/O |
| Durability | RAM (or SQLite for long-term, when wired) | Files on disk (`~/.datamirai/sessions/`) |
| Shared store? | No | No — engine never reads these files |

The two never share state. A "session" in the engine HTTP API (`/api/v1/sessions`) is a one-graph-execution record in
RAM; a "session" in the CLI is a durable interactive conversation on disk. They are different things with the same name.

---

## Search semantics

| Layer | Search mechanism | Ordering |
|-------|------------------|----------|
| Long-term (SQLite) | FTS5 `MATCH`; each whitespace token becomes `"token"*` (prefix match). Empty/blank query → empty result. | `created_at DESC` |
| Long-term (in-mem) | Case-insensitive substring (`content.contains`) | `created_at DESC` |
| Short-term | No text search — filter by `entry_type` or `node_id` only | insertion order |
| CLI transcript | No index — linear read of `transcript.jsonl` | append order |

The intended **writer** of long-term memory is the `MemoryFlusher` (`engine/src/intelligence/memory_flusher.rs`): it
extracts error-resolution and pattern entries from an execution trace and saves them to a `LongTermMemory`. It triggers
when accumulated context reaches a threshold (default 0.75 of max context), at most once per session (REGLA-58). See the
wiring caveat below — the flusher is implemented and tested but not currently invoked in the live path.

---

## API endpoints that expose memory/session data

Cross-reference: [API.md](API.md). They require `X-API-Key` when server
authentication is configured.

| Endpoint | Exposes | Backed by |
|----------|---------|-----------|
| `GET /api/v1/agents/{id}/memory` | Current agent KV memory | `AgentMemoryStore` (RAM) |
| `DELETE /api/v1/agents/{id}/memory` | Reset agent memory to initial values (409 if agent is playing) | `AgentMemoryStore` (RAM) |
| `GET /api/v1/sessions` | List execution sessions (`agent_id`, `limit` query params) | `state.sessions` (RAM, FIFO-evicted) |
| `GET /api/v1/sessions/{id}` | Full session detail incl. execution `trace` (404 if missing) | `state.sessions` (RAM) |
| `GET /api/v1/metrics` | Aggregated counts/metrics over stored sessions | `state.sessions` (RAM) |

`GET /api/v1/agents/{id}/memory` returns `{ "agent_id", "memory": { ...declared keys... } }`. The session endpoints return
RAM records (`id`, `status`, `trace`/`trace_len`, `error`).

> There is **no endpoint that exposes the long-term `SqliteBackend`** — the long-term memory subsystem is not reachable
> over HTTP today. The `/memory` endpoints serve the agent KV store, not long-term learnings.

---

## Lifecycle (text diagram)

**Engine execution path (long-term memory is the dashed, not-yet-wired portion):**

```
graph execution
   │
   ├─► ShortTermMemory.add(...)        [RAM]  logs / decisions / metrics per node
   │
   ├─► ExecutionResult ──► state.sessions  [RAM, FIFO-evict @ 10k]  ──► GET /api/v1/sessions
   │
   └┄┄► MemoryFlusher.flush(trace)     [implemented, NOT invoked in live path]
          │
          └┄┄► LongTermMemory.save(entry)
                 │
                 ├─ MemoryFactory::create(Some(path)) ──► SqliteBackend   [disk, durable]
                 └─ MemoryFactory::create(None)        ──► InMemoryBackend [RAM]
```

**Agent "play" path:**

```
POST /agents/{id}/play ─► clear cycle memory ─► run cycles
                                                   │
                            AgentMemoryStore.cycle_store     [RAM, volatile per play session]
                            AgentMemoryStore.execution_store [RAM, across executions; resets on restart]
                                                   │
                          GET/DELETE /api/v1/agents/{id}/memory
```

**CLI session path (separate process, durable):**

```
CLI turn ─► append_entry(...) ─► transcript.jsonl (append)   [disk]
                              └─► manifest.json (bump ts / counts)  [disk]
   checkpoint ─► append checkpoint-role entry to transcript
   resume     ─► read_transcript ─► rebuild_messages
```

---

## Migration, retention & operational caveats

- **No migration framework for memory tables.** The SQLite schema is created at runtime via `CREATE ... IF NOT EXISTS`
  in `SqliteBackend::new`. There are no versioned migrations for `long_term_memory` / `memory_fts`; schema changes would
  need manual handling.
- **No retention / TTL on long-term entries.** SQLite memory grows until something deletes by id. There is no automatic
  expiry, compaction, or size cap.
- **Engine RAM layers reset on restart.** Short-term memory, the agent KV store (`AgentMemoryStore`, both tiers), and
  `state.sessions` are all in RAM. Sessions additionally FIFO-evict at 10,000.
- **CLI transcripts grow unbounded on disk.** `transcript.jsonl` only ever appends; tool-result content is truncated at
  5,000 characters, but the file itself is never compacted. Sessions are not auto-pruned.
- **Concurrency.** `SqliteBackend` serializes DB access through a `Mutex` and an `unsafe impl Send + Sync`; calls are
  offloaded with `spawn_blocking`. A poisoned mutex surfaces as a `RunnerError::Internal`.
- <a id="wiring-status"></a>**Wiring status (do not assume live behavior).** As of this writing, `MemoryFactory::create`
  / `in_memory` and `MemoryFlusher` have no callers outside their own unit tests. The long-term memory subsystem and the
  flusher are implemented and tested **components**, but a running OpenMirai server/CLI does **not** currently auto-persist
  learnings to long-term memory. Treat long-term memory as an available building block, not an active product behavior.

---

## Summary table

| Layer | File | Backend | Durable? | Process | Live-path? | Exposed via |
|-------|------|---------|----------|---------|-----------|-------------|
| Short-term memory | `engine/src/memory/short_term.rs` | `Vec` (RAM) | No | Engine, per-execution | Component | — |
| Long-term facade | `engine/src/memory/long_term.rs` | trait `MemoryBackend` | depends on backend | Engine | No (no callers) | — |
| SQLite backend | `engine/src/memory/sqlite_backend.rs` | SQLite + FTS5 | **Yes (disk)** | Engine | No (no callers) | — |
| In-memory backend | `engine/src/memory/in_memory_backend.rs` | `Arc<RwLock<Vec>>` | No | Engine | No (no callers) | — |
| Factory | `engine/src/memory/factory.rs` | selects backend | — | Engine | No (no callers) | — |
| Agent KV memory | `engine/src/runtime/agent_memory_store.rs` | `HashMap` (RAM) | No (resets on restart) | Engine, per-agent | **Yes** | `GET/DELETE /api/v1/agents/{id}/memory` |
| Engine sessions | `engine/src/server/state.rs` | `HashMap` (RAM) | No (FIFO @ 10k) | Engine HTTP | **Yes** | `GET /api/v1/sessions[/{id}]`, `/metrics` |
| CLI transcripts | `cli/src/session_storage.rs` | JSON + JSONL files | **Yes (disk)** | CLI | **Yes** | CLI interactive sessions |

**Related docs:** [API.md](API.md) · [SCHEMA.md](../database/SCHEMA.md) · [STORAGE.md](../database/STORAGE.md)
