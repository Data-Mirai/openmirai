//! GlobFilesTool

use super::*;

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

        let base = config.get("path").and_then(|v| v.as_str()).unwrap_or(".");
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
