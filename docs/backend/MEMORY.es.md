# MEMORY.es.md

OpenMirai tiene **varias capas de memoria deliberadamente separadas**, no un único almacén unificado. Difieren en
propósito, durabilidad y qué proceso las posee. La mayor fuente de confusión es que la palabra
"memoria" está sobrecargada a través de tres subsistemas del motor más un almacén de transcripciones de la CLI — y la capa que
los endpoints HTTP de `/memory` exponen **no** es la misma que el subsistema de memoria a largo plazo en
`engine/src/memory/`. Este documento mapea cada capa, qué persiste, dónde reside y — lo más importante —
qué capas están conectadas en la ruta de ejecución en vivo frente a aquellas implementadas-pero-aún-no-invocadas.

| Capa | Archivo | Backend | ¿Durable? | Alcance del proceso | ¿Conectada en ruta en vivo? |
|-------|------|---------|----------|---------------|------------------------|
| Memoria a corto plazo | `engine/src/memory/short_term.rs` | `Vec` en RAM | No | Motor, por-ejecución | Solo componente (no se invoca auto.) |
| Memoria a largo plazo (SQLite) | `engine/src/memory/sqlite_backend.rs` | SQLite + FTS5 file | **Sí** (en disco) | Motor | **No** — sin llamadores en vivo aún |
| Memoria a largo plazo (in-mem) | `engine/src/memory/in_memory_backend.rs` | `Arc<RwLock<Vec>>` | No | Motor | **No** — sin llamadores en vivo aún |
| Memoria KV de Agente | `engine/src/runtime/agent_memory_store.rs` | `HashMap` en RAM | No (se reinicia) | Motor, por-agente | **Sí** — `/api/v1/agents/{id}/memory` |
| Estado de sesión del motor | `engine/src/server/state.rs` | `HashMap` en RAM | No (evicción FIFO) | Servidor HTTP del motor | **Sí** — `/api/v1/sessions` |
| Transcripciones sesión CLI | `cli/src/session_storage.rs` | Archivos JSON + JSONL | **Sí** (en disco) | CLI solamente | **Sí** — sesiones chat CLI |

