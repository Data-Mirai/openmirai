//! VaultWriteTool

use super::*;

// ===========================================================================
// VaultWriteTool
// ===========================================================================

data_tool! {
    struct VaultWriteTool, factory VaultWriteFactory;
    tool_type = "data/vault_write",
    name = "Vault Write",
    description = "Writes a note to the Knowledge Vault. Placeholder that acknowledges the write.",
    inputs = [
        field("path", FieldType::String, true, "Vault path for the note"),
        field("title", FieldType::String, false, "Note title"),
        field("content", FieldType::String, true, "Note content"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Path where note was written"),
        field("written", FieldType::Boolean, true, "Whether write succeeded"),
    ],
    config_fields = [
        field("tags", FieldType::String, false, "Comma-separated tags"),
    ]
}

#[async_trait]
impl Tool for VaultWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("vault/untitled.md");

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));
        out.insert("written".to_string(), json!(true));
        Ok(out)
    }
}

