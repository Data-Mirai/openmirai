# RAG and Search

## Purpose

OpenMirai has a RAG/search subsystem for retrieving relevant text chunks from supplied documents. The current production RAG paths are stateless: each request or command chunks the provided documents, embeds every chunk and the query with an LLM provider, scores chunks with cosine similarity, and returns the top results.

There are three largely disconnected search code paths:

| Path | Runtime Surface | What It Does | Integration Status |
| --- | --- | --- | --- |
| Production RAG path | `POST /api/v1/rag/search`, `data/rag_search`, `mirai rag search` | Chunk -> embed query and chunks with a real LLM -> brute-force cosine similarity -> top-K | Active runtime path |
| Search provider library | `engine/src/search/` | `HybridSearch`, `InMemoryVectorProvider`, `VectorSearchProvider`, `FTSProvider` | Library/test-only; not wired into HTTP/tool/CLI |
| FTS5 keyword store | `SimpleVectorResource` via `context.with_vector()` | SQLite FTS5 keyword search through `VectorResource` | Separate from embedding RAG path |

This document describes the current code behavior, not planned roadmap behavior.

## Architecture at a glance

The production RAG flow does not use `HybridSearch` or `InMemoryVectorProvider`.

```text
request
  -> documents
  -> chunk documents
  -> embed query
  -> embed each chunk
  -> cosine similarity(query_embedding, chunk_embedding)
  -> sort descending
  -> top-K results
```

The reusable search library is separate:

```text
query + optional query embedding
  -> HybridSearch
  -> optional VectorSearchProvider + optional FTSProvider
  -> normalize provider scores
  -> merge/dedup
  -> top-K SearchResult
```

The FTS5 keyword store is also separate:

```text
context.with_vector(SimpleVectorResource)
  -> context.vector()
  -> upsert/search/delete
  -> SQLite FTS5 keyword ranking
```

## Provider model & lifecycle

The provider traits live in `engine/src/search/providers.rs`.

| Type | Purpose |
| --- | --- |
| `SearchResult` | `{ id: String, score: f64, content: String, metadata: HashMap<String, Value> }` |
| `SearchError` | `ProviderError(String)` or `NoResults` |
| `VectorSearchProvider` | Async vector search over a query embedding |
| `FTSProvider` | Async keyword/full-text search over a query string |

```rust
async fn search(
    &self,
    query_embedding: &[f64],
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError>;
```

```rust
async fn search(
    &self,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError>;
```

`engine/src/search/mod.rs` re-exports:

| Export |
| --- |
| `HybridSearch` |
| `cosine_similarity` |
| `FTSProvider` |
| `InMemoryVectorProvider` |
| `SearchError` |
| `SearchResult` |
| `VectorSearchProvider` |

The shared `cosine_similarity(a, b) -> f64` returns `0.0` if the vectors have different lengths, either vector is empty, or either vector has zero magnitude. Otherwise it returns `dot / (|a||b|)`, with a possible range of `-1.0..1.0`.

## In-memory vector provider

`InMemoryVectorProvider` is defined in `engine/src/search/providers.rs`. It stores documents as:

```text
Vec<(id, Vec<f64> embedding, content, metadata)>
```

| API | Behavior |
| --- | --- |
| `new()` / `Default` | Creates an empty provider |
| `add_document(id, embedding, content, metadata)` | Adds a document and its embedding |
| `search(query_embedding, limit)` | Scores every document with cosine similarity, sorts descending, truncates to `limit` |

If the provider is empty, `search` returns `SearchError::NoResults`.

This provider is brute-force, in-memory, and has no persistence. It is currently library-only and is only used by its own unit tests. It is not wired into the HTTP endpoint, agent tool, or CLI command.

## Hybrid search & scoring

`HybridSearch` is defined in `engine/src/search/hybrid.rs`.

| Field | Default / Meaning |
| --- | --- |
| `vector` | Optional `Box<dyn VectorSearchProvider>` |
| `fts` | Optional `Box<dyn FTSProvider>` |
| `vector_weight` | `0.7` |
| `fts_weight` | `0.3` |

