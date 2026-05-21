//! Search provider traits and in-memory implementations.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// A single search result with score and metadata.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f64,
    pub content: String,
    pub metadata: HashMap<String, Value>,
}

/// Errors returned by search providers.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("no results found")]
    NoResults,
}

/// Abstract vector (embedding) search provider.
#[async_trait]
pub trait VectorSearchProvider: Send + Sync {
    async fn search(
        &self,
        query_embedding: &[f64],
        limit: usize,
    ) -> Result<Vec<SearchResult>, SearchError>;
}

/// Abstract full-text search provider.
#[async_trait]
pub trait FTSProvider: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError>;
}

/// Cosine similarity between two vectors. Returns 0.0 if either vector has zero magnitude.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        mag_a += x * x;
        mag_b += y * y;
    }

    let denom = mag_a.sqrt() * mag_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// In-memory vector search using brute-force cosine similarity.
pub struct InMemoryVectorProvider {
    /// (id, embedding, content, metadata)
    documents: Vec<(String, Vec<f64>, String, HashMap<String, Value>)>,
}

impl InMemoryVectorProvider {
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
        }
    }

    /// Add a document with its embedding to the index.
    pub fn add_document(
        &mut self,
        id: String,
        embedding: Vec<f64>,
        content: String,
        metadata: HashMap<String, Value>,
    ) {
        self.documents.push((id, embedding, content, metadata));
    }
}

impl Default for InMemoryVectorProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VectorSearchProvider for InMemoryVectorProvider {
    async fn search(
        &self,
        query_embedding: &[f64],
        limit: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if self.documents.is_empty() {
            return Err(SearchError::NoResults);
        }

        let mut scored: Vec<SearchResult> = self
            .documents
            .iter()
            .map(|(id, emb, content, meta)| {
                let score = cosine_similarity(query_embedding, emb);
                SearchResult {
                    id: id.clone(),
                    score,
                    content: content.clone(),
                    metadata: meta.clone(),
                }
            })
            .collect();

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        if scored.is_empty() {
            Err(SearchError::NoResults)
        } else {
            Ok(scored)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_length_mismatch() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[tokio::test]
    async fn test_in_memory_vector_search() {
        let mut provider = InMemoryVectorProvider::new();
        provider.add_document(
            "doc1".into(),
            vec![1.0, 0.0, 0.0],
            "first".into(),
            HashMap::new(),
        );
        provider.add_document(
            "doc2".into(),
            vec![0.0, 1.0, 0.0],
            "second".into(),
            HashMap::new(),
        );
        provider.add_document(
            "doc3".into(),
            vec![0.9, 0.1, 0.0],
            "third".into(),
            HashMap::new(),
        );

        let query = vec![1.0, 0.0, 0.0];
        let results = provider.search(&query, 2).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "doc1"); // Exact match scores highest
        assert_eq!(results[1].id, "doc3"); // Close match
    }

    #[tokio::test]
    async fn test_in_memory_vector_search_empty() {
        let provider = InMemoryVectorProvider::new();
        let result = provider.search(&[1.0, 0.0], 5).await;
        assert!(result.is_err());
    }
}
