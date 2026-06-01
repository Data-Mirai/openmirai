# RAG y Búsqueda

## Propósito

OpenMirai cuenta con un subsistema RAG/búsqueda para recuperar fragmentos de texto relevantes de los documentos proporcionados. Las rutas de producción actuales de RAG son sin estado: cada solicitud o comando fragmenta los documentos proporcionados, incrusta cada fragmento y la consulta con un proveedor LLM, puntúa los fragmentos con similitud de coseno y devuelve los mejores resultados.

Existen tres rutas de código de búsqueda en gran medida desconectadas:

| Ruta | Superficie de Tiempo de Ejecución | Qué hace | Estado de Integración |
| --- | --- | --- | --- |
| Ruta de producción RAG | `POST /api/v1/rag/search`, `data/rag_search`, `mirai rag search` | Fragmentar -> incrustar consulta y fragmentos con un LLM real -> similitud de coseno por fuerza bruta -> top-K | Ruta de tiempo de ejecución activa |
| Librería de proveedor de búsqueda | `engine/src/search/` | `HybridSearch`, `InMemoryVectorProvider`, `VectorSearchProvider`, `FTSProvider` | Solo librería/prueba; no conectado a HTTP/herramienta/CLI |
| Almacén de palabras clave FTS5 | `SimpleVectorResource` vía `context.with_vector()` | Búsqueda de palabras clave SQLite FTS5 a través de `VectorResource` | Separado de la ruta RAG de incrustación |

Este documento describe el comportamiento actual del código, no el comportamiento planificado en la hoja de ruta.

## Arquitectura de un vistazo

El flujo de RAG de producción no utiliza `HybridSearch` o `InMemoryVectorProvider`.

```text
solicitud
  -> documentos
  -> fragmentar documentos
  -> incrustar consulta
  -> incrustar cada fragmento
  -> similitud de coseno(embedding_consulta, embedding_fragmento)
  -> ordenar descendente
  -> resultados top-K
```

La biblioteca de búsqueda reutilizable es independiente:

```text
query + optional query embedding
  -> HybridSearch
  -> optional VectorSearchProvider + optional FTSProvider
  -> normalizar puntuaciones de proveedor
  -> fusionar/eliminar duplicados
  -> SearchResult top-K
```

El almacén de palabras clave FTS5 también es independiente:

```text
context.with_vector(SimpleVectorResource)
  -> context.vector()
  -> upsert/search/delete
  -> clasificación de palabras clave SQLite FTS5
```

## Modelo y ciclo de vida del proveedor

Los traits del proveedor residen en `engine/src/search/providers.rs`.

| Tipo | Propósito |
| --- | --- |
| `SearchResult` | `{ id: String, score: f64, content: String, metadata: HashMap<String, Value> }` |
| `SearchError` | `ProviderError(String)` o `NoResults` |
| `VectorSearchProvider` | Búsqueda vectorial asíncrona sobre un embedding de consulta |
| `FTSProvider` | Búsqueda asíncrona de palabras clave/texto completo sobre una cadena de consulta |

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

`engine/src/search/mod.rs` re-exporta:

| Exportación |
| --- |
| `HybridSearch` |
| `cosine_similarity` |
| `FTSProvider` |
| `InMemoryVectorProvider` |
| `SearchError` |
| `SearchResult` |
| `VectorSearchProvider` |

La `cosine_similarity(a, b) -> f64` compartida devuelve `0.0` si los vectores tienen longitudes diferentes, si alguno de los vectores está vacío o si alguno de los vectores tiene magnitud cero. De lo contrario, devuelve `dot / (|a||b|)`, con un rango posible de `-1.0..1.0`.

## Proveedor de vectores en memoria

`InMemoryVectorProvider` se define en `engine/src/search/providers.rs`. Almacena documentos como:

```text
Vec<(id, Vec<f64> embedding, content, metadata)>
```

| API | Comportamiento |
| --- | --- |
| `new()` / `Default` | Crea un proveedor vacío |
| `add_document(id, embedding, content, metadata)` | Agrega un documento y su embedding |
| `search(query_embedding, limit)` | Puntúa cada documento con similitud de coseno, ordena descendentemente, trunca a `limit` |

Si el proveedor está vacío, `search` devuelve `SearchError::NoResults`.

Este proveedor es de fuerza bruta, en memoria y no tiene persistencia. Actualmente es solo de librería y solo se utiliza en sus propias pruebas unitarias. No está conectado al endpoint HTTP, la herramienta de agente o el comando CLI.

## Búsqueda híbrida y puntuación

`HybridSearch` se define en `engine/src/search/hybrid.rs`.

