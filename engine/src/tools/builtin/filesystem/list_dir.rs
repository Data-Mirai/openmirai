//! ListDirTool

use super::*;

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
        let raw_path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "filesystem/list_dir".into(),
                message: "missing required input: path".into(),
            }
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
                    let ftype = if m.is_dir() { "directory" } else { "file" };
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
