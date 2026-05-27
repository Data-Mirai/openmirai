//! EntityUpsertTool

use super::*;

// ===========================================================================
// EntityUpsertTool
// ===========================================================================

data_tool! {
    struct EntityUpsertTool, factory EntityUpsertFactory;
    tool_type = "data/entity_upsert",
    name = "Entity Upsert",
    description = "Creates or updates an entity in the database",
    inputs = [
        field("entity_type", FieldType::String, true, "Entity type"),
        field("data", FieldType::Object, true, "Entity field data"),
        field("id", FieldType::String, false, "Entity ID (if updating)"),
    ],
    outputs = [
        field("id", FieldType::String, true, "Entity ID"),
        field("action", FieldType::String, true, "Action performed: created or updated"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for EntityUpsertTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/entity_upsert".into(),
            message: "no database resource configured".into(),
        })?;

        let entity_type = inputs
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let data = inputs
            .get("data")
            .cloned()
            .unwrap_or(json!({}));
        let existing_id = inputs
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let action = if existing_id.is_some() { "updated" } else { "created" };

        let id = existing_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let params = vec![json!(id), json!(entity_type), data];
        db.execute("UPSERT", &params)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/entity_upsert".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("id".to_string(), json!(id));
        out.insert("action".to_string(), json!(action));
        Ok(out)
    }
}

