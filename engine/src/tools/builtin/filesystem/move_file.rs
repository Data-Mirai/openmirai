//! MoveTool

use super::*;

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
        out.insert(
            "destination".to_string(),
            json!(abs_dest.display().to_string()),
        );
        Ok(out)
    }
}
