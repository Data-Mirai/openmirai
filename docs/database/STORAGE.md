<!--
BLUEPRINT SEED — STORAGE.md
Responsable: → blueprint/agents/03-ENTITIES.md

Estructura esperada:
- Por bucket: descripción, público (sí/no), límite tamaño, tipos permitidos, estructura de paths, policies

Reglas:
- NO SQL / código — declarativo en tablas
- Referenciar → DOMINIO.md#rol-X en las policies
- Incluir límites concretos (tamaño, tipos de archivo)
- Si el proyecto no usa object storage, este archivo puede quedarse vacío o eliminarse
-->

# STORAGE.md

OpenMirai implements a **local-only storage system** with no external object storage (S3, MinIO, GCS). All data is persisted either to the **local filesystem** or held **in-memory** during execution. The storage layer is abstraction-first: a `StorageResource` trait enables pluggable backends, and both filesystem and in-memory implementations are provided.

---

## Architecture Overview

### Storage Backends

OpenMirai provides two concrete storage implementations:

#### 1. **LocalStorageResource** (Filesystem-backed)
**File:** `engine/src/adapters/local_storage.rs`

- **Root Directory:** All operations are scoped to a configurable `base_path`.
- **Path Traversal Protection:** Paths are canonicalized and validated to prevent escape attacks (REGLA-505).
- **Async Operations:** All I/O is offloaded to `tokio::task::spawn_blocking()` to avoid blocking the async runtime.
- **Directory Auto-creation:** Parent directories are created automatically on writes.

**Interface:**
- `put(path: &str, data: &[u8])` — Write or overwrite a file.
- `get(path: &str)` → `Result<Vec<u8>>` — Read a file; returns `NotFound` if missing.
- `delete(path: &str)` — Remove a file; no-op if file does not exist.

#### 2. **InMemoryStorageResource** (Development/Testing)
**File:** `engine/src/adapters/in_memory_storage.rs`

- **Backend:** `HashMap<String, Vec<u8>>` wrapped in `Arc<RwLock<...>>` for thread-safe concurrent access.
- **Use Case:** Development, integration tests, and scenarios where persistence is not required.
- **Bonus Method:** `list_keys(prefix: &str)` — Return all keys matching a prefix (useful for testing).

**Interface:** Same as `LocalStorageResource`.

---

## Storage Resource Trait

**File:** `engine/src/core/context.rs`

```rust
pub trait StorageResource: Send + Sync {
    async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError>;
    async fn put(&self, path: &str, data: &[u8]) -> Result<(), ResourceError>;
    async fn delete(&self, path: &str) -> Result<(), ResourceError>;
}
```

Implementors are injected into the `ExecutionContext` during agent execution, allowing tools and graphs to operate uniformly regardless of the underlying backend.

---

## Storage Tools

### storage_read
**File:** `engine/src/tools/builtin/data/storage_read.rs`

Reads data from local storage or generates presigned URLs.

| Input | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | ✓ | Storage path/key to read |

| Output | Type | Description |
|--------|------|-------------|
| `content` | string or null | File content (text); binary data shown as `<binary data: N bytes>`; null if not found or in presign mode |
| `path` | string | Path that was read |
| `found` | boolean | Whether the file was found |

| Config | Type | Default | Description |
|--------|------|---------|-------------|
| `mode` | string | `"read"` | `"read"` for data retrieval; `"presign"` for presigned URL placeholder (always returns `found: true`, `content: null`) |

---

### storage_write
**File:** `engine/src/tools/builtin/data/storage_write.rs`

Writes data to local storage at a specified path.

| Input | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | ✓ | Storage path/key to write to |
| `content` | string | ✓ | Content to write |

| Output | Type | Description |
|--------|------|-------------|
| `path` | string | Path that was written |
| `bytes_written` | number | Number of bytes written |

---

## Vault: Obsidian-Inspired Knowledge System

**Directory:** `engine/src/vault/`

The vault is a filesystem-backed Markdown note system inspired by Obsidian. It provides structured document storage with backlinks, tagging, and full-text search.

### VaultService
**File:** `engine/src/vault/service.rs`

#### Core Concepts

