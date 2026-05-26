use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{FieldType, ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder (same pattern as logic.rs)
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
        required,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.into())
        },
        default: None,
    }
}

// ---------------------------------------------------------------------------
// Macro: same pattern as logic_tool! but with category = "filesystem"
// ---------------------------------------------------------------------------

macro_rules! fs_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        category = $cat:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: $cat.into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ===========================================================================
// ReadFileTool
// ===========================================================================

fs_tool! {
    struct ReadFileTool, factory ReadFileFactory;
    tool_type = "filesystem/read_file",
    name = "Read File",
    description = "Reads a file and returns its contents with numbered lines. Supports offset/limit for large files.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Absolute or relative path to the file"),
    ],
    outputs = [
        field("content", FieldType::String, true, "File content with numbered lines"),
        field("lines", FieldType::Number, true, "Total number of lines in the file"),
        field("size", FieldType::Number, true, "File size in bytes"),
        field("path", FieldType::String, true, "Resolved absolute path"),
    ],
    config_fields = [
        field("offset", FieldType::Number, false, "Line number to start reading from (0-based)"),
        field("limit", FieldType::Number, false, "Maximum number of lines to read"),
    ]
}

#[async_trait]
impl Tool for ReadFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/read_file".into(),
                message: "missing required input: path".into(),
            })?;

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/read_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/read_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let metadata = fs::metadata(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/read_file".into(),
            message: format!("Cannot stat file: {e}"),
        })?;
        let size = metadata.len();

        let offset = config
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = config
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000) as usize;

        let content = fs::read_to_string(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/read_file".into(),
            message: format!("Cannot read file: {e}"),
        })?;

        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        let end = std::cmp::min(offset + limit, total_lines);
        let selected = if offset < total_lines {
            &all_lines[offset..end]
        } else {
            &[]
        };

        let numbered: Vec<String> = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", offset + i + 1, line))
            .collect();

        let mut out = HashMap::new();
        out.insert("content".to_string(), json!(numbered.join("\n")));
        out.insert("lines".to_string(), json!(total_lines));
        out.insert("size".to_string(), json!(size));
        out.insert("path".to_string(), json!(path.display().to_string()));
        Ok(out)
    }
}

// ===========================================================================
// WriteFileTool
// ===========================================================================

fs_tool! {
    struct WriteFileTool, factory WriteFileFactory;
    tool_type = "filesystem/write_file",
    name = "Write File",
    description = "Creates a new file or overwrites an existing one with the provided content.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Absolute or relative path for the file"),
        field("content", FieldType::String, true, "Content to write"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Resolved absolute path"),
        field("bytes_written", FieldType::Number, true, "Number of bytes written"),
        field("created", FieldType::Boolean, true, "True if the file was newly created"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for WriteFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/write_file".into(),
                message: "missing required input: path".into(),
            })?;
        let content = inputs
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/write_file".into(),
                message: "missing required input: content".into(),
            })?;

        let path = PathBuf::from(raw_path);
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&path)
        };

        let created = !abs_path.exists();

        if let Some(parent) = abs_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "filesystem/write_file".into(),
                    message: format!("Cannot create parent directories: {e}"),
                })?;
            }
        }

        let bytes = content.as_bytes();
        fs::write(&abs_path, bytes).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/write_file".into(),
            message: format!("Cannot write file: {e}"),
        })?;

        let mut out = HashMap::new();
        out.insert(
            "path".to_string(),
            json!(abs_path.display().to_string()),
        );
        out.insert("bytes_written".to_string(), json!(bytes.len()));
        out.insert("created".to_string(), json!(created));
        Ok(out)
    }
}

// ===========================================================================
// ListDirTool
// ===========================================================================

fs_tool! {
    struct ListDirTool, factory ListDirFactory;
    tool_type = "filesystem/list_dir",
    name = "List Directory",
    description = "Lists the contents of a directory with file type, size, and modification time.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Directory path to list"),
    ],
    outputs = [
        field("entries", FieldType::Array, true, "List of {name, type, size, modified} entries"),
        field("count", FieldType::Number, true, "Number of entries"),
    ],
    config_fields = [
        field("show_hidden", FieldType::Boolean, false, "Include hidden files (starting with '.')"),
    ]
}

