# FEAT-032 — Engine Rust: Paridad Completa con Python

**Estado**: Draft
**Fecha**: 2026-05-21
**Epic**: EPIC-032

---

## Problem Statement

**Tipo**: Migración — completar paridad Rust vs Python
**Actor**: Desarrollador que consume `datamirai-engine` como dependencia (Local app, Cloud backend)

El Engine Rust está al **~88% de paridad** con el Engine Python. El core (GraphRunner, GraphDef, ToolRegistry) funciona, los LLM adapters están completos (8/8), y el runtime está portado. Sin embargo:

1. **2 tools faltan**: `data/html_to_markdown` y `trigger/heartbeat` no existen en Rust
2. **Memory system incompleto**: Falta la abstracción de backend trait y el factory pattern (2 módulos)
3. **Resources son mocks**: Las implementaciones de `DBResource`, `StorageResource`, `LLMResource` y `VectorResource` son in-memory/mock — no hay persistencia real
4. **Inconsistencias de naming**: `fs/` vs `filesystem/`, `mcp/mcp_call` vs `mcp/call`

Esto causa que el backend Rust (Local app) no pueda ejecutar agentes con persistencia real, y que grafos con `html_to_markdown` o `heartbeat` fallen silenciosamente.

---

## Objetivo

Cuando esto esté implementado:
1. El Engine Rust tiene **100% de paridad funcional** con Python en tools, memory, y resources
2. Los grafos que funcionan en Python funcionan idénticamente en Rust
3. Los resources tienen implementaciones reales (SQLite, filesystem, Ollama LLM)
4. El naming de tools es consistente entre Python y Rust
5. Tests E2E validan la paridad ejecutando grafos reales con datos reales

---

## Features

### 32.1 — Tools faltantes

**Problema**: 2 de 47 tools de Python no existen en Rust. Grafos que los usan fallan.

**Solución**: Implementar los 2 tools faltantes siguiendo la misma interfaz que Python.

**Tools a implementar**:

#### `data/html_to_markdown`
- **Input**: `html` (String) — HTML raw
- **Output**: `markdown` (String) — Markdown limpio
- **Config**: `remove_images` (bool, default false), `remove_links` (bool, default false)
- **Referencia Python**: `Engine/python/src/datamirai_engine/tools/builtin/data/html_to_markdown.py`
- **Dependencia Rust**: crate `html2md` o equivalente

#### `trigger/heartbeat`
- **Input**: ninguno (trigger)
- **Output**: `triggered_at` (String ISO), `evaluation_count` (u64)
- **Config**: `interval_seconds` (u64, default 300), `condition_type` (String), `condition_config` (JSON)
- **Referencia Python**: `Engine/python/src/datamirai_engine/tools/builtin/trigger/heartbeat.py`

**Archivos a crear**:
- CREAR `engine/src/tools/data/html_to_markdown.rs`
- CREAR `engine/src/tools/trigger/heartbeat.rs`
- MODIFICAR `engine/src/tools/data/mod.rs` — registrar html_to_markdown
- MODIFICAR `engine/src/tools/trigger/mod.rs` — registrar heartbeat

---

### 32.2 — Naming consistency

**Problema**: Inconsistencias entre Python y Rust causan que grafos creados en un contexto fallen en otro.

| Python | Rust actual | Corrección |
|--------|-------------|------------|
| `filesystem/*` | `fs/*` | Rust → `filesystem/*` |
| `mcp/call` | `mcp/mcp_call` | Rust → `mcp/call` |

**Solución**: Renombrar tool_type en el registry de Rust para alinearse con Python. Mantener aliases temporales (`fs/*` → `filesystem/*`) para backward compat en grafos existentes.

**Archivos a modificar**:
- MODIFICAR `engine/src/tools/filesystem/mod.rs` — prefijo `filesystem/`
- MODIFICAR `engine/src/tools/mcp/mod.rs` — `mcp/call`
- MODIFICAR `engine/src/tools/registry.rs` — aliases de compatibilidad

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-500 | El tool_type de Rust DEBE ser idéntico al de Python para el mismo tool. Sin excepciones | Registry validation en CI |
| REGLA-501 | Aliases legacy (`fs/*`, `mcp/mcp_call`) se mantienen por 1 versión para backward compat. Se remueven en v0.3.0 | Registry con alias map |

