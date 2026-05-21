//! VaultService -- filesystem-backed linked Markdown notes with in-memory index.

use crate::vault::parser::{build_note_markdown, extract_wiki_links, parse_note, NoteMetadata, ParsedNote};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// An indexed entry holding note metadata (no body content).
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    pub outlinks: Vec<String>,
    /// Seconds since UNIX epoch of last modification.
    pub modified_at: f64,
}

/// In-memory index over all vault notes with forward and backward link maps.
#[derive(Debug, Default)]
pub struct NoteIndex {
    /// path -> indexed note
    pub notes: HashMap<String, IndexedNote>,
    /// target path -> list of source paths that link to it
    pub backlinks: HashMap<String, Vec<String>>,
}

/// Obsidian-inspired vault service operating on a local directory of `.md` files.
pub struct VaultService {
    root_path: PathBuf,
    index: Arc<RwLock<NoteIndex>>,
}

impl VaultService {
    /// Create a new vault rooted at `root_path`, scanning and indexing all `.md` files.
    pub fn new(root_path: &Path) -> std::io::Result<Self> {
        if !root_path.exists() {
            fs::create_dir_all(root_path)?;
        }
        let service = Self {
            root_path: root_path.to_path_buf(),
            index: Arc::new(RwLock::new(NoteIndex::default())),
        };
        service.rebuild_index();
        Ok(service)
    }

    /// Walk the vault directory, re-parse every `.md` file and rebuild the index
    /// from scratch (backlink map included).
    pub fn rebuild_index(&self) {
        let mut new_index = NoteIndex::default();
        self.walk_and_index(&self.root_path, &mut new_index);

        // Build backlinks from outlinks
        let paths_and_outlinks: Vec<(String, Vec<String>)> = new_index
            .notes
            .iter()
            .map(|(p, n)| (p.clone(), n.outlinks.clone()))
            .collect();

        for (source, outlinks) in &paths_and_outlinks {
            for target in outlinks {
                new_index
                    .backlinks
                    .entry(target.clone())
                    .or_default()
                    .push(source.clone());
            }
        }

        let mut idx = self.index.write().unwrap();
        *idx = new_index;
    }

    /// Recursively walk `dir`, parsing `.md` files into `index`.
    fn walk_and_index(&self, dir: &Path, index: &mut NoteIndex) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.walk_and_index(&path, index);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(text) = fs::read_to_string(&path) {
                    let note = parse_note(&text);
                    let rel = self.relative_path(&path);
                    let modified_at = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|t| {
                            t.duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                        })
                        .unwrap_or(0.0);

                    let title = note
                        .metadata
                        .title
                        .clone()
                        .unwrap_or_else(|| title_from_content(&note.content));

                    index.notes.insert(
                        rel.clone(),
                        IndexedNote {
                            path: rel,
                            title,
                            tags: note.metadata.tags.clone(),
                            outlinks: note.outlinks.clone(),
                            modified_at,
                        },
                    );
                }
            }
        }
    }

    /// Read and parse a single note from disk.
    pub fn read_note(&self, path: &str) -> std::io::Result<ParsedNote> {
        let full = self.root_path.join(path);
        let text = fs::read_to_string(&full)?;
        Ok(parse_note(&text))
    }

    /// Write a note to disk with YAML frontmatter and update the index.
    pub fn write_note(
        &self,
        path: &str,
        content: &str,
        metadata: NoteMetadata,
    ) -> std::io::Result<()> {
        let full = self.root_path.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }

        let md = build_note_markdown(&metadata, content);
        fs::write(&full, &md)?;

        // Update index
        let outlinks = extract_wiki_links(content);
        let modified_at = fs::metadata(&full)
            .and_then(|m| m.modified())
            .map(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
            })
            .unwrap_or(0.0);

        let title = metadata
            .title
            .clone()
            .unwrap_or_else(|| title_from_content(content));

        let indexed = IndexedNote {
            path: path.to_string(),
            title,
            tags: metadata.tags.clone(),
            outlinks: outlinks.clone(),
            modified_at,
        };

        let mut idx = self.index.write().unwrap();

        // Collect old outlinks to remove from backlinks map
        let old_outlinks: Vec<String> = idx
            .notes
            .get(path)
            .map(|n| n.outlinks.clone())
            .unwrap_or_default();

        for old_target in &old_outlinks {
            if let Some(sources) = idx.backlinks.get_mut(old_target) {
                sources.retain(|s| s != path);
            }
        }

        // Insert new backlinks
        for target in &outlinks {
            idx.backlinks
                .entry(target.clone())
                .or_default()
                .push(path.to_string());
        }

        idx.notes.insert(path.to_string(), indexed);
        Ok(())
    }

    /// Case-insensitive content search across all notes. Returns up to `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<IndexedNote> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        let idx = self.index.read().unwrap();
        for (path, note) in &idx.notes {
            // Check title
            if note.title.to_lowercase().contains(&query_lower) {
                results.push(note.clone());
                continue;
            }
            // Check file content
            let full = self.root_path.join(path);
            if let Ok(text) = fs::read_to_string(&full) {
                if text.to_lowercase().contains(&query_lower) {
                    results.push(note.clone());
                }
            }
        }

        // Sort by modified_at descending so most recent matches come first
        results.sort_by(|a, b| b.modified_at.partial_cmp(&a.modified_at).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    /// Return notes that have ALL of the provided tags.
    pub fn search_by_tags(&self, tags: &[String]) -> Vec<IndexedNote> {
        let idx = self.index.read().unwrap();
        idx.notes
            .values()
            .filter(|note| tags.iter().all(|t| note.tags.contains(t)))
            .cloned()
            .collect()
    }

    /// Return list of source paths that link TO the given note path.
    pub fn get_backlinks(&self, path: &str) -> Vec<String> {
        let idx = self.index.read().unwrap();
        idx.backlinks.get(path).cloned().unwrap_or_default()
    }

    /// Return the most recently modified notes, up to `limit`.
    pub fn list_recent(&self, limit: usize) -> Vec<IndexedNote> {
        let idx = self.index.read().unwrap();
        let mut notes: Vec<IndexedNote> = idx.notes.values().cloned().collect();
        notes.sort_by(|a, b| b.modified_at.partial_cmp(&a.modified_at).unwrap_or(std::cmp::Ordering::Equal));
        notes.truncate(limit);
        notes
    }

    /// Delete a note from disk and remove it from the index.
    pub fn delete_note(&self, path: &str) -> std::io::Result<()> {
        let full = self.root_path.join(path);
        if full.exists() {
            fs::remove_file(&full)?;
        }

        let mut idx = self.index.write().unwrap();

        // Collect outlinks to remove from backlinks map
        let old_outlinks: Vec<String> = idx
            .notes
            .get(path)
            .map(|n| n.outlinks.clone())
            .unwrap_or_default();

        for target in &old_outlinks {
            if let Some(sources) = idx.backlinks.get_mut(target) {
                sources.retain(|s| s != path);
            }
        }

        // Remove this note from any backlink target lists
        idx.backlinks.remove(path);

        idx.notes.remove(path);
        Ok(())
    }

    /// Compute relative path string from the vault root.
    fn relative_path(&self, full_path: &Path) -> String {
        full_path
            .strip_prefix(&self.root_path)
            .unwrap_or(full_path)
            .to_string_lossy()
            .to_string()
    }
}

