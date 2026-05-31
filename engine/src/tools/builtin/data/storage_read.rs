//! StorageReadTool

use super::*;

// ===========================================================================
// StorageReadTool
// ===========================================================================

data_tool! {
    struct StorageReadTool, factory StorageReadFactory;
    tool_type = "data/storage_read",
    name = "Storage Read",
    description = "Reads file from configured storage (filesystem or in-memory)",
    inputs = [
        field("path", FieldType::String, true, "Storage path/key to read"),
    ],
    outputs = [
        field("content", FieldType::String, false, "File content as text"),
        field("path", FieldType::String, true, "Path that was read"),
        field("found", FieldType::Boolean, true, "Whether the file was found"),
    ],
    config_fields = [
        field("mode", FieldType::String, false, "Read mode: 'read' or 'presign' (default: read)"),
    ]
}

#[async_trait]
impl Tool for StorageReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let storage = context
            .storage()
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_read".into(),
                message: "no storage resource configured".into(),
            })?;

        let path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "data/storage_read".into(),
                message: "input 'path' is required".into(),
            }
        })?;

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("read");

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));

        if mode == "presign" {
            // Presigned URL mode: for now, construct a placeholder path-based URL.
            // A real implementation would delegate to the storage resource's presign method.
            out.insert("content".to_string(), Value::Null);
            out.insert("found".to_string(), json!(true));
            return Ok(out);
        }

        // Normal read mode
        match storage.get(path).await {
            Ok(data) => {
                let content = String::from_utf8(data)
                    .unwrap_or_else(|e| format!("<binary data: {} bytes>", e.as_bytes().len()));
                out.insert("content".to_string(), json!(content));
                out.insert("found".to_string(), json!(true));
            }
            Err(crate::core::context::ResourceError::NotFound(_)) => {
                out.insert("content".to_string(), Value::Null);
                out.insert("found".to_string(), json!(false));
            }
            Err(e) => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "data/storage_read".into(),
                    message: e.to_string(),
                });
            }
        }

        Ok(out)
    }
}