#[async_trait]
impl Tool for ListDirTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/list_dir".into(),
                message: "missing required input: path".into(),
            })?;
        let show_hidden = config
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/list_dir".into(),
            message: format!("Directory not found: {raw_path} ({e})"),
        })?;

        if !path.is_dir() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/list_dir".into(),
                message: format!("Not a directory: {}", path.display()),
            });
        }

        let mut entries_raw: Vec<(String, Value)> = Vec::new();
        let read_dir = fs::read_dir(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/list_dir".into(),
            message: format!("Cannot read directory: {e}"),
        })?;

        for entry_result in read_dir {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().to_string();

            if !show_hidden && name.starts_with('.') {
                continue;
            }

            let meta = entry.metadata();
            let (ftype, size, modified) = match meta {
                Ok(m) => {
                    let ftype = if m.is_dir() {
                        "directory"
                    } else {
                        "file"
                    };
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0);
                    (ftype, m.len(), modified)
                }
                Err(_) => ("unknown", 0, 0.0),
            };

            entries_raw.push((
                name.clone(),
                json!({
                    "name": name,
                    "type": ftype,
                    "size": size,
                    "modified": modified,
                }),
            ));
        }

        // Sort by name
        entries_raw.sort_by(|a, b| a.0.cmp(&b.0));
        let entries: Vec<Value> = entries_raw.into_iter().map(|(_, v)| v).collect();
        let count = entries.len();

        let mut out = HashMap::new();
        out.insert("entries".to_string(), json!(entries));
        out.insert("count".to_string(), json!(count));
        Ok(out)
    }
}

// ===========================================================================
// GlobFilesTool
// ===========================================================================

fs_tool! {
    struct GlobFilesTool, factory GlobFilesFactory;
    tool_type = "filesystem/glob_files",
    name = "Glob (Find Files)",
    description = "Finds files matching a glob pattern. Returns paths sorted by modification time (newest first).",
    category = "filesystem",
    inputs = [
        field("pattern", FieldType::String, true, "Glob pattern (e.g. '**/*.py')"),
    ],
    outputs = [
        field("files", FieldType::Array, true, "List of matching file paths"),
        field("count", FieldType::Number, true, "Number of matches"),
    ],
    config_fields = [
        field("path", FieldType::String, false, "Base directory to search in"),
        field("max_results", FieldType::Number, false, "Maximum number of results"),
    ]
}

#[async_trait]
impl Tool for GlobFilesTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let pattern = inputs
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/glob_files".into(),
                message: "missing required input: pattern".into(),
            })?;

        let base = config
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let max_results = config
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(200) as usize;

        let base_path = fs::canonicalize(base).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/glob_files".into(),
            message: format!("Base path not found: {base} ({e})"),
        })?;

        let full_pattern = format!("{}/{}", base_path.display(), pattern);

        let paths = glob::glob(&full_pattern).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/glob_files".into(),
            message: format!("Invalid glob pattern: {e}"),
        })?;

        // Collect files with mtime
        let mut files_with_mtime: Vec<(PathBuf, f64)> = Vec::new();
        for entry in paths {
            let p = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !p.is_file() {
                continue;
            }
            let mtime = p
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            files_with_mtime.push((p, mtime));
        }

        // Sort by mtime descending (newest first)
        files_with_mtime.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        files_with_mtime.truncate(max_results);

        let files: Vec<String> = files_with_mtime
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect();
        let count = files.len();

        let mut out = HashMap::new();
        out.insert("files".to_string(), json!(files));
        out.insert("count".to_string(), json!(count));
        Ok(out)
    }
}

// ===========================================================================
// GrepFilesTool
// ===========================================================================

fs_tool! {
    struct GrepFilesTool, factory GrepFilesFactory;
    tool_type = "filesystem/grep_files",
    name = "Grep (Search Content)",
    description = "Searches file contents using regex. Returns matching lines with file paths and line numbers.",
    category = "filesystem",
    inputs = [
        field("pattern", FieldType::String, true, "Regex pattern to search for"),
    ],
    outputs = [
        field("matches", FieldType::Array, true, "List of {file, line, content} matches"),
        field("files", FieldType::Array, true, "Unique files with matches"),
        field("count", FieldType::Number, true, "Total number of matches"),
    ],
    config_fields = [
        field("path", FieldType::String, false, "Directory or file to search in"),
        field("glob", FieldType::String, false, "Glob filter for files (e.g. '*.py')"),
        field("max_results", FieldType::Number, false, "Maximum matches to return"),
        field("case_insensitive", FieldType::Boolean, false, "Case-insensitive search"),
        field("context_lines", FieldType::Number, false, "Lines of context around each match"),
    ]
}

