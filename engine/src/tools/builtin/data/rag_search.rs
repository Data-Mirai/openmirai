//! RagSearchTool — semantic search with REAL embeddings

use super::*;

// ===========================================================================
// RagSearchTool — semantic search with REAL embeddings
// ===========================================================================

data_tool! {
    struct RagSearchTool, factory RagSearchFactory;
    tool_type = "data/rag_search",
    name = "RAG Search",
    description = "Semantic search over documents using real embeddings. Chunks documents, generates embeddings via LLM, returns top-K by cosine similarity.",
    inputs = [
        field("query", FieldType::String, true, "Search query"),
        field("documents", FieldType::Array, false, "Array of text documents to search. If not provided, uses config."),
    ],
    outputs = [
        field("results", FieldType::Array, true, "Ranked chunks with scores"),
        field("chunks_total", FieldType::Number, true, "Total chunks generated"),
        field("embedding_dimensions", FieldType::Number, true, "Embedding vector dimensions"),
    ],
    config_fields = [
        field("documents", FieldType::Array, false, "Static documents to search (alternative to input)"),
        field("top_k", FieldType::Number, false, "Number of results (default 3)"),
        field("chunk_strategy", FieldType::String, false, "Chunking: paragraph, sentence, fixed_size (default paragraph)"),
        field("chunk_size", FieldType::Number, false, "Max chars per chunk (default 512)"),
        field("embedding_model", FieldType::String, false, "Embedding model (default: provider default)"),
    ]
}

#[async_trait]
impl Tool for RagSearchTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let query = inputs.get("query")
            .and_then(|v| v.as_str())
            .or_else(|| config.get("query").and_then(|v| v.as_str()))
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: "input 'query' is required".into(),
            })?;

        // Get documents from input or config.
        let docs: Vec<String> = inputs.get("documents")
            .or_else(|| config.get("documents"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if docs.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: "documents are required (via input or config)".into(),
            });
        }

        let top_k = config.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
        let chunk_strategy = config.get("chunk_strategy").and_then(|v| v.as_str()).unwrap_or("paragraph");
        let chunk_size = config.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(512) as usize;
        let embed_model = config.get("embedding_model").and_then(|v| v.as_str()).unwrap_or("");

        // Chunk documents.
        let rag_config = crate::rag::RAGPipelineConfig {
            name: "tool".into(),
            source_type: crate::rag::SourceType::Text,
            chunking_strategy: match chunk_strategy {
                "sentence" => crate::rag::ChunkingStrategy::Sentence,
                "fixed_size" => crate::rag::ChunkingStrategy::FixedSize,
                _ => crate::rag::ChunkingStrategy::Paragraph,
            },
            chunk_size,
            chunk_overlap: 50,
            embedding_model: embed_model.to_string(),
        };

        let mut all_chunks: Vec<String> = Vec::new();
        for doc in &docs {
            all_chunks.extend(crate::rag::chunk_text(doc, &rag_config));
        }

        if all_chunks.is_empty() {
            let mut out = HashMap::new();
            out.insert("results".to_string(), json!([]));
            out.insert("chunks_total".to_string(), json!(0));
            out.insert("embedding_dimensions".to_string(), json!(0));
            return Ok(out);
        }

        // Generate REAL embeddings via context.llm().embed().
        let query_emb = context.llm().embed(query, embed_model).await.map_err(|e| {
            ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: format!("Failed to embed query: {e}"),
            }
        })?;

        let mut chunk_embs = Vec::new();
        for chunk in &all_chunks {
            let emb = context.llm().embed(chunk, embed_model).await.map_err(|e| {
                ToolError::ExecutionFailed {
                    tool_type: "data/rag_search".into(),
                    message: format!("Failed to embed chunk: {e}"),
                }
            })?;
            chunk_embs.push(emb);
        }

        // Cosine similarity ranking.
        let mut scored: Vec<(usize, f64)> = chunk_embs.iter().enumerate().map(|(i, emb)| {
            let dot: f64 = query_emb.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
            let mag_a: f64 = query_emb.iter().map(|x| x * x).sum::<f64>().sqrt();
            let mag_b: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();
            let sim = if mag_a > 0.0 && mag_b > 0.0 { dot / (mag_a * mag_b) } else { 0.0 };
            (i, sim)
        }).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<Value> = scored.iter().take(top_k).map(|(i, score)| {
            json!({"chunk": all_chunks[*i], "score": score, "index": i})
        }).collect();

        let dims = query_emb.len();

        let mut out = HashMap::new();
        out.insert("results".to_string(), json!(results));
        out.insert("chunks_total".to_string(), json!(all_chunks.len()));
        out.insert("embedding_dimensions".to_string(), json!(dims));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

