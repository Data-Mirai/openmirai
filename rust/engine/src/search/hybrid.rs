//! Hybrid search -- combines vector + FTS results using weighted score fusion.

use crate::search::providers::{FTSProvider, SearchError, SearchResult, VectorSearchProvider};
use std::collections::HashMap;

/// Hybrid search engine that merges vector and full-text search results.
///
/// Score formula: `total = (vector_weight * norm_vector_score) + (fts_weight * norm_fts_score)`
/// If only one provider is available, its scores are used directly.
pub struct HybridSearch {
    vector: Option<Box<dyn VectorSearchProvider>>,
    fts: Option<Box<dyn FTSProvider>>,
    vector_weight: f64,
    fts_weight: f64,
}

impl HybridSearch {
    /// Create with default weights (0.7 vector, 0.3 FTS).
    pub fn new() -> Self {
        Self {
            vector: None,
            fts: None,
            vector_weight: 0.7,
            fts_weight: 0.3,
        }
    }

    /// Set the vector search provider.
    pub fn with_vector(mut self, provider: Box<dyn VectorSearchProvider>) -> Self {
        self.vector = Some(provider);
        self
    }

    /// Set the full-text search provider.
    pub fn with_fts(mut self, provider: Box<dyn FTSProvider>) -> Self {
        self.fts = Some(provider);
        self
    }

    /// Override the default weights. They will be normalized so they sum to 1.0.
    pub fn with_weights(mut self, vector_w: f64, fts_w: f64) -> Self {
        let total = vector_w + fts_w;
        if total > 0.0 {
            self.vector_weight = vector_w / total;
            self.fts_weight = fts_w / total;
        }
        self
    }

    /// Execute hybrid search.
    ///
    /// - Runs vector search if a provider and `query_embedding` are available.
    /// - Runs FTS search if a provider is available.
    /// - Merges results: normalizes scores to 0-1, combines with weights, deduplicates
    ///   by id (keeping the highest combined score), and returns top `limit` results
    ///   sorted by descending score.
    pub async fn search(
        &self,
        query: &str,
        query_embedding: Option<&[f64]>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let mut vector_results: HashMap<String, SearchResult> = HashMap::new();
        let mut fts_results: HashMap<String, SearchResult> = HashMap::new();

        // Vector search
        if let (Some(provider), Some(embedding)) = (&self.vector, query_embedding) {
            if let Ok(results) = provider.search(embedding, limit * 2).await {
                for r in results {
                    vector_results.insert(r.id.clone(), r);
                }
            }
        }

        // FTS search
        if let Some(provider) = &self.fts {
            if let Ok(results) = provider.search(query, limit * 2).await {
                for r in results {
                    fts_results.insert(r.id.clone(), r);
                }
            }
        }

        let all_ids: Vec<String> = {
            let mut ids: Vec<String> = vector_results.keys().cloned().collect();
            for k in fts_results.keys() {
                if !vector_results.contains_key(k) {
                    ids.push(k.clone());
                }
            }
            ids
        };

        if all_ids.is_empty() {
            return Err(SearchError::NoResults);
        }

        // Normalize vector scores to 0-1
        let vec_scores = normalize_scores(
            &vector_results
                .iter()
                .map(|(id, r)| (id.clone(), r.score))
                .collect::<HashMap<_, _>>(),
        );

        // Normalize FTS scores to 0-1
        let fts_scores = normalize_scores(
            &fts_results
                .iter()
                .map(|(id, r)| (id.clone(), r.score))
                .collect::<HashMap<_, _>>(),
        );

        let have_vec = !vector_results.is_empty();
        let have_fts = !fts_results.is_empty();

        let mut merged: Vec<SearchResult> = all_ids
            .into_iter()
            .map(|id| {
                let v_score = vec_scores.get(&id).copied().unwrap_or(0.0);
                let f_score = fts_scores.get(&id).copied().unwrap_or(0.0);

                let total_score = if have_vec && have_fts {
                    (self.vector_weight * v_score) + (self.fts_weight * f_score)
                } else if have_vec {
                    v_score
                } else {
                    f_score
                };

                // Take content/metadata from whichever provider has them
                let source = vector_results
                    .get(&id)
                    .or_else(|| fts_results.get(&id))
                    .cloned()
                    .unwrap_or_else(|| SearchResult {
                        id: id.clone(),
                        score: 0.0,
                        content: String::new(),
                        metadata: HashMap::new(),
                    });

                SearchResult {
                    id,
                    score: total_score,
                    content: source.content,
                    metadata: source.metadata,
                }
            })
            .collect();

        merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        merged.truncate(limit);
        Ok(merged)
    }
}

