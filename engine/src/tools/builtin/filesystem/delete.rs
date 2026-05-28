//! DeleteTool

use super::*;

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