| Campo | Predeterminado / Significado |
| --- | --- |
| `vector` | `Box<dyn VectorSearchProvider>` opcional |
| `fts` | `Box<dyn FTSProvider>` opcional |
| `vector_weight` | `0.7` |
| `fts_weight` | `0.3` |

Métodos de construcción:

| Método | Comportamiento |
| --- | --- |
| `new()` | Crea una búsqueda híbrida vacía con pesos predeterminados |
| `with_vector(provider)` | Agrega un proveedor de vectores |
| `with_fts(provider)` | Agrega un proveedor FTS |
| `with_weights(vw, fw)` | Re-normaliza los pesos para que `vw + fw == 1.0` cuando el total sea mayor que `0` |

Firma de búsqueda:

```rust
search(
    query: &str,
    query_embedding: Option<&[f64]>,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError>
```

Comportamiento:

| Paso | Comportamiento |
| --- | --- |
| Búsqueda vectorial | Se ejecuta solo si están presentes un proveedor vectorial y `query_embedding`; recupera `limit * 2` |
| Búsqueda FTS | Se ejecuta si está presente un proveedor FTS; recupera `limit * 2` |
| Normalización | Normaliza Min-Max las puntuaciones de cada conjunto de resultados a `0..1` |
| Puntuaciones iguales | `normalize_scores` mapea todas las puntuaciones iguales a `1.0` |
| Puntuaciones vacías | `normalize_scores` devuelve un mapa vacío |
| Fusión | Desduplica por `id`; el contenido y los metadatos provienen del proveedor que tenía el documento |
| Puntuación | Si ambas fuentes devuelven resultados, `total = vector_weight * norm_vec + fts_weight * norm_fts`; de lo contrario, se utiliza directamente la puntuación normalizada de la única fuente disponible |
| Resultado final | Ordena descendentemente y trunca a `limit` |
| Sin resultados | Devuelve `SearchError::NoResults` si ninguno de los proveedores devuelve ids |

`HybridSearch` actualmente es solo de librería y solo se ejerce mediante pruebas unitarias. No está conectado al endpoint HTTP, la herramienta de agente o el comando CLI.

## La pipeline RAG (rag.rs): configuración, tipos de fuente, estrategias de fragmentación, fragmentadores, formatos

`engine/src/rag.rs` contiene los tipos de datos RAG compartidos y las funciones de ayuda para la fragmentación.

### Configuración

Campos de `RAGPipelineConfig`:

| Campo | Predeterminado |
| --- | --- |
| `name: String` | No se lista predeterminado |
| `source_type: SourceType` | No se lista predeterminado |
| `chunking_strategy: ChunkingStrategy` | `Paragraph` |
| `chunk_size: usize` | `512` |
| `chunk_overlap: usize` | `50` |
| `embedding_model: String` | `"text-embedding-3-small"` |

### Tipos de fuente

`SourceType` utiliza serde snake_case.

| Variante |
| --- |
| `File` |
| `Url` |
| `Directory` |
| `Text` |

### Estrategias de fragmentación

| Estrategia | Comportamiento |
| --- | --- |
| `FixedSize` | Fragmentación de ventana de caracteres |
| `Sentence` | Agrupación de oraciones |
| `Paragraph` | Agrupación de párrafos; predeterminado |
| `Semantic` | No implementado; vuelve a la fragmentación por párrafo |

`chunk_text(text, &config) -> Vec<String>` despacha según la estrategia configurada.

### Estructuras de datos

| Tipo | Campos |
| --- | --- |
| `Chunk` | `id`, `text`, `source`, `chunk_index`, `metadata` |
| `RAGSearchResult` | `chunk: Chunk`, `score: f64` |

### Fragmentadores

| Función | Comportamiento |
| --- | --- |
| `chunk_fixed_size(text, size, overlap)` | Ventana deslizante basada en caracteres. `step = size.saturating_sub(overlap).max(1)`. Omite fragmentos que solo contienen espacios en blanco. Texto vacío o tamaño `0` devuelve vacío. |
| `chunk_by_sentence(text, max_chars)` | Divide por `.`, `!`, y `?`, incluyendo la puntuación. Agrupa oraciones hasta `max_chars`. |
| `chunk_by_paragraph(text, max_chars)` | Divide por `

`. Agrupa párrafos hasta `max_chars`. |

### Formatos y lecturas de archivos

`supported_formats()` devuelve:

```text
[".txt", ".md", ".json", ".csv", ".html"]
```

