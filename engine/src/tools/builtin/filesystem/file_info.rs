//! FileInfoTool

use super::*;

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
        let raw_path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "filesystem/file_info".into(),
                message: "missing required input: path".into(),
            }
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