const BINARY_EXT: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg",
    ".woff", ".woff2", ".ttf", ".eot",
    ".zip", ".gz", ".tar", ".bz2", ".7z", ".rar",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx",
    ".pyc", ".pyo", ".so", ".dylib", ".dll", ".exe",
    ".db", ".sqlite", ".sqlite3",
    ".mp3", ".mp4", ".wav", ".avi", ".mov",
];

const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "__pycache__", ".venv", "venv",
    "dist", "build", ".next", ".cache", ".tox", "egg-info",
];

fn has_binary_ext(p: &Path) -> bool {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    match ext {
        Some(e) => BINARY_EXT.contains(&e.as_str()),
        None => false,
    }
}

fn should_skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

/// Collect all searchable files under `base`, skipping binary extensions and
/// directories in SKIP_DIRS. If `glob_filter` is non-empty, only files whose
/// name matches the glob pattern are included.
fn collect_files(base: &Path, glob_filter: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if base.is_file() {
        result.push(base.to_path_buf());
        return result;
    }
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if ft.is_dir() {
                if !should_skip_dir(&name) {
                    stack.push(entry.path());
                }
            } else if ft.is_file() {
                let p = entry.path();
                if has_binary_ext(&p) {
                    continue;
                }
                if !glob_filter.is_empty() {
                    // Simple fnmatch-style: use glob pattern matching on filename
                    let pat = glob::Pattern::new(glob_filter);
                    if let Ok(pat) = pat {
                        if !pat.matches(&name) {
                            continue;
                        }
                    }
                }
                result.push(p);
            }
        }
    }
    result
}

#[async_trait]
impl Tool for GrepFilesTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let pattern_str = inputs
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/grep_files".into(),
                message: "missing required input: pattern".into(),
            })?;

        let base = config
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let glob_filter = config
            .get("glob")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let max_results = config
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;
        let case_insensitive = config
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let context_lines = config
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern_str)
        } else {
            pattern_str.to_string()
        };

        let re = Regex::new(&regex_pattern).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/grep_files".into(),
            message: format!("Invalid regex pattern: {e}"),
        })?;

        let base_path = fs::canonicalize(base).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/grep_files".into(),
            message: format!("Path not found: {base} ({e})"),
        })?;

        let target_files = collect_files(&base_path, glob_filter);

        let mut matches: Vec<Value> = Vec::new();
        let mut files_seen: Vec<String> = Vec::new();

        'outer: for fpath in &target_files {
            let content = match fs::read_to_string(fpath) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let lines: Vec<&str> = content.lines().collect();

            for (i, line) in lines.iter().enumerate() {
                if matches.len() >= max_results {
                    break 'outer;
                }
                if re.is_match(line) {
                    let fpath_str = fpath.display().to_string();
                    if !files_seen.contains(&fpath_str) {
                        files_seen.push(fpath_str.clone());
                    }
                    let mut entry = json!({
                        "file": fpath_str,
                        "line": i + 1,
                        "content": line.trim_end(),
                    });
                    if context_lines > 0 {
                        let start = i.saturating_sub(context_lines);
                        let end = std::cmp::min(lines.len(), i + context_lines + 1);
                        let ctx: Vec<&str> = lines[start..end]
                            .iter()
                            .map(|l| l.trim_end())
                            .collect();
                        entry["context"] = json!(ctx);
                    }
                    matches.push(entry);
                }
            }
        }

        files_seen.sort();
        let count = matches.len();

        let mut out = HashMap::new();
        out.insert("matches".to_string(), json!(matches));
        out.insert("files".to_string(), json!(files_seen));
        out.insert("count".to_string(), json!(count));
        Ok(out)
    }
}

// ===========================================================================
// EditFileTool
// ===========================================================================