impl Default for HybridSearch {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a map of scores to the 0-1 range using min-max normalization.
fn normalize_scores(scores: &HashMap<String, f64>) -> HashMap<String, f64> {
    if scores.is_empty() {
        return HashMap::new();
    }

    let min = scores.values().cloned().fold(f64::INFINITY, f64::min);
    let max = scores.values().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range == 0.0 {
        // All scores are equal -- map to 1.0
        scores.iter().map(|(k, _)| (k.clone(), 1.0)).collect()
    } else {
        scores
            .iter()
            .map(|(k, v)| (k.clone(), (v - min) / range))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::providers::InMemoryVectorProvider;
    use async_trait::async_trait;

    /// Stub FTS provider that returns hardcoded results.
    struct StubFTS {
        results: Vec<SearchResult>,
    }

    #[async_trait]
    impl FTSProvider for StubFTS {
        async fn search(
            &self,
            _query: &str,
            limit: usize,
        ) -> Result<Vec<SearchResult>, SearchError> {
            let mut r = self.results.clone();
            r.truncate(limit);
            if r.is_empty() {
                Err(SearchError::NoResults)
            } else {
                Ok(r)
            }
        }
    }

    #[tokio::test]
    async fn test_hybrid_vector_only() {
        let mut vec_provider = InMemoryVectorProvider::new();
        vec_provider.add_document("a".into(), vec![1.0, 0.0], "alpha".into(), HashMap::new());
        vec_provider.add_document("b".into(), vec![0.0, 1.0], "beta".into(), HashMap::new());

        let engine = HybridSearch::new().with_vector(Box::new(vec_provider));

        let results = engine.search("", Some(&[1.0, 0.0]), 10).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "a");
    }

    #[tokio::test]
    async fn test_hybrid_fts_only() {
        let fts = StubFTS {
            results: vec![
                SearchResult {
                    id: "x".into(),
                    score: 0.9,
                    content: "hello".into(),
                    metadata: HashMap::new(),
                },
                SearchResult {
                    id: "y".into(),
                    score: 0.5,
                    content: "world".into(),
                    metadata: HashMap::new(),
                },
            ],
        };

        let engine = HybridSearch::new().with_fts(Box::new(fts));
        let results = engine.search("hello", None, 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "x");
    }

    #[tokio::test]
    async fn test_hybrid_combined() {
        let mut vec_provider = InMemoryVectorProvider::new();
        vec_provider.add_document("shared".into(), vec![1.0, 0.0], "shared doc".into(), HashMap::new());
        vec_provider.add_document("vec_only".into(), vec![0.8, 0.2], "vector doc".into(), HashMap::new());

        let fts = StubFTS {
            results: vec![
                SearchResult {
                    id: "shared".into(),
                    score: 0.8,
                    content: "shared doc".into(),
                    metadata: HashMap::new(),
                },
                SearchResult {
                    id: "fts_only".into(),
                    score: 0.6,
                    content: "fts doc".into(),
                    metadata: HashMap::new(),
                },
            ],
        };

        let engine = HybridSearch::new()
            .with_vector(Box::new(vec_provider))
            .with_fts(Box::new(fts))
            .with_weights(0.5, 0.5);

        let results = engine.search("test", Some(&[1.0, 0.0]), 10).await.unwrap();
        // "shared" should appear only once and score highest due to both sources
        let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"shared"));
        assert!(ids.contains(&"vec_only"));
        assert!(ids.contains(&"fts_only"));
        // No duplicates
        let unique_count = {
            let mut u = ids.clone();
            u.sort();
            u.dedup();
            u.len()
        };
        assert_eq!(unique_count, ids.len());
    }

    #[tokio::test]
    async fn test_hybrid_no_providers() {
        let engine = HybridSearch::new();
        let result = engine.search("test", None, 10).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_scores() {
        let mut scores = HashMap::new();
        scores.insert("a".into(), 10.0);
        scores.insert("b".into(), 5.0);
        scores.insert("c".into(), 0.0);

        let normed = normalize_scores(&scores);
        assert!((normed["a"] - 1.0).abs() < 1e-9);
        assert!((normed["b"] - 0.5).abs() < 1e-9);
        assert!((normed["c"] - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_normalize_scores_equal() {
        let mut scores = HashMap::new();
        scores.insert("a".into(), 5.0);
        scores.insert("b".into(), 5.0);

        let normed = normalize_scores(&scores);
        assert!((normed["a"] - 1.0).abs() < 1e-9);
        assert!((normed["b"] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_normalize_scores_empty() {
        let scores: HashMap<String, f64> = HashMap::new();
        let normed = normalize_scores(&scores);
        assert!(normed.is_empty());
    }

    #[test]
    fn test_with_weights_normalization() {
        let engine = HybridSearch::new().with_weights(3.0, 7.0);
        assert!((engine.vector_weight - 0.3).abs() < 1e-9);
        assert!((engine.fts_weight - 0.7).abs() < 1e-9);
    }
}