/// Extract a title from the first H1 heading or first non-empty line.
fn title_from_content(body: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().to_string();
        }
        if !trimmed.is_empty() {
            let title: String = trimmed.chars().take(100).collect();
            return title;
        }
    }
    "Untitled".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_vault() -> (TempDir, VaultService) {
        let dir = TempDir::new().unwrap();
        let svc = VaultService::new(dir.path()).unwrap();
        (dir, svc)
    }

    #[test]
    fn test_write_and_read_note() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Test Note".to_string()),
            tags: vec!["rust".to_string()],
            ..Default::default()
        };
        svc.write_note("test.md", "Hello [[world]]", meta).unwrap();

        let note = svc.read_note("test.md").unwrap();
        assert_eq!(note.metadata.title.as_deref(), Some("Test Note"));
        assert!(note.content.contains("Hello [[world]]"));
        assert_eq!(note.outlinks, vec!["world"]);
    }

    #[test]
    fn test_search_content() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Alpha".to_string()),
            ..Default::default()
        };
        svc.write_note("alpha.md", "Unique keyword zebra", meta).unwrap();

        let meta2 = NoteMetadata {
            title: Some("Beta".to_string()),
            ..Default::default()
        };
        svc.write_note("beta.md", "Nothing special", meta2).unwrap();

        let results = svc.search("zebra", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Alpha");
    }

    #[test]
    fn test_search_by_tags() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Tagged".to_string()),
            tags: vec!["finance".to_string(), "macro".to_string()],
            ..Default::default()
        };
        svc.write_note("tagged.md", "Content", meta).unwrap();

        let results = svc.search_by_tags(&["finance".to_string()]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Tagged");

        let results = svc.search_by_tags(&["nonexistent".to_string()]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_backlinks() {
        let (_dir, svc) = setup_vault();
        let m1 = NoteMetadata {
            title: Some("A".to_string()),
            ..Default::default()
        };
        svc.write_note("a.md", "Links to [[b]]", m1).unwrap();

        let m2 = NoteMetadata {
            title: Some("B".to_string()),
            ..Default::default()
        };
        svc.write_note("b.md", "No links", m2).unwrap();

        let backlinks = svc.get_backlinks("b");
        assert_eq!(backlinks, vec!["a.md"]);
    }

    #[test]
    fn test_list_recent() {
        let (_dir, svc) = setup_vault();
        for i in 0..5 {
            let m = NoteMetadata {
                title: Some(format!("Note {}", i)),
                ..Default::default()
            };
            svc.write_note(&format!("note_{}.md", i), "Body", m).unwrap();
        }
        let recent = svc.list_recent(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_delete_note() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Gone".to_string()),
            ..Default::default()
        };
        svc.write_note("gone.md", "Bye", meta).unwrap();

        svc.delete_note("gone.md").unwrap();
        assert!(svc.read_note("gone.md").is_err());

        let idx = svc.index.read().unwrap();
        assert!(!idx.notes.contains_key("gone.md"));
    }

    #[test]
    fn test_rebuild_index() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Indexed".to_string()),
            tags: vec!["idx".to_string()],
            ..Default::default()
        };
        svc.write_note("indexed.md", "Content", meta).unwrap();

        // Clear index manually, then rebuild
        {
            let mut idx = svc.index.write().unwrap();
            idx.notes.clear();
            idx.backlinks.clear();
        }

        svc.rebuild_index();

        let idx = svc.index.read().unwrap();
        assert!(idx.notes.contains_key("indexed.md"));
    }

    #[test]
    fn test_subdirectory_notes() {
        let (_dir, svc) = setup_vault();
        let meta = NoteMetadata {
            title: Some("Nested".to_string()),
            ..Default::default()
        };
        svc.write_note("sub/dir/note.md", "Deep note", meta).unwrap();

        let note = svc.read_note("sub/dir/note.md").unwrap();
        assert_eq!(note.metadata.title.as_deref(), Some("Nested"));
    }
}
