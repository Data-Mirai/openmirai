//! StorageWriteTool

use super::*;

// ===========================================================================
// StorageWriteTool
// ===========================================================================

data_tool! {
    struct StorageWriteTool, factory StorageWriteFactory;
    tool_type = "data/storage_write",
    name = "Storage Write",
    description = "Writes file to S3-compatible storage",
    inputs = [
        field("path", FieldType::String, true, "Storage path/key to write to"),
        field("content", FieldType::String, true, "Content to write"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Path that was written"),
        field("bytes_written", FieldType::Number, true, "Number of bytes written"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for StorageWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let storage = context.storage().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/storage_write".into(),
            message: "no storage resource configured".into(),
        })?;

        let path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: "input 'path' is required".into(),
            })?;

        let content = inputs
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: "input 'content' is required".into(),
            })?;

        let data = content.as_bytes();
        let bytes_written = data.len();

        storage
            .put(path, data)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));
        out.insert("bytes_written".to_string(), json!(bytes_written));
        Ok(out)
    }
}

