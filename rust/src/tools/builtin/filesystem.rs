use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder (same pattern as logic.rs)
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: &str, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type: field_type.into(),
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
            fn create(&self) -> Box<dyn Tool> {
                Box::new($tool)
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
    tool_type = "fs/read_file",
    name = "Read File",
    description = "Reads a file and returns its contents with numbered lines. Supports offset/limit for large files.",
    category = "filesystem",
    inputs = [
        field("path", "string", true, "Absolute or relative path to the file"),
    ],
    outputs = [
        field("content", "string", true, "File content with numbered lines"),
        field("lines", "number", true, "Total number of lines in the file"),
        field("size", "number", true, "File size in bytes"),
        field("path", "string", true, "Resolved absolute path"),
    ],
    config_fields = [
        field("offset", "number", false, "Line number to start reading from (0-based)"),
        field("limit", "number", false, "Maximum number of lines to read"),
    ]
}

#[async_trait]
impl Tool for ReadFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/read_file".into(),
                message: "missing required input: path".into(),
            })?;

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/read_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/read_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let metadata = fs::metadata(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/read_file".into(),
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
            tool_type: "fs/read_file".into(),
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
    tool_type = "fs/write_file",
    name = "Write File",
    description = "Creates a new file or overwrites an existing one with the provided content.",
    category = "filesystem",
    inputs = [
        field("path", "string", true, "Absolute or relative path for the file"),
        field("content", "string", true, "Content to write"),
    ],
    outputs = [
        field("path", "string", true, "Resolved absolute path"),
        field("bytes_written", "number", true, "Number of bytes written"),
        field("created", "boolean", true, "True if the file was newly created"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for WriteFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/write_file".into(),
                message: "missing required input: path".into(),
            })?;
        let content = inputs
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/write_file".into(),
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
                    tool_type: "fs/write_file".into(),
                    message: format!("Cannot create parent directories: {e}"),
                })?;
            }
        }

        let bytes = content.as_bytes();
        fs::write(&abs_path, bytes).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/write_file".into(),
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
    tool_type = "fs/list_dir",
    name = "List Directory",
    description = "Lists the contents of a directory with file type, size, and modification time.",
    category = "filesystem",
    inputs = [
        field("path", "string", true, "Directory path to list"),
    ],
    outputs = [
        field("entries", "array", true, "List of {name, type, size, modified} entries"),
        field("count", "number", true, "Number of entries"),
    ],
    config_fields = [
        field("show_hidden", "boolean", false, "Include hidden files (starting with '.')"),
    ]
}

#[async_trait]
impl Tool for ListDirTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/list_dir".into(),
                message: "missing required input: path".into(),
            })?;
        let show_hidden = config
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/list_dir".into(),
            message: format!("Directory not found: {raw_path} ({e})"),
        })?;

        if !path.is_dir() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/list_dir".into(),
                message: format!("Not a directory: {}", path.display()),
            });
        }

        let mut entries_raw: Vec<(String, Value)> = Vec::new();
        let read_dir = fs::read_dir(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/list_dir".into(),
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
    tool_type = "fs/glob_files",
    name = "Glob (Find Files)",
    description = "Finds files matching a glob pattern. Returns paths sorted by modification time (newest first).",
    category = "filesystem",
    inputs = [
        field("pattern", "string", true, "Glob pattern (e.g. '**/*.py')"),
    ],
    outputs = [
        field("files", "array", true, "List of matching file paths"),
        field("count", "number", true, "Number of matches"),
    ],
    config_fields = [
        field("path", "string", false, "Base directory to search in"),
        field("max_results", "number", false, "Maximum number of results"),
    ]
}

