//! RAG Pipeline — Retrieval Augmented Generation.
//!
//! Manages the full pipeline: Ingest → Chunk → Embed → Store → Retrieve.
//! Uses the existing vector search infrastructure for storage.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Source type for RAG ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    File,
    Url,
    Directory,
    Text,
}

/// Chunking strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ChunkingStrategy {
    FixedSize,
    Sentence,
    #[default]
    Paragraph,
    Semantic,
}

/// RAG pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGPipelineConfig {
    pub name: String,
    pub source_type: SourceType,
    #[serde(default)]
    pub chunking_strategy: ChunkingStrategy,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_overlap")]
    pub chunk_overlap: usize,
    #[serde(default = "default_embed_model")]
    pub embedding_model: String,
}

fn default_chunk_size() -> usize {
    512
}
fn default_overlap() -> usize {
    50
}
fn default_embed_model() -> String {
    "text-embedding-3-small".into()
}

/// A single chunk of text from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub text: String,
    pub source: String,
    pub chunk_index: usize,
    #[serde(default)]
    pub metadata: Value,
}

/// A search result from the RAG pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGSearchResult {
    pub chunk: Chunk,
    pub score: f64,
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split text into chunks based on the configured strategy.
pub fn chunk_text(text: &str, config: &RAGPipelineConfig) -> Vec<String> {
    match config.chunking_strategy {
        ChunkingStrategy::FixedSize => {
            chunk_fixed_size(text, config.chunk_size, config.chunk_overlap)
        }
        ChunkingStrategy::Sentence => chunk_by_sentence(text, config.chunk_size),
        ChunkingStrategy::Paragraph => chunk_by_paragraph(text, config.chunk_size),
        ChunkingStrategy::Semantic => {
            // Semantic chunking requires embeddings — fall back to paragraph.
            chunk_by_paragraph(text, config.chunk_size)
        }
    }
}

/// Fixed-size chunking with overlap.
fn chunk_fixed_size(text: &str, size: usize, overlap: usize) -> Vec<String> {
    if text.is_empty() || size == 0 {
        return vec![];
    }
    let chars: Vec<char> = text.chars().collect();
    let step = size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + size).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        start += step;
    }
    chunks
}

/// Sentence-based chunking: group sentences until chunk_size chars.
fn chunk_by_sentence(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for sentence in text.split_inclusive(['.', '!', '?']) {
        if current.len() + sentence.len() > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(sentence);
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

/// Paragraph-based chunking: group paragraphs until chunk_size chars.
fn chunk_by_paragraph(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if current.len() + para.len() + 2 > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

/// Supported file formats for RAG ingestion.
pub fn supported_formats() -> Vec<&'static str> {
    vec![".txt", ".md", ".json", ".csv", ".html"]
}

/// Read a file and return its text content for chunking.
pub fn read_file_for_rag(path: &str) -> Result<String, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    if !supported_formats().contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported format: {ext}. Supported: {:?}",
            supported_formats()
        ));
    }

    std::fs::read_to_string(path).map_err(|e| format!("Failed to read {path}: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_fixed_size_basic() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        let chunks = chunk_fixed_size(text, 10, 2);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].len(), 10);
    }

    #[test]
    fn chunk_fixed_size_no_overlap() {
        let text = "abcdefghij1234567890";
        let chunks = chunk_fixed_size(text, 10, 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "abcdefghij");
        assert_eq!(chunks[1], "1234567890");
    }

    #[test]
    fn chunk_fixed_size_empty() {
        let chunks = chunk_fixed_size("", 10, 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_by_sentence_basic() {
        let text = "First sentence. Second sentence. Third sentence. Fourth sentence.";
        let chunks = chunk_by_sentence(text, 40);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn chunk_by_paragraph_basic() {
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = chunk_by_paragraph(text, 30);
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn chunk_text_uses_config() {
        let config = RAGPipelineConfig {
            name: "test".into(),
            source_type: SourceType::Text,
            chunking_strategy: ChunkingStrategy::FixedSize,
            chunk_size: 5,
            chunk_overlap: 0,
            embedding_model: "test".into(),
        };
        let chunks = chunk_text("hello world!", &config);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn supported_formats_list() {
        let formats = supported_formats();
        assert!(formats.contains(&".txt"));
        assert!(formats.contains(&".md"));
        assert!(formats.contains(&".json"));
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = RAGPipelineConfig {
            name: "my-rag".into(),
            source_type: SourceType::Directory,
            chunking_strategy: ChunkingStrategy::Semantic,
            chunk_size: 1024,
            chunk_overlap: 100,
            embedding_model: "text-embedding-3-small".into(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: RAGPipelineConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.chunk_size, 1024);
        assert_eq!(back.chunking_strategy, ChunkingStrategy::Semantic);
    }
}
