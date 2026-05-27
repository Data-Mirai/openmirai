//! ReadFileTool

use super::*;

// ===========================================================================
// ReadFileTool
// ===========================================================================

fs_tool! {
    struct ReadFileTool, factory ReadFileFactory;
    tool_type = "filesystem/read_file",
    name = "Read File",
    description = "Reads a file and returns its contents with numbered lines. Supports offset/limit for large files.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Absolute or relative path to the file"),
    ],
    outputs = [
        field("content", FieldType::String, true, "File content with numbered lines"),
        field("lines", FieldType::Number, true, "Total number of lines in the file"),
        field("size", FieldType::Number, true, "File size in bytes"),
        field("path", FieldType::String, true, "Resolved absolute path"),
    ],
    config_fields = [
        field("offset", FieldType::Number, false, "Line number to start reading from (0-based)"),
        field("limit", FieldType::Number, false, "Maximum number of lines to read"),
    ]
}

#[async_trait]
impl Tool for ReadFileTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/read_file".into(),
                message: "missing required input: path".into(),
            })?;

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/read_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/read_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let metadata = fs::metadata(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/read_file".into(),
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
            tool_type: "filesystem/read_file".into(),
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

