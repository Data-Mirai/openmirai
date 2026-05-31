//! EntityQueryTool

use super::*;

// ===========================================================================
// EntityQueryTool
// ===========================================================================

data_tool! {
    struct EntityQueryTool, factory EntityQueryFactory;
    tool_type = "data/entity_query",
    name = "Entity Query",
    description = "Queries entities from the database with field extraction",
    inputs = [
        field("entity_type", FieldType::String, true, "Entity type to query"),
        field("filters", FieldType::Object, false, "Filter conditions as {field: value}"),
    ],
    outputs = [
        field("entities", FieldType::Array, true, "Matched entities"),
        field("count", FieldType::Number, true, "Number of entities returned"),
    ],
    config_fields = [
        field("limit", FieldType::Number, false, "Maximum entities to return"),
        field("order_by", FieldType::String, false, "Field to order by"),
    ]
}

#[async_trait]
impl Tool for EntityQueryTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/entity_query".into(),
            message: "no database resource configured".into(),
        })?;

        let entity_type = inputs
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let limit = config.get("limit").and_then(|v| v.as_u64()).unwrap_or(100);

        let query = format!("SELECT * FROM entities WHERE type = ? LIMIT {}", limit);
        let params = vec![json!(entity_type)];

        let rows = db
            .fetch_all(&query, &params)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/entity_query".into(),
                message: e.to_string(),
            })?;

        let count = rows.len();
        let mut out = HashMap::new();
        out.insert("entities".to_string(), json!(rows));
        out.insert("count".to_string(), json!(count));
        Ok(out)
    }
}
