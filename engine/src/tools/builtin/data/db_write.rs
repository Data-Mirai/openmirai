//! DbWriteTool

use super::*;

// ===========================================================================
// DbWriteTool
// ===========================================================================

data_tool! {
    struct DbWriteTool, factory DbWriteFactory;
    tool_type = "data/db_write",
    name = "DB Write",
    description = "Writes data to relational database with automatic table creation",
    inputs = [
        field("data", FieldType::Object, true, "Row data to write"),
    ],
    outputs = [
        field("table", FieldType::String, true, "Target table name"),
        field("action", FieldType::String, true, "Action performed: inserted or updated"),
        field("id", FieldType::String, false, "Row ID of the written record"),
    ],
    config_fields = [
        field("table", FieldType::String, true, "Target table name"),
        field("mode", FieldType::String, false, "Write mode: 'insert' or 'upsert' (default: insert)"),
    ]
}

#[async_trait]
impl Tool for DbWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/db_write".into(),
            message: "no database resource configured".into(),
        })?;

        let table = config
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("insert");

        // Get data from input, normalizing from string if needed
        let data = match inputs.get("data") {
            Some(Value::String(s)) => {
                // Try to parse JSON string
                serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({"content": s}))
            }
            Some(Value::Object(_)) => inputs.get("data").cloned().unwrap(),
            Some(other) => json!({"content": other.to_string()}),
            None => json!({}),
        };

        // Build SQL based on mode
        let action = if mode == "upsert" {
            "updated"
        } else {
            "inserted"
        };

        // Use the data as params for the execute call
        let params = vec![json!(table), data.clone(), json!(mode)];

        let result =
            db.execute("INSERT", &params)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "data/db_write".into(),
                    message: e.to_string(),
                })?;

        // Extract row_id from result
        let row_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut out = HashMap::new();
        out.insert("table".to_string(), json!(table));
        out.insert("action".to_string(), json!(action));
        out.insert("id".to_string(), json!(row_id));
        Ok(out)
    }
}
