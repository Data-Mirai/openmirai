//! SOUL.md — Agent personality and identity system.
//!
//! A Soul defines who an agent is, how it behaves, what it can do, and what
//! workflows it can execute. Parsed from SOUL.md files (Markdown with YAML
//! frontmatter).
//!
//! Format:
//! ```markdown
//! ---
//! name: analyst
//! identity: "I am a senior data analyst"
//! personality: "Direct, data-driven, no fluff"
//! capabilities: [analysis, reporting]
//! constraints: ["No predictions without data"]
//! workflows: [analyze.yaml, report.yaml]
//! ---
//!
//! # Additional Context
//! [Free text injected as part of the system prompt]
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Parsed Soul — agent personality and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Soul {
    /// Unique name for this personality.
    pub name: String,
    /// Who the agent is (first person description).
    #[serde(default)]
    pub identity: String,
    /// How the agent behaves (tone, style).
    #[serde(default)]
    pub personality: String,
    /// What the agent can do (capability tags).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// What the agent must NOT do.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Workflow files this agent can execute.
    #[serde(default)]
    pub workflows: Vec<String>,
    /// Optional knowledge references (docs, URLs).
    #[serde(default)]
    pub knowledge_refs: Vec<String>,
    /// Free-text context from the body of SOUL.md (below the frontmatter).
    #[serde(skip_deserializing, default)]
    pub context: String,
}

/// Errors that can occur when loading a Soul.
#[derive(Debug, thiserror::Error)]
pub enum SoulError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SOUL.md must have YAML frontmatter between --- markers")]
    MissingFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Load a Soul from a SOUL.md file.
pub fn load_from_file(path: &Path) -> Result<Soul, SoulError> {
    let content = std::fs::read_to_string(path)?;
    parse(&content)
}

/// Parse a Soul from a SOUL.md string.
pub fn parse(content: &str) -> Result<Soul, SoulError> {
    // Split frontmatter from body.
    let content = content.trim();
    if !content.starts_with("---") {
        return Err(SoulError::MissingFrontmatter);
    }

    let after_first = &content[3..];
    let end = after_first
        .find("---")
        .ok_or(SoulError::MissingFrontmatter)?;

    let yaml_str = &after_first[..end].trim();
    let body = after_first[end + 3..].trim();

    let mut soul: Soul = serde_yaml::from_str(yaml_str)?;
    soul.context = body.to_string();

    Ok(soul)
}

impl Soul {
    /// Generate the system prompt for this Soul.
    ///
    /// Combines identity, personality, capabilities, constraints, and context
    /// into a structured system prompt that shapes the agent's behavior.
    pub fn to_system_prompt(&self) -> String {
        let mut parts = Vec::new();

        if !self.identity.is_empty() {
            parts.push(format!("## Identity\n{}", self.identity));
        }
        if !self.personality.is_empty() {
            parts.push(format!("## Personality\n{}", self.personality));
        }
        if !self.capabilities.is_empty() {
            let caps = self
                .capabilities
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("## Capabilities\n{caps}"));
        }
        if !self.constraints.is_empty() {
            let cons = self
                .constraints
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("## Constraints\n{cons}"));
        }
        if !self.context.is_empty() {
            parts.push(format!("## Context\n{}", self.context));
        }

        parts.join("\n\n")
    }

    /// List of workflow names (filenames without path).
    pub fn workflow_names(&self) -> Vec<&str> {
        self.workflows.iter().map(|w| w.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SOUL: &str = r#"---
name: analyst
identity: "I am a senior data analyst with 10 years of experience"
personality: "Direct, data-driven, no fluff. I speak with confidence."
capabilities:
  - data_analysis
  - reporting
  - visualization
constraints:
  - "Never make predictions without supporting data"
  - "Always cite data sources"
workflows:
  - analyze-dataset.yaml
  - generate-report.yaml
---

# Additional Context

I specialize in e-commerce analytics and have deep knowledge of
customer behavior patterns and retention metrics.
"#;

    #[test]
    fn parse_full_soul() {
        let soul = parse(SAMPLE_SOUL).unwrap();
        assert_eq!(soul.name, "analyst");
        assert!(soul.identity.contains("senior data analyst"));
        assert!(soul.personality.contains("Direct"));
        assert_eq!(soul.capabilities.len(), 3);
        assert_eq!(soul.constraints.len(), 2);
        assert_eq!(soul.workflows.len(), 2);
        assert!(soul.context.contains("e-commerce analytics"));
    }

    #[test]
    fn parse_minimal_soul() {
        let content = "---\nname: minimal\n---\n";
        let soul = parse(content).unwrap();
        assert_eq!(soul.name, "minimal");
        assert!(soul.identity.is_empty());
        assert!(soul.capabilities.is_empty());
    }

    #[test]
    fn missing_frontmatter_fails() {
        let result = parse("No frontmatter here");
        assert!(result.is_err());
    }

    #[test]
    fn system_prompt_generation() {
        let soul = parse(SAMPLE_SOUL).unwrap();
        let prompt = soul.to_system_prompt();
        assert!(prompt.contains("## Identity"));
        assert!(prompt.contains("senior data analyst"));
        assert!(prompt.contains("## Personality"));
        assert!(prompt.contains("## Capabilities"));
        assert!(prompt.contains("- data_analysis"));
        assert!(prompt.contains("## Constraints"));
        assert!(prompt.contains("Never make predictions"));
        assert!(prompt.contains("## Context"));
        assert!(prompt.contains("e-commerce"));
    }

    #[test]
    fn workflow_names() {
        let soul = parse(SAMPLE_SOUL).unwrap();
        let names = soul.workflow_names();
        assert_eq!(names, vec!["analyze-dataset.yaml", "generate-report.yaml"]);
    }

    #[test]
    fn serde_roundtrip() {
        let soul = parse(SAMPLE_SOUL).unwrap();
        let json = serde_json::to_string(&soul).unwrap();
        let back: Soul = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "analyst");
        assert_eq!(back.capabilities.len(), 3);
    }
}
