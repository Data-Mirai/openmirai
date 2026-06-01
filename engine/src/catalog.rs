//! Curated catalog of popular Ollama models (PRD-014).
//!
//! Ollama exposes no public API for its full remote library, so we ship a
//! maintained list here and merge it with the locally-installed models
//! (`GET /api/tags`, see [`crate::preflight::installed_models`]). This keeps the
//! model picker offline-first and predictable.

#[derive(Debug, Clone, serde::Serialize)]
pub struct CuratedModel {
    pub name: &'static str,
    pub family: &'static str,
    pub size_gb: f32,
    pub description: &'static str,
    pub recommended: bool,
}

/// Popular Ollama models a developer can pick as their default and pull.
/// Sizes are approximate download sizes — a guide, not a contract.
pub const CATALOG: &[CuratedModel] = &[
    CuratedModel {
        name: "llama3.2:1b",
        family: "llama",
        size_gb: 1.3,
        description: "Tiny & fast — ideal first run",
        recommended: true,
    },
    CuratedModel {
        name: "llama3.2:3b",
        family: "llama",
        size_gb: 2.0,
        description: "Small, capable general model",
        recommended: true,
    },
    CuratedModel {
        name: "qwen3:1.7b",
        family: "qwen",
        size_gb: 1.4,
        description: "Tiny Qwen3 — fast local runs",
        recommended: true,
    },
    CuratedModel {
        name: "qwen3:8b",
        family: "qwen",
        size_gb: 5.2,
        description: "Strong general model (current default)",
        recommended: true,
    },
    CuratedModel {
        name: "gemma3:1b",
        family: "gemma",
        size_gb: 0.9,
        description: "Tiny Gemma 3",
        recommended: false,
    },
    CuratedModel {
        name: "gemma3:4b",
        family: "gemma",
        size_gb: 3.3,
        description: "Small multimodal Gemma 3",
        recommended: false,
    },
    CuratedModel {
        name: "phi4",
        family: "phi",
        size_gb: 9.1,
        description: "Microsoft Phi-4 — reasoning",
        recommended: false,
    },
    CuratedModel {
        name: "mistral",
        family: "mistral",
        size_gb: 4.1,
        description: "Mistral 7B Instruct",
        recommended: false,
    },
    CuratedModel {
        name: "deepseek-r1:7b",
        family: "deepseek",
        size_gb: 4.7,
        description: "Reasoning model (R1 distill)",
        recommended: false,
    },
    CuratedModel {
        name: "nomic-embed-text",
        family: "nomic",
        size_gb: 0.3,
        description: "Text embeddings",
        recommended: false,
    },
];

/// Look up a curated entry by exact name.
pub fn find(name: &str) -> Option<&'static CuratedModel> {
    CATALOG.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_recommended_and_default() {
        assert!(CATALOG.iter().any(|m| m.recommended));
        assert!(find("qwen3:8b").is_some());
        assert!(find("llama3.2:1b").is_some());
    }
}
