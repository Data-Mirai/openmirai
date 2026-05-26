//! Search subsystem -- vector, full-text, and hybrid search providers.

pub mod hybrid;
pub mod providers;

pub use hybrid::HybridSearch;
pub use providers::{
    cosine_similarity, FTSProvider, InMemoryVectorProvider, SearchError, SearchResult,
    VectorSearchProvider,
};