fs_tool! {
    struct EditFileTool, factory EditFileFactory;
    tool_type = "filesystem/edit_file",
    name = "Edit File",
    description = "Performs exact string replacement in a file. The old_string must match exactly. By default replaces only the first occurrence.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Path to the file to edit"),
        field("old_string", FieldType::String, true, "Exact string to find"),
        field("new_string", FieldType::String, true, "Replacement string"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Resolved absolute path"),
        field("replacements", FieldType::Number, true, "Number of replacements made"),
        field("diff", FieldType::String, true, "Summary of changes"),
    ],
    config_fields = [
        field("replace_all", FieldType::Boolean, false, "Replace all occurrences instead of just the first"),
    ]
}

#[async_trait]
impl Tool for EditFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: path".into(),
            })?;
        let old_string = inputs
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: old_string".into(),
            })?;
        let new_string = inputs
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: new_string".into(),
            })?;
        let replace_all = config
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let content = fs::read_to_string(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("Cannot read file: {e}"),
        })?;

        if old_string == new_string {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "old_string and new_string are identical".into(),
            });
        }

        let count = content.matches(old_string).count();
        if count == 0 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!("old_string not found in {}", path.display()),
            });
        }

        if !replace_all && count > 1 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!(
                    "old_string found {} times in {}. Provide more context to make it unique, or set replace_all=true.",
                    count,
                    path.display()
                ),
            });
        }

        let (new_content, replacements) = if replace_all {
            (content.replace(old_string, new_string), count)
        } else {
            (content.replacen(old_string, new_string, 1), 1)
        };

        fs::write(&path, &new_content).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("Cannot write file: {e}"),
        })?;

        // Build diff preview (truncate long strings)
        let old_preview: String = old_string.chars().take(80).collect::<String>().replace('\n', "\\n");
        let new_preview: String = new_string.chars().take(80).collect::<String>().replace('\n', "\\n");
        let diff = format!(
            "-  {}\n+  {}\n({} replacement(s))",
            old_preview, new_preview, replacements
        );

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path.display().to_string()));
        out.insert("replacements".to_string(), json!(replacements));
        out.insert("diff".to_string(), json!(diff));
        Ok(out)
    }
}

// ===========================================================================
// CopyTool
// ===========================================================================

fs_tool! {
    struct CopyTool, factory CopyFactory;
    tool_type = "filesystem/copy",
    name = "Copy File",
    description = "Copies a file from source to destination",
    category = "filesystem",
    inputs = [
        field("source", FieldType::String, true, "Source file path"),
        field("destination", FieldType::String, true, "Destination file path"),
    ],
    outputs = [
        field("source", FieldType::String, true, "Resolved source path"),
        field("destination", FieldType::String, true, "Resolved destination path"),
        field("bytes_copied", FieldType::Number, true, "Number of bytes copied"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for CopyTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let source = inputs
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/copy".into(),
                message: "missing required input: source".into(),
            })?;
        let destination = inputs
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/copy".into(),
                message: "missing required input: destination".into(),
            })?;

        let src_path = fs::canonicalize(source).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/copy".into(),
            message: format!("Source not found: {source} ({e})"),
        })?;

        let dest_path = PathBuf::from(destination);
        let abs_dest = if dest_path.is_absolute() {
            dest_path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&dest_path)
        };

        // Create parent dirs if needed
        if let Some(parent) = abs_dest.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "filesystem/copy".into(),
                    message: format!("Cannot create parent directories: {e}"),
                })?;
            }
        }

        let bytes_copied = fs::copy(&src_path, &abs_dest).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/copy".into(),
            message: format!("Copy failed: {e}"),
        })?;

        let mut out = HashMap::new();
        out.insert("source".to_string(), json!(src_path.display().to_string()));
        out.insert("destination".to_string(), json!(abs_dest.display().to_string()));
        out.insert("bytes_copied".to_string(), json!(bytes_copied));
        Ok(out)
    }
}

// ===========================================================================
// MoveTool
// ===========================================================================