- **Notes:** `.md` files with optional YAML frontmatter (`title`, `tags`, custom fields).
- **Wiki-links:** References in the form `[[note-name]]` or `[[note-name|alias]]`.
- **Backlinks:** Bidirectional link tracking; automatically computed when notes are written.
- **Index:** In-memory `NoteIndex` mapping paths → metadata and target → sources (for backlinks).

#### API

| Method | Signature | Returns |
|--------|-----------|---------|
| `new(root_path)` | Create and scan vault | `io::Result<Self>` |
| `read_note(path)` | Parse a `.md` file | `io::Result<ParsedNote>` |
| `write_note(path, content, metadata)` | Save a note and update index | `io::Result<()>` |
| `search(query, limit)` | Full-text search (title + body) | `Vec<IndexedNote>` |
| `search_by_tags(tags)` | Find notes with ALL tags | `Vec<IndexedNote>` |
| `get_backlinks(path)` | List source paths linking to this note | `Vec<String>` |
| `list_recent(limit)` | Newest notes by mtime | `Vec<IndexedNote>` |
| `delete_note(path)` | Remove from disk and index | `io::Result<()>` |
| `rebuild_index()` | Rescan all `.md` files | (void) |

#### Markdown Format

Notes use **YAML frontmatter** followed by Markdown body:

```markdown
---
title: My Note
tags:
  - research
  - finance
custom_field: value
---

# Content

See [[related-note]] for context.
```

#### Data Structures

**IndexedNote:**
```rust
pub struct IndexedNote {
    pub path: String,              // Relative path from vault root
    pub title: String,             // From frontmatter or first H1 or body
    pub tags: Vec<String>,         // From frontmatter
    pub outlinks: Vec<String>,     // Wiki-links extracted from body
    pub modified_at: f64,          // UNIX timestamp (seconds)
}
```

**NoteIndex:**
```rust
pub struct NoteIndex {
    pub notes: HashMap<String, IndexedNote>,      // path → metadata
    pub backlinks: HashMap<String, Vec<String>>,  // target → [sources...]
}
```

---

## Vault Tools (Placeholder)

### vault_read
**File:** `engine/src/tools/builtin/data/vault_read.rs`

Reads notes from the vault (currently a placeholder returning empty results).

| Input | Type | Required | Description |
|-------|------|----------|-------------|
| `query` | string | | Search query for vault notes |
| `path` | string | | Specific vault path to read |

| Output | Type | Description |
|--------|------|-------------|
| `notes` | array | Matched vault notes |
| `count` | number | Number of notes returned |

| Config | Type | Description |
|--------|------|-------------|
| `folder` | string | Vault folder to search in |
| `limit` | number | Maximum notes to return |

---

### vault_write
**File:** `engine/src/tools/builtin/data/vault_write.rs`

Writes a note to the vault (currently a placeholder acknowledging writes).

| Input | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | ✓ | Vault path for the note |
| `title` | string | | Note title |
| `content` | string | ✓ | Note content |

| Output | Type | Description |
|--------|------|-------------|
| `path` | string | Path where note was written |
| `written` | boolean | Whether write succeeded |

| Config | Type | Description |
|--------|------|-------------|
| `tags` | string | Comma-separated tags |

---

## FileRef: First-Class File References

**File:** `engine/src/llm/media.rs`

`FileRef` is a **structured JSON object** that represents a file as a first-class value in graphs. It enables type-safe file passing between tools and supports metadata queries without re-reading files.

### FileRef Structure

```json
{
  "_type": "file_ref",
  "path": "/absolute/path/to/file.png",
  "mime_type": "image/png",
  "size_bytes": 12345
}
```

### FileRef Operations

| Function | Purpose |
|----------|---------|
| `create_file_ref(file_path, base_dir?)` | Build a FileRef from a path; returns `None` if file does not exist |
| `is_file_ref(value)` | Check if a JSON value is a valid FileRef object |
| `resolve_file_input(value)` | Extract path from a FileRef or string input (backward compat) |

### Use in Tools

Tools can accept FileRef objects in place of string paths, enabling:
- **Type safety:** The LLM can distinguish file references from raw strings.
- **Lazy evaluation:** File metadata is available without re-reading.
- **Backward compatibility:** String paths still work via `resolve_file_input()`.

