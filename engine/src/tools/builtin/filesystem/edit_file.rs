//! EditFileTool

use super::*;

// ===========================================================================
// EditFileTool
// ===========================================================================

fs_tool! {
    struct EditFileTool, factory EditFileFactory;
    tool_type = "filesystem/edit_file",
    name = "Edit File",
    description = "Performs exact string replacement in a file. The old_string must match exactly. By default replaces only the first occurrence.",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Path to the file to edit"),
        field("old_string", FieldType::String, true, "Exact string to find"),
        field("new_string", FieldType::String, true, "Replacement string"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Resolved absolute path"),
        field("replacements", FieldType::Number, true, "Number of replacements made"),
        field("diff", FieldType::String, true, "Summary of changes"),
    ],
    config_fields = [
        field("replace_all", FieldType::Boolean, false, "Replace all occurrences instead of just the first"),
    ]
}

#[async_trait]
impl Tool for EditFileTool {
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
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: path".into(),
            })?;
        let old_string = inputs
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: old_string".into(),
            })?;
        let new_string = inputs
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "missing required input: new_string".into(),
            })?;
        let replace_all = config
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("File not found: {raw_path} ({e})"),
        })?;

        if !path.is_file() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!("Not a file: {}", path.display()),
            });
        }

        let content = fs::read_to_string(&path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("Cannot read file: {e}"),
        })?;

        if old_string == new_string {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: "old_string and new_string are identical".into(),
            });
        }

        let count = content.matches(old_string).count();
        if count == 0 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!("old_string not found in {}", path.display()),
            });
        }

        if !replace_all && count > 1 {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/edit_file".into(),
                message: format!(
                    "old_string found {} times in {}. Provide more context to make it unique, or set replace_all=true.",
                    count,
                    path.display()
                ),
            });
        }

        let (new_content, replacements) = if replace_all {
            (content.replace(old_string, new_string), count)
        } else {
            (content.replacen(old_string, new_string, 1), 1)
        };

        fs::write(&path, &new_content).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/edit_file".into(),
            message: format!("Cannot write file: {e}"),
        })?;

        // Build diff preview (truncate long strings)
        let old_preview: String = old_string.chars().take(80).collect::<String>().replace('\n', "\\n");
        let new_preview: String = new_string.chars().take(80).collect::<String>().replace('\n', "\\n");
        let diff = format!(
            "-  {}\n+  {}\n({} replacement(s))",
            old_preview, new_preview, replacements
        );

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path.display().to_string()));
        out.insert("replacements".to_string(), json!(replacements));
        out.insert("diff".to_string(), json!(diff));
        Ok(out)
    }
}