Builder methods:

| Method | Behavior |
| --- | --- |
| `new()` | Creates an empty hybrid search with default weights |
| `with_vector(provider)` | Adds a vector provider |
| `with_fts(provider)` | Adds an FTS provider |
| `with_weights(vw, fw)` | Re-normalizes weights so `vw + fw == 1.0` when total is greater than `0` |

Search signature:

```rust
search(
    query: &str,
    query_embedding: Option<&[f64]>,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError>
```

Behavior:

| Step | Behavior |
| --- | --- |
| Vector search | Runs only if both a vector provider and `query_embedding` are present; fetches `limit * 2` |
| FTS search | Runs if an FTS provider is present; fetches `limit * 2` |
| Normalization | Min-max normalizes each result set's scores to `0..1` |
| Equal scores | `normalize_scores` maps all equal scores to `1.0` |
| Empty scores | `normalize_scores` returns an empty map |
| Merge | Deduplicates by `id`; content and metadata come from whichever provider had the doc |
| Scoring | If both sources return results, `total = vector_weight * norm_vec + fts_weight * norm_fts`; otherwise the single available source's normalized score is used directly |
| Final result | Sorts descending and truncates to `limit` |
| No results | Returns `SearchError::NoResults` if neither provider returns ids |

`HybridSearch` is currently library-only and is only exercised by unit tests. It is not wired into the HTTP endpoint, agent tool, or CLI command.

## The RAG pipeline (rag.rs): config, source types, chunking strategies, chunkers, formats

`engine/src/rag.rs` contains the shared RAG data types and chunking helpers.

### Config

`RAGPipelineConfig` fields:

| Field | Default |
| --- | --- |
| `name: String` | No listed default |
| `source_type: SourceType` | No listed default |
| `chunking_strategy: ChunkingStrategy` | `Paragraph` |
| `chunk_size: usize` | `512` |
| `chunk_overlap: usize` | `50` |
| `embedding_model: String` | `"text-embedding-3-small"` |

### Source types

`SourceType` uses serde snake_case.

| Variant |
| --- |
| `File` |
| `Url` |
| `Directory` |
| `Text` |

### Chunking strategies

| Strategy | Behavior |
| --- | --- |
| `FixedSize` | Character window chunking |
| `Sentence` | Sentence grouping |
| `Paragraph` | Paragraph grouping; default |
| `Semantic` | Not implemented; falls back to paragraph chunking |

`chunk_text(text, &config) -> Vec<String>` dispatches based on the configured strategy.

### Data structs

| Type | Fields |
| --- | --- |
| `Chunk` | `id`, `text`, `source`, `chunk_index`, `metadata` |
| `RAGSearchResult` | `chunk: Chunk`, `score: f64` |

### Chunkers

| Function | Behavior |
| --- | --- |
| `chunk_fixed_size(text, size, overlap)` | Character-based sliding window. `step = size.saturating_sub(overlap).max(1)`. Skips whitespace-only chunks. Empty text or size `0` returns empty. |
| `chunk_by_sentence(text, max_chars)` | Splits on `.`, `!`, and `?`, keeping punctuation inclusive. Groups sentences until `max_chars`. |
| `chunk_by_paragraph(text, max_chars)` | Splits on `\n\n`. Groups paragraphs until `max_chars`. |

### Formats and file reads

`supported_formats()` returns:

```text
[".txt", ".md", ".json", ".csv", ".html"]
```

`read_file_for_rag(path) -> Result<String, String>` validates the extension against `supported_formats()` and reads the file to a string. This is used for file ingestion helpers; the HTTP and agent-tool RAG paths take inline text documents, not files.

## Embeddings (how they are provided)

Embeddings are supplied through `LLMResource::embed` in `engine/src/core/context.rs`:

```rust
async fn embed(&self, text: &str, model: &str) -> Result<Vec<f64>, ResourceError>;
```