#[async_trait]
impl Tool for GlobFilesTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let pattern = inputs
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/glob_files".into(),
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
            tool_type: "fs/glob_files".into(),
            message: format!("Base path not found: {base} ({e})"),
        })?;

        let full_pattern = format!("{}/{}", base_path.display(), pattern);

        let paths = glob::glob(&full_pattern).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/glob_files".into(),
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
    tool_type = "fs/grep_files",
    name = "Grep (Search Content)",
    description = "Searches file contents using regex. Returns matching lines with file paths and line numbers.",
    category = "filesystem",
    inputs = [
        field("pattern", "string", true, "Regex pattern to search for"),
    ],
    outputs = [
        field("matches", "array", true, "List of {file, line, content} matches"),
        field("files", "array", true, "Unique files with matches"),
        field("count", "number", true, "Total number of matches"),
    ],
    config_fields = [
        field("path", "string", false, "Directory or file to search in"),
        field("glob", "string", false, "Glob filter for files (e.g. '*.py')"),
        field("max_results", "number", false, "Maximum matches to return"),
        field("case_insensitive", "boolean", false, "Case-insensitive search"),
        field("context_lines", "number", false, "Lines of context around each match"),
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
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let pattern_str = inputs
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/grep_files".into(),
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
            tool_type: "fs/grep_files".into(),
            message: format!("Invalid regex pattern: {e}"),
        })?;

        let base_path = fs::canonicalize(base).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/grep_files".into(),
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
    tool_type = "fs/edit_file",
    name = "Edit File",
    description = "Performs exact string replacement in a file. The old_string must match exactly. By default replaces only the first occurrence.",
    category = "filesystem",
    inputs = [
        field("path", "string", true, "Path to the file to edit"),
        field("old_string", "string", true, "Exact string to find"),
        field("new_string", "string", true, "Replacement string"),
    ],
    outputs = [
        field("path", "string", true, "Resolved absolute path"),
        field("replacements", "number", true, "Number of replacements made"),
        field("diff", "string", true, "Summary of changes"),
    ],
    config_fields = [
        field("replace_all", "boolean", false, "Replace all occurrences instead of just the first"),
    ]
}

#[async_trait]
impl Tool for EditFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: "missing required input: path".into(),
            })?;
        let old_string = inputs
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: "missing required input: old_string".into(),
            })?;
        let new_string = inputs
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: "missing required input: new_string".into(),
            })?;
        let replace_all = config
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/edit_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let content = fs::read_to_string(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "fs/edit_file".into(),
            message: format!("Cannot read file: {e}"),
        })?;

        if old_string == new_string {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: "old_string and new_string are identical".into(),
            });
        }

        let count = content.matches(old_string).count();
        if count == 0 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
                message: format!("old_string not found in {}", path.display()),
            });
        }

        if !replace_all && count > 1 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "fs/edit_file".into(),
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
            tool_type: "fs/edit_file".into(),
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

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all filesystem tools into the given registry.
pub fn register_filesystem_tools(registry: &mut ToolRegistry) {
    registry.register("fs/read_file", Box::new(ReadFileFactory::new()));
    registry.register("fs/write_file", Box::new(WriteFileFactory::new()));
    registry.register("fs/list_dir", Box::new(ListDirFactory::new()));
    registry.register("fs/glob_files", Box::new(GlobFilesFactory::new()));
    registry.register("fs/grep_files", Box::new(GrepFilesFactory::new()));
    registry.register("fs/edit_file", Box::new(EditFileFactory::new()));
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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await;
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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
            .execute(inputs, HashMap::new(), &ctx())
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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await;
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
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await;
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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

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
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- Registration -------------------------------------------------------

    #[test]
    fn register_filesystem_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_filesystem_tools(&mut reg);
        assert!(reg.get("fs/read_file").is_some());
        assert!(reg.get("fs/write_file").is_some());
        assert!(reg.get("fs/list_dir").is_some());
        assert!(reg.get("fs/glob_files").is_some());
        assert!(reg.get("fs/grep_files").is_some());
        assert!(reg.get("fs/edit_file").is_some());
        assert_eq!(reg.list_tools().len(), 6);
    }
}
