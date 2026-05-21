//! Vault subsystem -- Obsidian-inspired linked Markdown notes backed by the filesystem.

pub mod parser;
pub mod service;

pub use parser::{extract_wiki_links, parse_frontmatter, parse_note, NoteMetadata, ParsedNote};
pub use service::{IndexedNote, NoteIndex, VaultService};