OpenMirai uses a two-layer design described in `PRIMITIVES.md`:

| Layer | Meaning |
| --- | --- |
| `LLMAdapter` | How to talk to a provider |
| `LLMResource` | What tools and the runner need: `call` and `embed` |

Tools do not see provider-specific details.

| Implementation | Embedding Behavior |
| --- | --- |
| Ollama | Real embeddings. Sends `POST {base_url}/api/embed` with `{ model, input }`, uses a 30s timeout, and parses `embeddings[0]`. Empty model means the adapter's `default_model`. Errors on non-200, invalid JSON, or missing/empty vector. |
| Mock | Deterministic test embeddings. Maps SHA256 of the text to `embedding_dim` f64 values in `[-1, 1]`. |
| AdapterBridge / default `ExecutionContext` | Returns an error when embeddings are not configured. |

An empty model string conventionally means "use the provider default model".

The practical effect is that production RAG requires a provider that implements `embed()`, such as Ollama with an embedding model available. If embeddings are not configured, embed calls fail and the RAG endpoint/tool returns an error.

## HTTP surface (POST /api/v1/rag/search)

Route: `POST /api/v1/rag/search`

Handler: `engine/src/server/helpers.rs::rag_search`

When server authentication is configured, the endpoint requires an `X-API-Key`
header, as described in `API.md`.

### Request body

| Field | Required | Default | Notes |
| --- | --- | --- | --- |
| `query` | Yes | None | Must be a non-empty string |
| `documents` | Yes | None | Array of strings; missing or empty is an error |
| `top_k` | No | `3` | Number of results to return |
| `chunk_strategy` | No | `"paragraph"` | Accepts `"fixed_size"`, `"sentence"`, otherwise paragraph |
| `chunk_size` | No | `512` | Maximum chunk size for configured chunker |

HTTP-specific fixed values:

| Value | Behavior |
| --- | --- |
| `chunk_overlap` | Hardcoded to `50`; not a request parameter |
| `embedding_model` | Hardcoded to `"nomic-embed-text"`; not a request parameter |

### curl example

Use the URL for the running server. The repo API docs use localhost examples.

```bash
curl -X POST http://localhost:3000/api/v1/rag/search \
  -H "Content-Type: application/json" \
  -H "X-API-Key: $MIRAI_API_KEY" \
  -d '{
    "query": "how do I configure the server",
    "documents": [
      "The server is configured via environment variables…",
      "Authentication uses an X-API-Key header…"
    ],
    "top_k": 3,
    "chunk_strategy": "paragraph",
    "chunk_size": 512
  }'
```

### Success response

```json
{
  "results": [
    {
      "chunk": "Authentication uses an X-API-Key header…",
      "score": 0.83,
      "index": 1
    }
  ],
  "chunks_total": 2,
  "query": "how do I configure the server",
  "embedding_model": "nomic-embed-text",
  "dimensions": 768
}
```

If no chunks are produced:

```json
{
  "results": [],
  "chunks_total": 0
}
```

### Errors

| Status | Response / Message |
| --- | --- |
| `400` | `{"error":"query is required"}` |
| `400` | `{"error":"documents array is required"}` |
| `500` | `Failed to embed query: ...` |
| `500` | `Failed to embed chunk: ...` |

### Score behavior

The handler returns raw cosine similarity scores, with possible range `-1..1`. `API.md` says scores are "normalized 0.0-1.0", but the current handler does not min-max normalize HTTP RAG scores. Only the unused `HybridSearch` library normalizes scores.

## Agent-tool surface (data/rag_search)

Tool implementation: `engine/src/tools/builtin/data/rag_search.rs`

| Field | Value |
| --- | --- |
| `tool_type` | `data/rag_search` |
| Name | `RAG Search` |
| Description | `Semantic search over documents using real embeddings. Chunks documents, generates embeddings via LLM, returns top-K by cosine similarity.` |

### Inputs