`read_file_for_rag(path) -> Result<String, String>` valida la extensión contra `supported_formats()` y lee el archivo a una cadena. Esto se utiliza para las funciones de ayuda de ingesta de archivos; las rutas RAG HTTP y de herramientas de agente toman documentos de texto en línea, no archivos.

## Embeddings (cómo se proporcionan)

Los embeddings se proporcionan a través de `LLMResource::embed` en `engine/src/core/context.rs`:

```rust
async fn embed(&self, text: &str, model: &str) -> Result<Vec<f64>, ResourceError>;
```

OpenMirai utiliza un diseño de dos capas descrito en `PRIMITIVES.md`:

| Capa | Significado |
| --- | --- |
| `LLMAdapter` | Cómo comunicarse con un proveedor |
| `LLMResource` | Lo que las herramientas y el ejecutor necesitan: `call` y `embed` |

Las herramientas no ven detalles específicos del proveedor.

| Implementación | Comportamiento de Embedding |
| --- | --- |
| Ollama | Embeddings reales. Envía `POST {base_url}/api/embed` con `{ model, input }`, usa un tiempo de espera de 30s y analiza `embeddings[0]`. Un modelo vacío significa el `default_model` del adaptador. Errores en no-200, JSON inválido o vector faltante/vacío. |
| Mock | Embeddings de prueba deterministas. Mapea SHA256 del texto a `embedding_dim` valores f64 en `[-1, 1]`. |
| AdapterBridge / `ExecutionContext` predeterminado | Devuelve un error cuando los embeddings no están configurados. |

Una cadena de modelo vacía significa convencionalmente "usar el modelo predeterminado del proveedor".

El efecto práctico es que la producción de RAG requiere un proveedor que implemente `embed()`, como Ollama con un modelo de embedding disponible. Si los embeddings no están configurados, las llamadas a `embed` fallan y el endpoint/herramienta RAG devuelve un error.

## Superficie HTTP (POST /api/v1/rag/search)

Ruta: `POST /api/v1/rag/search`

Handler: `engine/src/server/helpers.rs::rag_search`

El endpoint requiere un encabezado de autorización `X-API-Key`, como se describe en `API.md`.

### Cuerpo de la solicitud

| Campo | Requerido | Predeterminado | Notas |
| --- | --- | --- | --- |
| `query` | Sí | Ninguno | Debe ser una cadena no vacía |
| `documents` | Sí | Ninguno | Array de cadenas; faltante o vacío es un error |
| `top_k` | No | `3` | Número de resultados a devolver |
| `chunk_strategy` | No | `"paragraph"` | Acepta `"fixed_size"`, `"sentence"`; de lo contrario, párrafo |
| `chunk_size` | No | `512` | Tamaño máximo de fragmento para el fragmentador configurado |

Valores fijos específicos de HTTP:

| Valor | Comportamiento |
| --- | --- |
| `chunk_overlap` | Codificado a `50`; no es un parámetro de solicitud |
| `embedding_model` | Codificado a `"nomic-embed-text"`; no es un parámetro de solicitud |

### Ejemplo de curl

Utilice la URL del servidor en ejecución. Los documentos de la API del repositorio utilizan ejemplos de localhost.

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

### Respuesta de éxito

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

Si no se producen fragmentos:

```json
{
  "results": [],
  "chunks_total": 0
}
```

### Errores

| Estado | Respuesta / Mensaje |
| --- | --- |
| `400` | `{"error":"query is required"}` |
| `400` | `{"error":"documents array is required"}` |
| `500` | `Failed to embed query: ...` |
| `500` | `Failed to embed chunk: ...` |

### Comportamiento de la puntuación

El manejador devuelve puntuaciones de similitud de coseno en bruto, con un rango posible de `-1..1`. `API.md` dice que las puntuaciones están "normalizadas 0.0-1.0", pero el manejador actual no normaliza min-max las puntuaciones RAG HTTP. Solo la librería `HybridSearch`, que no se utiliza, normaliza las puntuaciones.

## Superficie de la herramienta de agente (data/rag_search)

Implementación de la herramienta: `engine/src/tools/builtin/data/rag_search.rs`

| Campo | Valor |
| --- | --- |
| `tool_type` | `data/rag_search` |
| Nombre | `Búsqueda RAG` |
| Descripción | `Búsqueda semántica sobre documentos utilizando embeddings reales. Fragmenta documentos, genera embeddings a través de LLM, devuelve el top-K por similitud de coseno.` |

### Entradas

| Entrada | Requerido | Notas |
| --- | --- | --- |
| `query` | Sí | Puede provenir de la entrada o la configuración |
| `documents` | Opcional | Array; puede provenir de la entrada o la configuración |

