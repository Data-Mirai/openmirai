//! Markdown note parser -- frontmatter extraction, wiki-link discovery.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// YAML frontmatter metadata for a vault note.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteMetadata {
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub custom_fields: HashMap<String, Value>,
}

/// A fully parsed Markdown note.
#[derive(Debug, Clone)]
pub struct ParsedNote {
    pub metadata: NoteMetadata,
    /// Body content without the YAML frontmatter block.
    pub content: String,
    /// Wiki-links (`[[ref]]`) found in the content (deduplicated, order preserved).
    pub outlinks: Vec<String>,
}

// Pre-compiled regex for [[reference]] or [[reference|alias]] wiki-links.
static WIKI_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+?)(?:\|[^\]]+)?\]\]").unwrap());

/// Extract YAML frontmatter delimited by `---` markers and return the remaining body.
///
/// If the text does not start with `---` or YAML parsing fails, returns default
/// metadata and the full text as the body (REGLA-343).
pub fn parse_frontmatter(text: &str) -> (NoteMetadata, &str) {
    if !text.starts_with("---") {
        return (NoteMetadata::default(), text);
    }

    // Find closing --- (must appear after a newline following the opening ---)
    let after_open = &text[3..];
    let end_idx = match after_open.find("\n---") {
        Some(idx) => idx,
        None => return (NoteMetadata::default(), text),
    };

    // yaml_block sits between the opening --- and closing ---
    let yaml_block = &after_open[1..end_idx]; // skip leading \n
    let rest = &after_open[end_idx + 4..]; // skip \n---
    // Strip leading newlines from body (there can be one or two)
    let body = rest.trim_start_matches('\n');

    match serde_yaml::from_str::<NoteMetadata>(yaml_block) {
        Ok(meta) => (meta, body),
        Err(_) => (NoteMetadata::default(), text),
    }
}

/// Find all `[[link]]` patterns in the content. Returns deduplicated references
/// preserving first-seen order.
pub fn extract_wiki_links(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut links = Vec::new();

    for cap in WIKI_LINK_RE.captures_iter(content) {
        let reference = cap[1].trim().to_string();
        if !reference.is_empty() && seen.insert(reference.clone()) {
            links.push(reference);
        }
    }

    links
}

/// Parse a complete Markdown note: extract frontmatter, body, and wiki-links.
pub fn parse_note(text: &str) -> ParsedNote {
    let (metadata, body) = parse_frontmatter(text);
    let outlinks = extract_wiki_links(body);
    ParsedNote {
        metadata,
        content: body.to_string(),
        outlinks,
    }
}

/// Build a complete Markdown note with YAML frontmatter + body content.
pub fn build_note_markdown(metadata: &NoteMetadata, content: &str) -> String {
    match serde_yaml::to_string(metadata) {
        Ok(yaml) => {
            let yaml = yaml.trim_end();
            format!("---\n{}\n---\n\n{}", yaml, content)
        }
        Err(_) => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_with_yaml() {
        let text = "---\ntitle: My Note\ntags:\n  - rust\n  - test\n---\n\nBody content here.";
        let (meta, body) = parse_frontmatter(text);
        assert_eq!(meta.title.as_deref(), Some("My Note"));
        assert_eq!(meta.tags, vec!["rust", "test"]);
        assert_eq!(body, "Body content here.");
    }

    #[test]
    fn test_parse_frontmatter_no_yaml() {
        let text = "Just a plain note.";
        let (meta, body) = parse_frontmatter(text);
        assert!(meta.title.is_none());
        assert!(meta.tags.is_empty());
        assert_eq!(body, text);
    }

    #[test]
    fn test_parse_frontmatter_invalid_yaml() {
        let text = "---\n: broken: yaml: [[\n---\n\nBody.";
        let (meta, body) = parse_frontmatter(text);
        // Falls back to default metadata + full text
        assert!(meta.title.is_none());
        assert_eq!(body, text);
    }

    #[test]
    fn test_parse_frontmatter_no_closing() {
        let text = "---\ntitle: Open\nNo closing fence.";
        let (meta, body) = parse_frontmatter(text);
        assert!(meta.title.is_none());
        assert_eq!(body, text);
    }

    #[test]
    fn test_extract_wiki_links_basic() {
        let content = "See [[fed-news]] and [[markets|US Markets]] for details.";
        let links = extract_wiki_links(content);
        assert_eq!(links, vec!["fed-news", "markets"]);
    }

    #[test]
    fn test_extract_wiki_links_dedup() {
        let content = "Link [[alpha]] and [[alpha]] again.";
        let links = extract_wiki_links(content);
        assert_eq!(links, vec!["alpha"]);
    }

    #[test]
    fn test_extract_wiki_links_empty() {
        let links = extract_wiki_links("No links here.");
        assert!(links.is_empty());
    }

    #[test]
    fn test_parse_note_full() {
        let text = "---\ntitle: Test\ntags:\n  - demo\n---\n\nSee [[other-note]] for more.";
        let note = parse_note(text);
        assert_eq!(note.metadata.title.as_deref(), Some("Test"));
        assert_eq!(note.metadata.tags, vec!["demo"]);
        assert!(note.content.contains("See [[other-note]]"));
        assert_eq!(note.outlinks, vec!["other-note"]);
    }

    #[test]
    fn test_build_note_markdown() {
        let meta = NoteMetadata {
            title: Some("Hello".to_string()),
            tags: vec!["a".to_string()],
            custom_fields: HashMap::new(),
        };
        let md = build_note_markdown(&meta, "Body text.");
        assert!(md.starts_with("---\n"));
        assert!(md.contains("title: Hello"));
        assert!(md.contains("Body text."));
    }
}
