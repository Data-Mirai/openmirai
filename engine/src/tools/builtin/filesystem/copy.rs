//! CopyTool

use super::*;

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

        let bytes_copied =
            fs::copy(&src_path, &abs_dest).map_err(|e| ToolError::ExecutionFailed {
                tool_type: "filesystem/copy".into(),
                message: format!("Copy failed: {e}"),
            })?;

        let mut out = HashMap::new();
        out.insert("source".to_string(), json!(src_path.display().to_string()));
        out.insert(
            "destination".to_string(),
            json!(abs_dest.display().to_string()),
        );
        out.insert("bytes_copied".to_string(), json!(bytes_copied));
        Ok(out)
    }
}
