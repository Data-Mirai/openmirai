//! VaultReadTool

use super::*;

// ===========================================================================
// VaultReadTool
// ===========================================================================

data_tool! {
    struct VaultReadTool, factory VaultReadFactory;
    tool_type = "data/vault_read",
    name = "Vault Read",
    description = "Reads notes from the Knowledge Vault. Placeholder that returns empty results.",
    inputs = [
        field("query", FieldType::String, false, "Search query for vault notes"),
        field("path", FieldType::String, false, "Specific vault path to read"),
    ],
    outputs = [
        field("notes", FieldType::Array, true, "Matched vault notes"),
        field("count", FieldType::Number, true, "Number of notes returned"),
    ],
    config_fields = [
        field("folder", FieldType::String, false, "Vault folder to search in"),
        field("limit", FieldType::Number, false, "Maximum notes to return"),
    ]
}

#[async_trait]
impl Tool for VaultReadTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // Placeholder: vault requires a filesystem backend.
        // In a real implementation this would query the vault service.
        let mut out = HashMap::new();
        out.insert("notes".to_string(), json!([]));
        out.insert("count".to_string(), json!(0));
        Ok(out)
    }
}