| Input | Required | Notes |
| --- | --- | --- |
| `query` | Yes | Can come from input or config |
| `documents` | Optional | Array; can come from input or config |

### Config

| Config field | Default | Notes |
| --- | --- | --- |
| `documents` | None | Alternative to input documents |
| `top_k` | `3` | Number of results |
| `chunk_strategy` | `"paragraph"` | Chunking strategy |
| `chunk_size` | `512` | Chunk size |
| `embedding_model` | `""` | Provider default |

Resolution behavior:

| Condition | Result |
| --- | --- |
| `query` missing or empty | Error: `input 'query' is required` |
| No documents from input or config | Error: `documents are required (via input or config)` |

The tool embeds with:

```rust
context.llm().embed(text, embed_model)
```

It uses its own inline cosine computation: `dot / (|a||b|)`, returning `0.0` if either magnitude is zero.

### Output

| Field | Meaning |
| --- | --- |
| `results` | Array of `{ chunk, score, index }` |
| `chunks_total` | Number of chunks searched |
| `embedding_dimensions` | Embedding vector length |

### Agent YAML node example

```yaml
nodes:
  - id: retrieve
    tool_type: data/rag_search
    config:
      top_k: 5
      chunk_strategy: paragraph
      chunk_size: 512
      documents:
        - "First reference document text…"
        - "Second reference document text…"
```

`query` is typically supplied at runtime as the node input.

## CLI surface (mirai rag search)

Implementation: `cli/src/main.rs::cmd_rag`

Command:

```bash
mirai rag search --query "how do I configure the server" --documents ./docs/backend/API.md,./docs/backend/PRIMITIVES.md --top-k 3
```

Only the `search` subcommand exists. There is no `ingest` subcommand.

| CLI behavior | Detail |
| --- | --- |
| `--documents` | Comma-separated list of file paths |
| File reads | Each file is read to string |
| Unreadable files | Warned and skipped |
| No loaded documents | Error/exit |
| Chunking | Paragraph chunking |
| Chunk size | `512` |
| Chunk overlap | `50` |
| Embedding model | `""`, meaning provider default |
| Ranking | Inline cosine similarity |
| Output | Top-K results, each truncated to 80 chars with score |

Example output shape:

```text
  [0.8421] First chunk text…
```

## Data flow: request -> retrieved result

This is the end-to-end HTTP path:

1. Client sends `POST /api/v1/rag/search` with `X-API-Key`, `query`, `documents`, and optional `top_k`, `chunk_strategy`, and `chunk_size`.
2. The handler validates `query`. An empty query returns `400 {"error":"query is required"}`.
3. The handler validates `documents`. Missing or empty documents return `400 {"error":"documents array is required"}`.
4. The handler builds a RAG config using the request chunk settings, hardcoded `chunk_overlap = 50`, and hardcoded `embedding_model = "nomic-embed-text"`.
5. Every document string is chunked, and all chunks are concatenated into one per-request chunk list.
6. If no chunks are produced, the handler returns `{"results": [], "chunks_total": 0}`.
7. The query is embedded with `(state.llm_factory)().embed(query, "nomic-embed-text")`.
8. Each chunk is embedded with `(state.llm_factory)().embed(chunk, "nomic-embed-text")`.
9. The handler computes raw cosine similarity between the query embedding and each chunk embedding.
10. Results are sorted descending by score.
11. The handler returns the top `top_k` results with `chunk`, `score`, and original chunk `index`, plus `chunks_total`, `query`, `embedding_model`, and `dimensions`.

## Current Limitations / production caveats