---

### 32.3 — Memory system: backend trait + factory

**Problema**: Python tiene `backend.py` (trait abstracto) y `factory.py` (factory pattern) que permiten intercambiar backends de memoria (InMemory, SQLite, futuro Redis). Rust tiene las implementaciones concretas pero no la abstracción.

**Solución**: Crear el trait `MemoryBackend` y el factory `MemoryFactory` en Rust.

**Componentes**:

```
MemoryBackend (trait)
├── InMemoryBackend (existente, adaptar)
├── SqliteBackend (existente, adaptar)
└── (futuro: RedisBackend, PostgresBackend)

MemoryFactory
├── from_config(config: MemoryConfig) -> Box<dyn MemoryBackend>
└── default() -> Box<dyn MemoryBackend>  // InMemory
```

**Archivos a crear/modificar**:
- CREAR `engine/src/memory/backend.rs` — trait MemoryBackend
- CREAR `engine/src/memory/factory.rs` — MemoryFactory
- MODIFICAR `engine/src/memory/in_memory_backend.rs` — implementar trait
- MODIFICAR `engine/src/memory/sqlite_backend.rs` — implementar trait
- MODIFICAR `engine/src/memory/mod.rs` — re-exportar

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-502 | MemoryBackend trait DEBE ser object-safe (dyn-compatible) para permitir Box<dyn MemoryBackend> | Compilador Rust |
| REGLA-503 | MemoryFactory.default() siempre retorna InMemoryBackend. SQLite requiere path explícito | Factory logic |

---

### 32.4 — Resources: implementaciones reales

**Problema**: Rust tiene `InMemoryDBResource`, `InMemoryStorageResource`, `MockLLMResource`. No hay persistencia real. Esto significa que:
- Datos de agentes se pierden al cerrar el proceso
- LLM calls no llegan a ningún provider real (solo retorna mock response)
- Storage no persiste archivos al filesystem
- Vector search no funciona

**Solución**: Implementar resources reales equivalentes a los de Python.

#### 32.4.1 — SqliteDBResource

Reemplaza `InMemoryDBResource` con SQLite real via `rusqlite`.

**Interfaz**:
```
impl DBResource for SqliteDBResource {
    async fn execute(query, params) -> Result<()>
    async fn fetch_one(query, params) -> Result<Option<Row>>
    async fn fetch_all(query, params) -> Result<Vec<Row>>
}
```

**Config**: `{ path: String }` — ruta al archivo .db
**Referencia Python**: `Engine/python/src/datamirai_engine/resources/sqlite_db.py`

#### 32.4.2 — LocalStorageResource

Reemplaza `InMemoryStorageResource` con filesystem real.

**Interfaz**:
```
impl StorageResource for LocalStorageResource {
    async fn read(path) -> Result<Vec<u8>>
    async fn write(path, data) -> Result<()>
    async fn delete(path) -> Result<()>
    async fn list(prefix) -> Result<Vec<String>>
    async fn exists(path) -> Result<bool>
}
```

**Config**: `{ base_path: String }` — directorio base
**Referencia Python**: `Engine/python/src/datamirai_engine/resources/local_storage.py`

#### 32.4.3 — OllamaLLMResource (mejora)

El backend Rust ya tiene un wrapper `OllamaLLMResource` en `sessions.rs`, pero está hardcodeado ahí. Moverlo al Engine como resource reutilizable.

**Interfaz**:
```
impl LLMResource for OllamaLLMResource {
    async fn call(messages, config) -> Result<LLMResponse>
    async fn stream(messages, config) -> Result<Stream<LLMChunk>>
    async fn embed(texts) -> Result<Vec<Vec<f32>>>
}
```

**Config**: `{ base_url: String, model: String }`
**Referencia Python**: `Engine/python/src/datamirai_engine/resources/llm.py`

#### 32.4.4 — SimpleVectorResource (stub con búsqueda básica)

Implementación mínima que usa SQLite FTS5 para búsqueda de texto. No es un vector store real pero permite que grafos con nodos de búsqueda funcionen.

**Interfaz**:
```
impl VectorResource for SimpleVectorResource {
    async fn upsert(id, text, metadata) -> Result<()>
    async fn search(query, top_k) -> Result<Vec<SearchResult>>
    async fn delete(id) -> Result<()>
}
```