fs_tool! {
    struct MoveTool, factory MoveFactory;
    tool_type = "filesystem/move",
    name = "Move/Rename",
    description = "Moves or renames a file or directory",
    category = "filesystem",
    inputs = [
        field("source", FieldType::String, true, "Source path"),
        field("destination", FieldType::String, true, "Destination path"),
    ],
    outputs = [
        field("source", FieldType::String, true, "Original path"),
        field("destination", FieldType::String, true, "New path"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for MoveTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let source = inputs
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/move".into(),
                message: "missing required input: source".into(),
            })?;
        let destination = inputs
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/move".into(),
                message: "missing required input: destination".into(),
            })?;

        let src_path = fs::canonicalize(source).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/move".into(),
            message: format!("Source not found: {source} ({e})"),
        })?;

        let dest_path = PathBuf::from(destination);
        let abs_dest = if dest_path.is_absolute() {
            dest_path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&dest_path)
        };

        if let Some(parent) = abs_dest.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "filesystem/move".into(),
                    message: format!("Cannot create parent directories: {e}"),
                })?;
            }
        }

        fs::rename(&src_path, &abs_dest).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/move".into(),
            message: format!("Move failed: {e}"),
        })?;

        let mut out = HashMap::new();
        out.insert("source".to_string(), json!(src_path.display().to_string()));
        out.insert("destination".to_string(), json!(abs_dest.display().to_string()));
        Ok(out)
    }
}

// ===========================================================================
// DeleteTool
// ===========================================================================

fs_tool! {
    struct DeleteTool, factory DeleteFactory;
    tool_type = "filesystem/delete",
    name = "Delete",
    description = "Deletes a file or directory (recursive for directories)",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Path to delete"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Path that was deleted"),
        field("deleted", FieldType::Boolean, true, "Whether deletion succeeded"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for DeleteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/delete".into(),
                message: "missing required input: path".into(),
            })?;

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/delete".into(),
            message: format!("Path not found: {raw_path} ({e})"),
        })?;

        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| ToolError::ExecutionFailed {
                tool_type: "filesystem/delete".into(),
                message: format!("Cannot delete directory: {e}"),
            })?;
        } else {
            fs::remove_file(&path).map_err(|e| ToolError::ExecutionFailed {
                tool_type: "filesystem/delete".into(),
                message: format!("Cannot delete file: {e}"),
            })?;
        }

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path.display().to_string()));
        out.insert("deleted".to_string(), json!(true));
        Ok(out)
    }
}

// ===========================================================================
// MkdirTool
// ===========================================================================

fs_tool! {
    struct MkdirTool, factory MkdirFactory;
    tool_type = "filesystem/mkdir",
    name = "Create Directory",
    description = "Creates a directory and all parent directories as needed",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Directory path to create"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Created directory path"),
        field("created", FieldType::Boolean, true, "Whether directory was created"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for MkdirTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/mkdir".into(),
                message: "missing required input: path".into(),
            })?;

        let path = PathBuf::from(raw_path);
        let abs_path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&path)
        };

        fs::create_dir_all(&abs_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/mkdir".into(),
            message: format!("Cannot create directory: {e}"),
        })?;

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(abs_path.display().to_string()));
        out.insert("created".to_string(), json!(true));
        Ok(out)
    }
}

// ===========================================================================
// TreeTool
// ===========================================================================

fs_tool! {
    struct TreeTool, factory TreeFactory;
    tool_type = "filesystem/tree",
    name = "Directory Tree",
    description = "Displays a recursive directory listing with indentation",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Root directory path"),
    ],
    outputs = [
        field("tree", FieldType::String, true, "Formatted directory tree"),
        field("files", FieldType::Number, true, "Total file count"),
        field("dirs", FieldType::Number, true, "Total directory count"),
    ],
    config_fields = [
        field("max_depth", FieldType::Number, false, "Maximum depth to traverse (default 5)"),
        field("show_hidden", FieldType::Boolean, false, "Include hidden files"),
    ]
}

fn build_tree(
    dir: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    show_hidden: bool,
    lines: &mut Vec<String>,
    file_count: &mut usize,
    dir_count: &mut usize,
) {
    if depth > max_depth {
        return;
    }

    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .collect(),
        Err(_) => return,
    };
    entries.sort_by(|a, b| {
        a.file_name()
            .to_string_lossy()
            .cmp(&b.file_name().to_string_lossy())
    });

    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };

        lines.push(format!("{}{}{}{}", prefix, connector, name, suffix));

        if is_dir {
            *dir_count += 1;
            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            build_tree(
                &entry.path(),
                &child_prefix,
                depth + 1,
                max_depth,
                show_hidden,
                lines,
                file_count,
                dir_count,
            );
        } else {
            *file_count += 1;
        }
    }
}

