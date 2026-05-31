//! WriteFileTool

use super::*;

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
        let raw_path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "filesystem/write_file".into(),
                message: "missing required input: path".into(),
            }
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
        out.insert("path".to_string(), json!(abs_path.display().to_string()));
        out.insert("bytes_written".to_string(), json!(bytes.len()));
        out.insert("created".to_string(), json!(created));
        Ok(out)
    }
}
