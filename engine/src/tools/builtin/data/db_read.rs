//! DbReadTool

use super::*;

// ===========================================================================
// DbReadTool
// ===========================================================================

data_tool! {
    struct DbReadTool, factory DbReadFactory;
    tool_type = "data/db_read",
    name = "DB Read",
    description = "Reads data from relational database with filtering and pagination",
    inputs = [
        field("query_params", FieldType::Object, false, "Filter parameters: {column: value} for WHERE clause"),
    ],
    outputs = [
        field("rows", FieldType::Array, false, "Array of matched rows (mode=all)"),
        field("row", FieldType::Object, false, "Single matched row (mode=one)"),
        field("count", FieldType::Number, true, "Number of rows returned"),
    ],
    config_fields = [
        field("query", FieldType::String, true, "SQL query to execute"),
        field("mode", FieldType::String, false, "Read mode: 'one' or 'all' (default: all)"),
    ]
}

#[async_trait]
impl Tool for DbReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/db_read".into(),
            message: "no database resource configured".into(),
        })?;

        let query = config
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("SELECT");

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        // Build params from query_params input
        let params: Vec<Value> = match inputs.get("query_params") {
            Some(Value::Object(map)) => map.values().cloned().collect(),
            _ => vec![],
        };

        let mut out = HashMap::new();

        if mode == "one" {
            let row = db
                .fetch_one(query, &params)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "data/db_read".into(),
                    message: e.to_string(),
                })?;

            let count = if row.is_some() { 1 } else { 0 };
            out.insert("row".to_string(), row.unwrap_or(Value::Null));
            out.insert("count".to_string(), json!(count));
        } else {
            let rows = db
                .fetch_all(query, &params)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "data/db_read".into(),
                    message: e.to_string(),
                })?;

            let count = rows.len();
            out.insert("rows".to_string(), json!(rows));
            out.insert("count".to_string(), json!(count));
        }

        Ok(out)
    }
}

