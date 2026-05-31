//! MkdirTool

use super::*;

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
        let raw_path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "filesystem/mkdir".into(),
                message: "missing required input: path".into(),
            }
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