#[async_trait]
impl Tool for TreeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/tree".into(),
                message: "missing required input: path".into(),
            })?;
        let max_depth = config
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        let show_hidden = config
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/tree".into(),
            message: format!("Path not found: {raw_path} ({e})"),
        })?;

        if !path.is_dir() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/tree".into(),
                message: format!("Not a directory: {}", path.display()),
            });
        }

        let mut lines = vec![format!("{}/", path.display())];
        let mut file_count: usize = 0;
        let mut dir_count: usize = 0;
        build_tree(
            &path, "", 0, max_depth, show_hidden, &mut lines, &mut file_count, &mut dir_count,
        );

        let mut out = HashMap::new();
        out.insert("tree".to_string(), json!(lines.join("\n")));
        out.insert("files".to_string(), json!(file_count));
        out.insert("dirs".to_string(), json!(dir_count));
        Ok(out)
    }
}

// ===========================================================================
// FileInfoTool
// ===========================================================================

fs_tool! {
    struct FileInfoTool, factory FileInfoFactory;
    tool_type = "filesystem/file_info",
    name = "File Info",
    description = "Returns metadata about a file: size, modified time, type, permissions",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Path to get info for"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Resolved absolute path"),
        field("size", FieldType::Number, true, "Size in bytes"),
        field("modified", FieldType::Number, true, "Last modified timestamp (seconds since epoch)"),
        field("is_file", FieldType::Boolean, true, "Whether path is a file"),
        field("is_dir", FieldType::Boolean, true, "Whether path is a directory"),
        field("permissions", FieldType::String, true, "Permission mode (octal on unix)"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for FileInfoTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/file_info".into(),
                message: "missing required input: path".into(),
            })?;

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/file_info".into(),
            message: format!("Path not found: {raw_path} ({e})"),
        })?;

        let metadata = fs::metadata(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/file_info".into(),
            message: format!("Cannot stat path: {e}"),
        })?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            format!("{:o}", metadata.permissions().mode())
        };
        #[cfg(not(unix))]
        let permissions = if metadata.permissions().readonly() {
            "readonly".to_string()
        } else {
            "read-write".to_string()
        };

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path.display().to_string()));
        out.insert("size".to_string(), json!(metadata.len()));
        out.insert("modified".to_string(), json!(modified));
        out.insert("is_file".to_string(), json!(metadata.is_file()));
        out.insert("is_dir".to_string(), json!(metadata.is_dir()));
        out.insert("permissions".to_string(), json!(permissions));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all filesystem tools into the given registry.