> **Lea esto primero:** el subsistema de memoria a largo plazo (`LongTermMemory`, `SqliteBackend`, `InMemoryBackend`,
> `MemoryFactory`) y el `MemoryFlusher` que está destinado a poblarlo están completamente implementados y probados unitariamente,
> pero actualmente **no tienen llamadores en el servidor o CLI en ejecución**. Son bloques de construcción disponibles, no
> un comportamiento activo de un OpenMirai en funcionamiento hoy. Ver [Estado de conexión](#wiring-status).

---

## Descripción general del modelo de memoria

Hay tres conceptos del lado del motor y un concepto del lado de la CLI:

1. **Memoria a corto plazo** — una traza en RAM de una única ejecución de grafo (logs, decisiones, métricas por bloque).
   Vive y muere con la ejecución.
2. **Memoria a largo plazo** — aprendizaje persistente a través de ejecuciones, detrás de un trait `MemoryBackend` conectable con
   dos implementaciones (SQLite-con-FTS5, y un backend solo en RAM para dev/testing). Seleccionado por `MemoryFactory`.
3. **Memoria KV de Agente** — un almacén clave/valor por agente (`AgentMemoryStore`) usado por la característica "play" del agente en vivo,
   con un nivel con alcance de ejecución y un nivel con alcance de ciclo (volátil). Solo RAM en V1.
4. **Transcripciones de sesión de la CLI** — logs de conversación durables en disco que la CLI escribe para sus sesiones de chat
   interactivas. El motor nunca lee estos; son un asunto exclusivo de la CLI.

Estos no comparten almacenamiento. En particular, la memoria del motor y las transcripciones de la CLI viven en procesos diferentes con
diferentes garantías de durabilidad — ver [Almacenamiento de sesión CLI vs memoria del motor](#cli-session-storage-vs-engine-memory).

---

## Memoria a corto plazo

**Archivo:** `engine/src/memory/short_term.rs`

`ShortTermMemory` captura el detalle paso a paso de la ejecución **actual** del grafo. Es un simple
`Vec<ShortTermEntry>` mantenido en RAM — no hay persistencia ni backend. Cuando el valor poseedor se libera
(la ejecución termina), la traza desaparece.

**Campos de `ShortTermEntry`:**

| Campo | Tipo | Descripción |
|-------|------|-------------|
| `timestamp` | `f64` | Segundos Unix (fraccionales) cuando la entrada fue registrada |
| `entry_type` | `String` | `"log"`, `"decision"`, o `"metric"` |
| `node_id` | `Option<String>` | Bloque/nodo que produjo la entrada, si aplica |
| `content` | `String` | Contenido legible por humanos |
| `metadata` | `HashMap<String, Value>` | Datos extra arbitrarios |

**API:** `add(entry_type, content, node_id, metadata)` para registrar; `get_all()` devuelve todas las entradas en orden
de inserción; `get_by_type(t)` y `get_by_node(id)` filtran; `clear()` vacía; `len()` / `is_empty()` para tamaño.

**Durabilidad:** RAM local del proceso solamente.

---

## Memoria a largo plazo

**Archivo:** `engine/src/memory/long_term.rs`

La memoria a largo plazo persiste aprendizajes/decisiones/patrones/resoluciones-de-errores a través de las ejecuciones. Es una fachada delgada
(`LongTermMemory`) sobre un backend conectable (trait `MemoryBackend`), por lo que el motor de almacenamiento puede ser intercambiado sin
cambiar a los llamadores.

**Campos de `LongTermEntry`:**

| Campo | Tipo | Descripción |
|-------|------|-------------|
| `id` | `String` | UUID — **asignado por el backend** al guardar (los llamadores pasan una cadena vacía) |
| `entry_type` | `String` | `"decision"`, `"learning"`, `"pattern"`, o `"error_resolution"` |
| `content` | `String` | Contenido legible por humanos |
| `tags` | `Vec<String>` | Etiquetas buscables |
| `session_id` | `Option<String>` | Sesión que produjo la entrada, si aplica |
| `created_at` | `f64` | Segundos Unix; si se deja en `0.0`, el backend lo estampa con el reloj de pared actual |
| `metadata` | `HashMap<String, Value>` | Datos extra arbitrarios |

**Trait `MemoryBackend`** (todos los métodos `async`, falibles con `RunnerError`):

| Método | Devuelve | Propósito |
|--------|---------|---------|
| `save(&entry)` | `String` (id asignado) | Persistir una entrada |
| `search(query, limit)` | `Vec<LongTermEntry>` | Búsqueda de texto completo, hasta `limit` coincidencias |
| `get(id)` | `Option<LongTermEntry>` | Buscar una entrada por id |
| `delete(id)` | `()` | Eliminar por id (idempotente) |
| `list_recent(limit)` | `Vec<LongTermEntry>` | Entradas más recientes, primero las más nuevas |

`LongTermMemory::new(Box<dyn MemoryBackend>)` envuelve un backend y delega cada llamada a él.

---

## Comportamiento del backend SQLite

**Archivo:** `engine/src/memory/sqlite_backend.rs`

`SqliteBackend` es el `MemoryBackend` durable, respaldado por `rusqlite` con un índice de texto completo FTS5. Es la
única capa de memoria (además de las transcripciones de la CLI) que escribe en disco.

**Esquema** (creado al abrir mediante `CREATE ... IF NOT EXISTS`):

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

**Notas de comportamiento:**

- **Construcción:** `SqliteBackend::new(path)` abre (o crea) el archivo, establece `PRAGMA journal_mode=WAL` y
  `PRAGMA foreign_keys=ON`, luego aplica el esquema. `new_in_memory()` abre `:memory:` (usado por pruebas).
- **`tags` y `metadata`** se almacenan como cadenas JSON serializadas (valores por defecto `'[]'` / `'{}'`), deserializadas al leer.
- **La sincronización FTS** es automática a través de los disparadores `memory_ai` / `memory_ad`; no hay un paso separado de reconstrucción del índice.
- **Seguridad async:** `rusqlite` es síncrono, por lo que cada llamada a la DB se ejecuta dentro de `tokio::task::spawn_blocking`. La
  conexión se mantiene detrás de `Arc<Mutex<Connection>>`; el envoltorio lleva un `unsafe impl Send + Sync` justificado por
  ese protector de mutex.
- **`save`** asigna un nuevo UUID, establece por defecto `created_at` a ahora cuando el llamador pasó `0.0`, y devuelve el id.
- **`get` / `delete`** operan por id; eliminar un id inexistente no es un error.
- Este esquema **no** está documentado en [SCHEMA.md](../database/SCHEMA.md) (que cubre la tabla declarativa `sessions`); el
  esquema de memoria vive aquí y es creado en tiempo de ejecución por este backend.

**Durabilidad:** persistente en disco en la ruta configurada.

---

## Comportamiento del backend en memoria

**Archivo:** `engine/src/memory/in_memory_backend.rs`

`InMemoryBackend` es el `MemoryBackend` por defecto para desarrollo y pruebas. Almacena entradas en un
`Arc<RwLock<Vec<LongTermEntry>>>`, por lo que es económico de clonar y seguro para compartir a través de tareas async.

- **`save`** asigna un UUID y establece por defecto `created_at` a ahora cuando es `0.0`, luego inserta la entrada.
- **`search`** es una coincidencia de subcadena insensible a mayúsculas (`contains`) en `content` (no FTS), ordenada por la más nueva primero y
  truncada a `limit`.
- **`get` / `delete`** son por id; la eliminación es idempotente.
- **`list_recent`** devuelve todas las entradas ordenadas por la más nueva primero, truncadas a `limit`.

**Durabilidad:** RAM local del proceso solamente — todo se pierde al reiniciar.

---

## Selección de fábrica / proveedor

**Archivo:** `engine/src/memory/factory.rs`

`MemoryFactory` elige el backend a partir de una ruta de base de datos opcional:

```rust
MemoryFactory::create(Some(path))  // → SqliteBackend (persistente)
MemoryFactory::create(None)        // → InMemoryBackend (sin persistencia)
MemoryFactory::in_memory()         // → InMemoryBackend (conveniencia)
```

El contrato es simple: un `Some(path)` selecciona SQLite durable; `None` selecciona el backend en RAM. No hay una capa de
resolución de variables de entorno o archivos de configuración aquí — la selección es puramente por el argumento pasado.

> **Advertencia de conexión:** `MemoryFactory::create` / `in_memory` actualmente **no tienen llamadores** fuera de las propias
> pruebas de este módulo. Nada en el servidor o CLI en ejecución construye un backend de memoria a largo plazo todavía. Ver [Estado de conexión](#wiring-status).

---

## Almacén de memoria KV de Agente

**Archivo:** `engine/src/runtime/agent_memory_store.rs` (PRD-008)

`AgentMemoryStore` es un almacén clave/valor por agente usado por la característica **play** del agente en vivo. Es **distinto del
subsistema de memoria a largo plazo anterior** y es la capa que los endpoints HTTP de `/memory` realmente exponen.

Mantiene dos mapas, cada uno `Arc<RwLock<HashMap<agent_id, HashMap<String, Value>>>>`:

- **Execution memory** (`execution_store`) — destinada a persistir a través de ejecuciones de un agente.
- **Cycle memory** (`cycle_store`) — volátil, con alcance de una sola sesión de play; se limpia cuando comienza una nueva sesión de play.

Las escrituras están **filtradas a las claves de memoria declaradas del agente** (las claves no declaradas en la especificación del agente se descartan).

**Durabilidad:** solo RAM. El módulo lo establece explícitamente — *"V1: in-memory only — resets on server restart."* Incluso
la memoria de "ejecución" no sobrevive a un reinicio del servidor; solo sobrevive a ejecuciones individuales dentro de un proceso de servidor.

---

## Estado de sesión del motor

**Archivo:** `engine/src/server/state.rs`

El servidor HTTP mantiene los resultados de ejecución finalizados en `state.sessions`, un `Arc<RwLock<HashMap<String, ExecutionResult>>>`.
Un vector compañero `session_order` impone la **evicción FIFO** una vez que se excede `DEFAULT_MAX_SESSIONS` (10,000).

**Durabilidad:** RAM local del proceso. Las sesiones existen solo durante la vida del proceso del servidor y se evitan las más antiguas primero
al llegar al límite. (Nota: `SCHEMA.md` describe una tabla declarativa `sessions`, pero el servidor en vivo no persiste las sesiones en ella).

---

## Almacenamiento de sesión CLI vs memoria del motor

**Archivo:** `cli/src/session_storage.rs`

Este es el **único** almacén de conversación durable y respaldado por disco en el producto en vivo, y pertenece enteramente a la **CLI**.
El motor nunca lo lee ni lo escribe.

**Distribución:** cada sesión vive bajo `~/.datamirai/sessions/<session_id>/` (el nombre del directorio `.datamirai` se mantiene por
compatibilidad con versiones anteriores):

- `manifest.json` — metadatos: `id`, `provider`, `model`, `cwd`, `created_at`, `updated_at`, `message_count`,
  `checkpoint_count`, `status` (`"active"` | `"closed"`).
- `transcript.jsonl` — log de solo anexado; un objeto JSON por línea.

**Las entradas de la transcripción** llevan `ts`, `role`, `content` opcional y `metadata` aplanados. Los roles incluyen `user`,
`assistant`, `tool_call`, `tool_result` y `checkpoint`. Comportamientos notables:

- **Los resultados de las herramientas se truncan** a 5000 caracteres (con un sufijo `...(truncated)`) antes de ser escritos.
- **Los puntos de control (checkpoints) son entradas de la transcripción**, no instantáneas separadas. `create_checkpoint` anexa una entrada de rol `checkpoint`
  registrando un `message_index`; `list_checkpoints` los reconstruye escaneando la transcripción.
- **`rebuild_messages`** reconstruye un historial de mensajes de LLM reproduciendo solo las entradas de `user` / `assistant` / `system`.
- El anexado actualiza `updated_at` y `message_count`; `list_sessions` lee los manifiestos ordenados por recencia; `close_session`
  cambia el `status` a `"closed"`.

### Engine memory ≠ CLI transcripts

| | Engine memory | CLI session transcripts |
|---|---|---|
| Owning process | Engine (HTTP server / runtime) | CLI binary |
| What it stores | Execution traces, agent KV, learnings | Full chat conversation + tool I/O |
| Durability | RAM (or SQLite for long-term, when wired) | Files on disk (`~/.datamirai/sessions/`) |
| Shared store? | No | No — engine never reads these files |

Los dos nunca comparten estado. Una "sesión" en la API HTTP del motor (`/api/v1/sessions`) es un registro de una ejecución de grafo en
RAM; una "sesión" en la CLI es una conversación interactiva durable en disco. Son cosas diferentes con el mismo nombre.

---

## Semántica de búsqueda

| Capa | Mecanismo de búsqueda | Ordenación |
|-------|------------------|----------|
| Largo plazo (SQLite) | FTS5 `MATCH`; cada token de espacio en blanco se convierte en `"token"*` (coincidencia de prefijo). Consulta vacía/en blanco → resultado vacío. | `created_at DESC` |
| Largo plazo (en-mem) | Coincidencia de subcadena insensible a mayúsculas (`content.contains`) | `created_at DESC` |
| Corto plazo | Sin búsqueda de texto — filtrar por `entry_type` o `node_id` solamente | orden de inserción |
| Transcripción CLI | Sin índice — lectura lineal de `transcript.jsonl` | orden de anexado |

El **escritor** previsto de la memoria a largo plazo es el `MemoryFlusher` (`engine/src/intelligence/memory_flusher.rs`): este
extrae entradas de resolución-de-errores y patrones de una traza de ejecución y las guarda en un `LongTermMemory`. Se dispara
cuando el contexto acumulado alcanza un umbral (por defecto 0.75 del contexto máximo), como máximo una vez por sesión (REGLA-58). Ver la
advertencia de conexión a continuación — el flusher está implementado y probado pero actualmente no se invoca en la ruta en vivo.

---

## Endpoints de la API que exponen datos de memoria/sesión

Referencia cruzada: [API.md](API.md). Requieren `X-API-Key` cuando la
autenticación del servidor está configurada.

| Endpoint | Expone | Respaldado por |
|----------|---------|-----------|
| `GET /api/v1/agents/{id}/memory` | Memoria KV actual del agente | `AgentMemoryStore` (RAM) |
| `DELETE /api/v1/agents/{id}/memory` | Restablece la memoria del agente a valores iniciales (409 si el agente está en play) | `AgentMemoryStore` (RAM) |
| `GET /api/v1/sessions` | Lista sesiones de ejecución (parámetros de consulta `agent_id`, `limit`) | `state.sessions` (RAM, evicción FIFO) |
| `GET /api/v1/sessions/{id}` | Detalle completo de sesión incl. `trace` de ejecución (404 si falta) | `state.sessions` (RAM) |
| `GET /api/v1/metrics` | Conteos/métricas agregados sobre sesiones almacenadas | `state.sessions` (RAM) |

`GET /api/v1/agents/{id}/memory` devuelve `{ "agent_id", "memory": { ...claves declaradas... } }`. Los endpoints de sesión devuelven
registros en RAM (`id`, `status`, `trace`/`trace_len`, `error`).

> **No hay ningún endpoint que exponga el `SqliteBackend` a largo plazo** — el subsistema de memoria a largo plazo no es accesible
> a través de HTTP hoy. Los endpoints de `/memory` sirven al almacén KV del agente, no a los aprendizajes a largo plazo.

---

## Ciclo de vida (diagrama de texto)

**Ruta de ejecución del motor (la memoria a largo plazo es la porción punteada, aún no conectada):**

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

**Ruta "play" del agente:**

```
POST /agents/{id}/play ─► clear cycle memory ─► run cycles
                                                   │
                            AgentMemoryStore.cycle_store     [RAM, volatile per play session]
                            AgentMemoryStore.execution_store [RAM, across executions; resets on restart]
                                                   │
                          GET/DELETE /api/v1/agents/{id}/memory
```

**Ruta de sesión CLI (proceso separado, durable):**

```
CLI turn ─► append_entry(...) ─► transcript.jsonl (append)   [disk]
                              └─► manifest.json (bump ts / counts)  [disk]
   checkpoint ─► append checkpoint-role entry to transcript
   resume     ─► read_transcript ─► rebuild_messages
```

---

## Migración, retención y advertencias operativas

- **Sin marco de migración para las tablas de memoria.** El esquema SQLite se crea en tiempo de ejecución mediante `CREATE ... IF NOT EXISTS`
  en `SqliteBackend::new`. No hay migraciones versionadas para `long_term_memory` / `memory_fts`; los cambios de esquema
  necesitarían manejo manual.
- **Sin retención / TTL en las entradas a largo plazo.** La memoria SQLite crece hasta que algo elimine por id. No hay expiración
  automática, compactación o límite de tamaño.
- **Las capas de RAM del motor se reinician al reiniciar.** La memoria a corto plazo, el almacén KV del agente (`AgentMemoryStore`, ambos niveles), y
  `state.sessions` están todos en RAM. Las sesiones adicionalmente se evitan por FIFO a las 10,000.
- **Las transcripciones de la CLI crecen sin límites en disco.** `transcript.jsonl` solo anexan; el contenido de los resultados de herramientas se trunca a
  5,000 caracteres, pero el archivo en sí nunca se compacta. Las sesiones no se purgan automáticamente.
- **Concurrencia.** `SqliteBackend` serializa el acceso a la DB a través de un `Mutex` y un `unsafe impl Send + Sync`; las llamadas
  se descargan con `spawn_blocking`. Un mutex envenenado surge como un `RunnerError::Internal`.
- <a id="wiring-status"></a>**Estado de conexión (no asuma comportamiento en vivo).** Al momento de escribir esto, `MemoryFactory::create`
  / `in_memory` y `MemoryFlusher` no tienen llamadores fuera de sus propias pruebas unitarias. El subsistema de memoria a largo plazo y el
  flusher son **componentes** implementados y probados, pero un servidor/CLI OpenMirai en ejecución **no** persiste actualmente de forma automática los
  aprendizajes en la memoria a largo plazo. Trate la memoria a largo plazo como un bloque de construcción disponible, no como un comportamiento activo del producto.

---

## Tabla de resumen

| Capa | Archivo | Backend | ¿Durable? | Proceso | ¿Ruta-en-vivo? | Expuesto vía |
|-------|------|---------|----------|---------|-----------|-------------|
| Memoria a corto plazo | `engine/src/memory/short_term.rs` | `Vec` (RAM) | No | Motor, por-ejecución | Componente | — |
| Fachada largo plazo | `engine/src/memory/long_term.rs` | trait `MemoryBackend` | depende del backend | Motor | No (sin llamadores) | — |
| Backend SQLite | `engine/src/memory/sqlite_backend.rs` | SQLite + FTS5 | **Sí (disco)** | Motor | No (sin llamadores) | — |
| Backend en memoria | `engine/src/memory/in_memory_backend.rs` | `Arc<RwLock<Vec>>` | No | Motor | No (sin llamadores) | — |
| Fábrica | `engine/src/memory/factory.rs` | selecciona backend | — | Motor | No (sin llamadores) | — |
| Memoria KV de Agente | `engine/src/runtime/agent_memory_store.rs` | `HashMap` (RAM) | No (se reinicia) | Motor, por-agente | **Sí** | `GET/DELETE /api/v1/agents/{id}/memory` |
| Sesiones del motor | `engine/src/server/state.rs` | `HashMap` (RAM) | No (FIFO @ 10k) | Motor HTTP | **Sí** | `GET /api/v1/sessions[/{id}]`, `/metrics` |
| Transcripciones CLI | `cli/src/session_storage.rs` | Archivos JSON + JSONL | **Sí (disco)** | CLI | **Sí** | Sesiones interactivas CLI |

**Documentos relacionados:** [API.md](API.md) · [SCHEMA.md](../database/SCHEMA.md) · [STORAGE.md](../database/STORAGE.md)
