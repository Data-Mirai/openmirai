//! GrepFilesTool

use super::*;

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
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg", ".woff", ".woff2", ".ttf", ".eot",
    ".zip", ".gz", ".tar", ".bz2", ".7z", ".rar", ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".pyc",
    ".pyo", ".so", ".dylib", ".dll", ".exe", ".db", ".sqlite", ".sqlite3", ".mp3", ".mp4", ".wav",
    ".avi", ".mov",
];

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".cache",
    ".tox",
    "egg-info",
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

        let base = config.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let glob_filter = config.get("glob").and_then(|v| v.as_str()).unwrap_or("");
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
                        let ctx: Vec<&str> =
                            lines[start..end].iter().map(|l| l.trim_end()).collect();
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