pub fn register_filesystem_tools(registry: &mut ToolRegistry) {
    // Canonical names (match Python: filesystem/*)
    registry.register("filesystem/read_file", Box::new(ReadFileFactory::new()));
    registry.register("filesystem/write_file", Box::new(WriteFileFactory::new()));
    registry.register("filesystem/list_dir", Box::new(ListDirFactory::new()));
    registry.register("filesystem/glob_files", Box::new(GlobFilesFactory::new()));
    registry.register("filesystem/grep_files", Box::new(GrepFilesFactory::new()));
    registry.register("filesystem/edit_file", Box::new(EditFileFactory::new()));
    registry.register("filesystem/copy", Box::new(CopyFactory::new()));
    registry.register("filesystem/move", Box::new(MoveFactory::new()));
    registry.register("filesystem/delete", Box::new(DeleteFactory::new()));
    registry.register("filesystem/mkdir", Box::new(MkdirFactory::new()));
    registry.register("filesystem/tree", Box::new(TreeFactory::new()));
    registry.register("filesystem/file_info", Box::new(FileInfoFactory::new()));

    // Legacy aliases (fs/* → filesystem/*) — remove in v0.3.0
    registry.register_alias("fs/read_file", "filesystem/read_file");
    registry.register_alias("fs/write_file", "filesystem/write_file");
    registry.register_alias("fs/list_dir", "filesystem/list_dir");
    registry.register_alias("fs/glob_files", "filesystem/glob_files");
    registry.register_alias("fs/grep_files", "filesystem/grep_files");
    registry.register_alias("fs/edit_file", "filesystem/edit_file");
    registry.register_alias("fs/copy", "filesystem/copy");
    registry.register_alias("fs/move", "filesystem/move");
    registry.register_alias("fs/delete", "filesystem/delete");
    registry.register_alias("fs/mkdir", "filesystem/mkdir");
    registry.register_alias("fs/tree", "filesystem/tree");
    registry.register_alias("fs/file_info", "filesystem/file_info");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::InMemoryContext;
    use std::io::Write;
    use tempfile::TempDir;

    fn ctx() -> InMemoryContext {
        InMemoryContext::new("test-run")
    }

    // -- ReadFileTool -------------------------------------------------------

    #[tokio::test]
    async fn read_file_basic() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("hello.txt");
        {
            let mut f = fs::File::create(&file_path).unwrap();
            writeln!(f, "line one").unwrap();
            writeln!(f, "line two").unwrap();
            writeln!(f, "line three").unwrap();
        }

        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        let config = HashMap::new();
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["lines"], json!(3));
        let content = result["content"].as_str().unwrap();
        assert!(content.contains("line one"));
        assert!(content.contains("line three"));
    }

    #[tokio::test]
    async fn read_file_with_offset_limit() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("nums.txt");
        {
            let mut f = fs::File::create(&file_path).unwrap();
            for i in 1..=10 {
                writeln!(f, "line {}", i).unwrap();
            }
        }

        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        let mut config = HashMap::new();
        config.insert("offset".to_string(), json!(2));
        config.insert("limit".to_string(), json!(3));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        let content = result["content"].as_str().unwrap();
        assert!(content.contains("line 3"));
        assert!(content.contains("line 5"));
        assert!(!content.contains("line 1\t") || !content.starts_with("     1\t"));
    }

    #[tokio::test]
    async fn read_file_not_found() {
        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("/nonexistent/file.txt"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    // -- WriteFileTool ------------------------------------------------------

    #[tokio::test]
    async fn write_file_creates_new() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new.txt");

        let tool = WriteFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("content".to_string(), json!("hello world"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert_eq!(result["bytes_written"], json!(11));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn write_file_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("a/b/c/deep.txt");

        let tool = WriteFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("content".to_string(), json!("deep"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert!(file_path.exists());
    }

    // -- ListDirTool --------------------------------------------------------

    #[tokio::test]
    async fn list_dir_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        let tool = ListDirTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(dir.path().display().to_string()),
        );
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        // Hidden excluded by default
        assert_eq!(result["count"], json!(3));
        let entries = result["entries"].as_array().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"subdir"));
        assert!(!names.contains(&".hidden"));
    }

    #[tokio::test]
    async fn list_dir_show_hidden() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        let tool = ListDirTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(dir.path().display().to_string()),
        );
        let mut config = HashMap::new();
        config.insert("show_hidden".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- GlobFilesTool ------------------------------------------------------

    #[tokio::test]
    async fn glob_files_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("b.rs"), "fn test() {}").unwrap();
        fs::write(dir.path().join("c.txt"), "hello").unwrap();

        let tool = GlobFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("*.rs"));
        let mut config = HashMap::new();
        config.insert(
            "path".to_string(),
            json!(dir.path().display().to_string()),
        );
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- EditFileTool -------------------------------------------------------

    #[tokio::test]
    async fn edit_file_single_replacement() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("edit_me.txt");
        fs::write(&file_path, "hello world").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("old_string".to_string(), json!("hello"));
        inputs.insert("new_string".to_string(), json!("goodbye"));
        let result = tool
            .execute(inputs, &HashMap::new(), &ctx())
            .await
            .unwrap();

        assert_eq!(result["replacements"], json!(1));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "goodbye world");
    }

    #[tokio::test]
    async fn edit_file_replace_all() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("multi.txt");
        fs::write(&file_path, "aaa bbb aaa").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("old_string".to_string(), json!("aaa"));
        inputs.insert("new_string".to_string(), json!("ccc"));
        let mut config = HashMap::new();
        config.insert("replace_all".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["replacements"], json!(2));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "ccc bbb ccc");
    }

    #[tokio::test]
    async fn edit_file_ambiguous_without_replace_all() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("dup.txt");
        fs::write(&file_path, "aaa bbb aaa").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("old_string".to_string(), json!("aaa"));
        inputs.insert("new_string".to_string(), json!("ccc"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn edit_file_not_found_string() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("nope.txt");
        fs::write(&file_path, "hello world").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "path".to_string(),
            json!(file_path.display().to_string()),
        );
        inputs.insert("old_string".to_string(), json!("xyz"));
        inputs.insert("new_string".to_string(), json!("abc"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    // -- GrepFilesTool ------------------------------------------------------

    #[tokio::test]
    async fn grep_files_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world\nfoo bar\nhello again").unwrap();
        fs::write(dir.path().join("b.txt"), "nothing here").unwrap();

        let tool = GrepFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("hello"));
        let mut config = HashMap::new();
        config.insert(
            "path".to_string(),
            json!(dir.path().display().to_string()),
        );
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn grep_files_case_insensitive() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "Hello World\nhello world").unwrap();

        let tool = GrepFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("HELLO"));
        let mut config = HashMap::new();
        config.insert(
            "path".to_string(),
            json!(dir.path().display().to_string()),
        );
        config.insert("case_insensitive".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- CopyTool -----------------------------------------------------------

    #[tokio::test]
    async fn copy_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        fs::write(&src, "copy me").unwrap();
        let dest = dir.path().join("dest.txt");

        let tool = CopyTool;
        let mut inputs = HashMap::new();
        inputs.insert("source".to_string(), json!(src.display().to_string()));
        inputs.insert("destination".to_string(), json!(dest.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["bytes_copied"], json!(7));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "copy me");
    }

    // -- MoveTool -----------------------------------------------------------

    #[tokio::test]
    async fn move_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("old.txt");
        fs::write(&src, "move me").unwrap();
        let dest = dir.path().join("new.txt");

        let tool = MoveTool;
        let mut inputs = HashMap::new();
        inputs.insert("source".to_string(), json!(src.display().to_string()));
        inputs.insert("destination".to_string(), json!(dest.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "move me");
        assert!(result["destination"].as_str().is_some());
    }

    // -- DeleteTool ---------------------------------------------------------

    #[tokio::test]
    async fn delete_file_basic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("delete_me.txt");
        fs::write(&file, "bye").unwrap();

        let tool = DeleteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["deleted"], json!(true));
        assert!(!file.exists());
    }

    // -- MkdirTool ----------------------------------------------------------

    #[tokio::test]
    async fn mkdir_creates_nested() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a/b/c");

        let tool = MkdirTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(nested.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert!(nested.is_dir());
    }

    // -- TreeTool -----------------------------------------------------------

    #[tokio::test]
    async fn tree_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), "b").unwrap();

        let tool = TreeTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(dir.path().display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert!(result["tree"].as_str().unwrap().contains("a.txt"));
        assert!(result["tree"].as_str().unwrap().contains("sub/"));
        assert_eq!(result["files"], json!(2));
        assert_eq!(result["dirs"], json!(1));
    }

    // -- FileInfoTool -------------------------------------------------------

    #[tokio::test]
    async fn file_info_basic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("info.txt");
        fs::write(&file, "hello").unwrap();

        let tool = FileInfoTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["size"], json!(5));
        assert_eq!(result["is_file"], json!(true));
        assert_eq!(result["is_dir"], json!(false));
    }

    // -- Registration -------------------------------------------------------

    #[test]
    fn register_filesystem_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_filesystem_tools(&mut reg);
        // Canonical names
        assert!(reg.get("filesystem/read_file").is_some());
        assert!(reg.get("filesystem/write_file").is_some());
        assert!(reg.get("filesystem/list_dir").is_some());
        assert!(reg.get("filesystem/glob_files").is_some());
        assert!(reg.get("filesystem/grep_files").is_some());
        assert!(reg.get("filesystem/edit_file").is_some());
        assert!(reg.get("filesystem/copy").is_some());
        assert!(reg.get("filesystem/move").is_some());
        assert!(reg.get("filesystem/delete").is_some());
        assert!(reg.get("filesystem/mkdir").is_some());
        assert!(reg.get("filesystem/tree").is_some());
        assert!(reg.get("filesystem/file_info").is_some());
        assert_eq!(reg.list_tools().len(), 12);
    }

    #[test]
    fn legacy_fs_aliases_resolve() {
        let mut reg = ToolRegistry::new();
        register_filesystem_tools(&mut reg);
        // Legacy aliases (fs/* → filesystem/*)
        assert!(reg.get("fs/read_file").is_some());
        assert!(reg.get("fs/write_file").is_some());
        assert!(reg.get("fs/list_dir").is_some());
        assert!(reg.get("fs/copy").is_some());
        assert!(reg.get("fs/delete").is_some());
    }
}