**Example tool input:**
```json
{
  "input_file": {
    "_type": "file_ref",
    "path": "/tmp/image.png",
    "mime_type": "image/png",
    "size_bytes": 45678
  }
}
```

---

## Execution Context Integration

**File:** `engine/src/adapters/context.rs`

The `ExecutionContext` trait (defined in `engine/src/core/context.rs`) provides optional access to storage. The concrete `DefaultExecutionContext` implementation wires all resources together:

```rust
pub trait ExecutionContext: Send + Sync {
    fn storage(&self) -> Option<&dyn StorageResource>;
    // ... other resources ...
}
```

### Configuration

**Development/Testing:**
```rust
let ctx = DefaultExecutionContext::default_dev();
// Includes InMemoryStorageResource
```

**Custom (Filesystem):**
```rust
let storage = Box::new(LocalStorageResource::new("/data/storage")?);
let ctx = DefaultExecutionContext::builder(llm)
    .with_storage(storage)
    .build();
```

Storage is optional; tools gracefully handle `None` by returning `"no storage resource configured"`.

---

## Limitations and Design Constraints

1. **No External Storage:** S3, MinIO, GCS, and other cloud object stores are not supported. Only local filesystem or in-memory backends.
2. **No Vault Real Implementation:** `vault_read` and `vault_write` tools are placeholders; actual vault operations must use `VaultService` directly.
3. **No Presigning:** The `storage_read` tool's `presign` mode is a placeholder; real presigned URL generation is not implemented.
4. **20 MB File Limit:** Media files (images, audio, video) for LLM consumption are capped at 20 MB.
5. **Path Traversal Prevention:** `LocalStorageResource` blocks `..` and symlink escapes; paths are canonicalized and validated.

---

## Security Model

### Path Traversal Protection (REGLA-505)

`LocalStorageResource` validates all paths to prevent directory escape:

1. **Normalize:** Reject paths containing `..` before file creation.
2. **Canonicalize:** For existing paths, canonicalize and verify the resolved path starts with `base_path`.
3. **Atomic:** Both checks are applied before any I/O operation.

### In-Memory Storage

`InMemoryStorageResource` is suitable only for development and testing; it has no persistence, no encryption, and no multi-process isolation.

---

## Examples

### Reading from Storage

YAML graph:
```yaml
nodes:
  - id: read_data
    tool: data/storage_read
    config: {}
    inputs:
      path: "documents/report.txt"
```

Output:
```json
{
  "path": "documents/report.txt",
  "content": "Annual Report 2024...",
  "found": true
}
```

### Writing to Storage

YAML graph:
```yaml
nodes:
  - id: save_result
    tool: data/storage_write
    config: {}
    inputs:
      path: "results/analysis.json"
      content: "{\"status\": \"complete\"}"
```

Output:
```json
{
  "path": "results/analysis.json",
  "bytes_written": 24
}
```

### Vault Search (via VaultService)

Code:
```rust
let vault = VaultService::new(&Path::new("./vault"))?;
let results = vault.search("quarterly forecast", 5);
for note in results {
    println!("{} ({})", note.title, note.path);
}
```

### Creating a FileRef

Code:
```rust
let file_ref = create_file_ref("./images/chart.png", Some("./data"));
// If file exists:
// {
//   "_type": "file_ref",
//   "path": "/absolute/path/to/data/images/chart.png",
//   "mime_type": "image/png",
//   "size_bytes": 87654
// }
```

---

## Summary Table

| Component | Type | Backend | Use Case |
|-----------|------|---------|----------|
| **LocalStorageResource** | Storage Backend | Filesystem | Production, persistent data |
| **InMemoryStorageResource** | Storage Backend | HashMap | Dev, testing, ephemeral data |
| **storage_read** | Tool | Abstracted | Read files from storage |
| **storage_write** | Tool | Abstracted | Write files to storage |
| **VaultService** | Service | Filesystem | Markdown notes + backlinks |
| **vault_read / vault_write** | Tools | Placeholder | (Not implemented) |
| **FileRef** | Data Type | JSON | Type-safe file references |