| Limitation | Current Behavior |
| --- | --- |
| Stateless / no persistence | Every request re-chunks and re-embeds all documents inline. Nothing is indexed, cached, or stored between requests. |
| O(N) embedding calls | Each query embeds the query plus every produced chunk. |
| No exposed hybrid retrieval | `HybridSearch` and `InMemoryVectorProvider` are not wired into HTTP/tool/CLI; they are library/test-only. |
| Separate FTS5 path | `SimpleVectorResource` is keyword-only and is not connected to the embedding RAG path. |
| Embedding-model inconsistency | `rag.rs` defaults to `"text-embedding-3-small"`; HTTP hardcodes `"nomic-embed-text"`; the tool and CLI pass `""` for provider default. |
| Semantic chunking | `Semantic` exists but is not implemented; it falls back to paragraph chunking. |
| Brute-force scoring | Cosine similarity is computed over every chunk; there is no ANN or vector index. This is only practical for small per-request document sets. |
| Embedding provider required | A provider implementing `embed()` is required. An unconfigured/default context returns embed errors, causing endpoint/tool failures. |
| Source types exceed runtime ingestion | `SourceType::{Directory, Url}` exist, but these paths do not implement directory or URL ingestion. |
| No CLI ingestion | There is no `mirai rag ingest` subcommand. |
| Score normalization discrepancy | `API.md` says HTTP scores are normalized `0.0-1.0`, but the handler returns raw cosine scores in `-1..1`. Only unused `HybridSearch` normalizes scores. |

## Status vs roadmap

`docs/GAPS.md` is historical at v0.4.3. The current version referenced by the brief is v0.7.0, and `docs/ROADMAP-PARITY.md` is the current status source.

| Gap | Historical Status | Current Code Reconciliation |
| --- | --- | --- |
| GAP-I, specialized server endpoints including `rag/search` | Marked resolved in v0.4.0 | `POST /api/v1/rag/search` exists |
| GAP-H, "RAG con embeddings reales" | Listed pending with missing `context.llm().embed()`, cosine over real embeddings, and `mirai rag ingest --source ./docs/` | Items 1 and 2 are done in HTTP, `data/rag_search`, and `mirai rag search`; persistent ingestion/indexing and `mirai rag ingest` remain absent |
| GAP-005, cross-thread semantic memory `Store` | Roadmap item | Still depends on extending `VectorResource` with real embeddings |

The current production RAG path uses real `context.llm().embed()` calls and cosine similarity. It does not provide persistent ingestion, indexing, or a `mirai rag ingest` command.

## Testing guidance & important test files

Run focused tests with:

```bash
cargo test -p openmirai-engine rag
cargo test -p openmirai-engine search
```

Important test coverage:

| File | Tests |
| --- | --- |
| `engine/src/rag.rs` | `chunk_fixed_size_basic`, `chunk_fixed_size_no_overlap`, `chunk_fixed_size_empty`, `chunk_by_sentence_basic`, `chunk_by_paragraph_basic`, `chunk_text_uses_config`, `supported_formats_list`, `config_serde_roundtrip` |
| `engine/src/search/providers.rs` | `test_cosine_similarity_*` for identical, orthogonal, opposite, empty, length mismatch, and zero vector cases; `test_in_memory_vector_search`; `test_in_memory_vector_search_empty` |
| `engine/src/search/hybrid.rs` | `test_hybrid_vector_only`, `test_hybrid_fts_only`, `test_hybrid_combined`, `test_hybrid_no_providers`, `test_normalize_scores*`, `test_with_weights_normalization` |
| `engine/src/adapters/simple_vector.rs` | `upsert_and_search`, `upsert_updates_existing`, `delete_removes_from_search`, `empty_search_returns_empty`, `search_respects_limit` |
| `engine/src/adapters/mock_llm.rs` | `embed_deterministic`, `embed_values_in_range` |

See also `../TESTS.md`.

## Related docs

| Doc | Notes |
| --- | --- |
| [API.md](./API.md) | `POST /api/v1/rag/search` endpoint reference |
| [PRIMITIVES.md](./PRIMITIVES.md) | `LLMResource`/`embed`, `ExecutionContext`, `VectorResource` |
| [STORAGE.md](../database/STORAGE.md) | Storage and vector resources |
| [GAPS.md](../GAPS.md) | GAP-H and GAP-I |
| [ROADMAP-PARITY.md](../ROADMAP-PARITY.md) | GAP-005 cross-thread semantic memory |