### Configuración

| Campo de configuración | Predeterminado | Notas |
| --- | --- | --- |
| `documents` | Ninguno | Alternativa a los documentos de entrada |
| `top_k` | `3` | Número de resultados |
| `chunk_strategy` | `"paragraph"` | Estrategia de fragmentación |
| `chunk_size` | `512` | Tamaño del fragmento |
| `embedding_model` | `""` | Predeterminado del proveedor |

Comportamiento de resolución:

| Condición | Resultado |
| --- | --- |
| `query` faltante o vacío | Error: `input 'query' is required` |
| No hay documentos de entrada o configuración | Error: `documents are required (via input or config)` |

La herramienta incrusta con:

```rust
context.llm().embed(text, embed_model)
```

Utiliza su propia computación de coseno en línea: `dot / (|a||b|)`, devolviendo `0.0` si alguna magnitud es cero.

### Salida

| Campo | Significado |
| --- | --- |
| `results` | Array de `{ chunk, score, index }` |
| `chunks_total` | Número total de fragmentos buscados |
| `embedding_dimensions` | Longitud del vector de embedding |

### Ejemplo de nodo YAML de agente

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

`query` se suministra típicamente en tiempo de ejecución como la entrada del nodo.

## Superficie CLI (mirai rag search)

Implementación: `cli/src/main.rs::cmd_rag`

Comando:

```bash
mirai rag search --query "how do I configure the server" --documents ./docs/backend/API.md,./docs/backend/PRIMITIVES.md --top-k 3
```

Solo existe el subcomando `search`. No hay un subcomando `ingest`.

| Comportamiento de CLI | Detalle |
| --- | --- |
| `--documents` | Lista de rutas de archivo separadas por comas |
| Lecturas de archivos | Cada archivo se lee como una cadena |
| Archivos ilegibles | Se advierte y se omiten |
| No hay documentos cargados | Error/salida |
| Fragmentación | Fragmentación por párrafo |
| Tamaño de fragmento | `512` |
| Superposición de fragmentos | `50` |
| Modelo de embedding | `""`, lo que significa el valor predeterminado del proveedor |
| Clasificación | Similitud de coseno en línea |
| Salida | Resultados top-K, cada uno truncado a 80 caracteres con puntuación |

Formato de salida de ejemplo:

```text
  [0.8421] Primer texto de fragmento…
```

## Flujo de datos: solicitud -> resultado recuperado

Esta es la ruta HTTP de extremo a extremo:

1. El cliente envía `POST /api/v1/rag/search` con `X-API-Key`, `query`, `documents` y `top_k`, `chunk_strategy` y `chunk_size` opcionales.
2. El manejador valida la `query`. Una consulta vacía devuelve `400 {"error":"query is required"}`.
3. El manejador valida los `documents`. Documentos faltantes o vacíos devuelven `400 {"error":"documents array is required"}`.
4. El manejador construye una configuración RAG utilizando la configuración de fragmentos de la solicitud, `chunk_overlap = 50` codificado y `embedding_model = "nomic-embed-text"` codificado.
5. Cada cadena de documento se fragmenta y todos los fragmentos se concatenan en una lista de fragmentos por solicitud.
6. Si no se producen fragmentos, el manejador devuelve `{"results": [], "chunks_total": 0}`.
7. La consulta se incrusta con `(state.llm_factory)().embed(query, "nomic-embed-text")`.
8. Cada fragmento se incrusta con `(state.llm_factory)().embed(chunk, "nomic-embed-text")`.
9. El manejador calcula la similitud de coseno en bruto entre el embedding de la consulta y el embedding de cada fragmento.
10. Los resultados se ordenan descendentemente por puntuación.
11. El manejador devuelve los `top_k` resultados con `chunk`, `score` e `index` del fragmento original, además de `chunks_total`, `query`, `embedding_model` y `dimensions`.

## Limitaciones actuales / advertencias de producción