**Archivos a crear/modificar**:
- CREAR `engine/src/resources/sqlite_db.rs`
- CREAR `engine/src/resources/local_storage.rs`
- CREAR `engine/src/resources/ollama_llm.rs`
- CREAR `engine/src/resources/simple_vector.rs`
- MODIFICAR `engine/src/resources/mod.rs` — re-exportar
- MODIFICAR `engine/src/resources/context.rs` — SimpleExecutionContext acepta resources reales

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-504 | SqliteDBResource usa WAL mode por defecto para lecturas concurrentes | Constructor |
| REGLA-505 | LocalStorageResource NUNCA escribe fuera de base_path. Path traversal bloqueado | Validación de path con canonicalize() |
| REGLA-506 | OllamaLLMResource timeout default: 300s para call, 30s para embed | Config defaults |
| REGLA-507 | Si Ollama no está disponible, error descriptivo con instrucciones de instalación, no panic | Error handling |
| REGLA-508 | InMemoryDBResource y MockLLMResource se mantienen para tests. No se eliminan | Coexistencia |

---

## Dependencias entre features

```
32.2 (Naming) ← independiente, puede ir primero
32.1 (Tools) ← independiente
32.3 (Memory) ← independiente
32.4 (Resources) ← depende de 32.3 para MemoryBackend si se integra memory con resources
```

Orden recomendado: 32.2 → 32.1 + 32.3 (paralelo) → 32.4

---

## Tests E2E esperados

Los tests validan paridad funcional entre Engine Python y Rust. Se ejecutan con Playwright `--headed` contra el backend Rust + frontend.

### Tests de tools

| # | Test | Valida |
|---|------|--------|
| T-032-01 | Grafo con nodo `data/html_to_markdown` convierte HTML real a Markdown | Tool existe y funciona |
| T-032-02 | Grafo con nodo `trigger/heartbeat` se dispara y ejecuta pipeline | Heartbeat trigger funcional |
| T-032-03 | Grafo con nodos `filesystem/*` (nuevo prefijo) ejecuta sin error | Naming consistency |
| T-032-04 | Grafo legacy con nodos `fs/*` ejecuta sin error (alias) | Backward compat |

### Tests de resources

| # | Test | Valida |
|---|------|--------|
| T-032-05 | Agente ejecuta `db_write` → datos persisten en SQLite → `db_read` los recupera | SqliteDBResource real |
| T-032-06 | Agente ejecuta `storage_write` → archivo existe en filesystem → `storage_read` lo lee | LocalStorageResource real |
| T-032-07 | Agente ejecuta `ai/llm_call` con Ollama → respuesta real con tokens > 0 | OllamaLLMResource real |
| T-032-08 | Pipeline completo: `web_scrape` → `html_to_markdown` → `db_write` → `llm_call` → `response` | Integración end-to-end |

### Tests de memory

| # | Test | Valida |
|---|------|--------|
| T-032-09 | Agente ejecuta, memoria se persiste. Segunda ejecución lee memoria anterior | Memory persistence |
| T-032-10 | MemoryFactory crea backend correcto según config | Factory pattern |

---

## Notas de implementación

- Los crates de Rust necesarios: `html2md` (html→markdown), `rusqlite` (ya existe como dep)
- `OllamaLLMResource` debe moverse de `server-rs/src/routes/sessions.rs` al Engine propiamente
- Los tests E2E requieren Ollama corriendo localmente con al menos un modelo (e.g. `llama3.2:3b`)
- La paridad de naming es crítica porque el frontend genera `GraphDef` con tool_types que deben coincidir exactamente

---

## doc_refs

- `Engine/python/src/datamirai_engine/tools/builtin/` — referencia de tools Python
- `Engine/engine/src/tools/` — implementación actual Rust
- `Engine/python/src/datamirai_engine/resources/` — referencia de resources Python
- `Engine/engine/src/resources/` — implementación actual Rust (mocks)
- `Engine/python/src/datamirai_engine/memory/` — referencia de memory Python
- `Engine/engine/src/memory/` — implementación actual Rust
- `docs/ARCHITECTURE.md` — stack y convenciones