| Limitación | Comportamiento actual |
| --- | --- |
| Sin estado / sin persistencia | Cada solicitud vuelve a fragmentar y a incrustar todos los documentos en línea. Nada se indexa, almacena en caché o guarda entre solicitudes. |
| Llamadas a embedding O(N) | Cada consulta incrusta la consulta más cada fragmento producido. |
| No hay recuperación híbrida expuesta | `HybridSearch` y `InMemoryVectorProvider` no están conectados a HTTP/herramienta/CLI; son solo de librería/prueba. |
| Ruta FTS5 separada | `SimpleVectorResource` es solo de palabras clave y no está conectado a la ruta RAG de embedding. |
| Inconsistencia del modelo de embedding | `rag.rs` por defecto es `"text-embedding-3-small"`; HTTP codifica `"nomic-embed-text"`; la herramienta y CLI pasan `""` para el valor predeterminado del proveedor. |
| Fragmentación semántica | `Semantic` existe pero no está implementado; recurre a la fragmentación por párrafo. |
| Puntuación por fuerza bruta | La similitud de coseno se calcula sobre cada fragmento; no hay ANN o índice vectorial. Esto solo es práctico para pequeños conjuntos de documentos por solicitud. |
| Proveedor de embedding requerido | Se requiere un proveedor que implemente `embed()`. Un contexto no configurado/predeterminado devuelve errores de embedding, causando fallos en el endpoint/herramienta. |
| Los tipos de fuente exceden la ingesta en tiempo de ejecución | `SourceType::{Directory, Url}` existen, pero estas rutas no implementan la ingesta de directorios o URLs. |
| No hay ingesta CLI | No hay un subcomando `mirai rag ingest`. |
| Discrepancia en la normalización de la puntuación | `API.md` dice que las puntuaciones HTTP se normalizan `0.0-1.0`, pero el manejador devuelve puntuaciones de coseno en bruto en `-1..1`. Solo el `HybridSearch` no utilizado normaliza las puntuaciones. |

## Estado vs hoja de ruta

`docs/GAPS.md` es histórico en v0.4.3. La versión actual referenciada por el resumen es v0.6.0, y `docs/ROADMAP-PARITY.md` es la fuente de estado actual.

| Brecha | Estado Histórico | Reconciliación del Código Actual |
| --- | --- | --- |
| GAP-I, endpoints de servidor especializados incluyendo `rag/search` | Marcado como resuelto en v0.4.0 | Existe `POST /api/v1/rag/search` |
| GAP-H, "RAG con embeddings reales" | Listado pendiente con `context.llm().embed()` faltante, coseno sobre embeddings reales y `mirai rag ingest --source ./docs/` | Los ítems 1 y 2 están hechos en HTTP, `data/rag_search` y `mirai rag search`; la ingesta/indexación persistente y `mirai rag ingest` siguen ausentes |
| GAP-005, memoria semántica entre hilos `Store` | Elemento de la hoja de ruta | Todavía depende de extender `VectorResource` con embeddings reales |

La ruta de producción actual de RAG utiliza llamadas `context.llm().embed()` reales y similitud de coseno. No proporciona ingesta persistente, indexación o un comando `mirai rag ingest`.

## Guía de pruebas y archivos de prueba importantes

Ejecute pruebas enfocadas con:

```bash
cargo test -p openmirai-engine rag
cargo test -p openmirai-engine search
```

Cobertura de pruebas importantes:

| Archivo | Pruebas |
| --- | --- |
| `engine/src/rag.rs` | `chunk_fixed_size_basic`, `chunk_fixed_size_no_overlap`, `chunk_fixed_size_empty`, `chunk_by_sentence_basic`, `chunk_by_paragraph_basic`, `chunk_text_uses_config`, `supported_formats_list`, `config_serde_roundtrip` |
| `engine/src/search/providers.rs` | `test_cosine_similarity_*` para casos idénticos, ortogonales, opuestos, vacíos, con longitud diferente y vectores cero; `test_in_memory_vector_search`; `test_in_memory_vector_search_empty` |
| `engine/src/search/hybrid.rs` | `test_hybrid_vector_only`, `test_hybrid_fts_only`, `test_hybrid_combined`, `test_hybrid_no_providers`, `test_normalize_scores*`, `test_with_weights_normalization` |
| `engine/src/adapters/simple_vector.rs` | `upsert_and_search`, `upsert_updates_existing`, `delete_removes_from_search`, `empty_search_returns_empty`, `search_respects_limit` |
| `engine/src/adapters/mock_llm.rs` | `embed_deterministic`, `embed_values_in_range` |

Ver también `../TESTS.md`.

## Documentos relacionados

| Doc | Notas |
| --- | --- |
| [API.md](./API.md) | Referencia del endpoint `POST /api/v1/rag/search` |
| [PRIMITIVES.md](./PRIMITIVES.md) | `LLMResource`/`embed`, `ExecutionContext`, `VectorResource` |
| [STORAGE.md](../database/STORAGE.md) | Recursos de almacenamiento y vectoriales |
| [GAPS.md](../GAPS.md) | GAP-H y GAP-I |
| [ROADMAP-PARITY.md](../ROADMAP-PARITY.md) | Memoria semántica entre hilos GAP-005 |
