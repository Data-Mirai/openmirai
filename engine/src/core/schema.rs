use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("missing table name in config")]
    MissingTableName,

    #[error("invalid column definition: {0}")]
    InvalidColumn(String),
}

// ---------------------------------------------------------------------------
// ColumnType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    Text,
    Integer,
    Float,
    Boolean,
    Json,
    Timestamp,
}

impl ColumnType {
    /// Parse from string, supporting aliases (e.g. "string" -> Text, "bool" -> Boolean).
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "text" | "string" => Self::Text,
            "integer" | "int" => Self::Integer,
            "float" | "number" => Self::Float,
            "boolean" | "bool" => Self::Boolean,
            "json" | "object" => Self::Json,
            "timestamp" | "datetime" => Self::Timestamp,
            _ => Self::Text, // default fallback
        }
    }
}

// ---------------------------------------------------------------------------
// ColumnDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub col_type: ColumnType,
    #[serde(default = "default_true")]
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// TableSchema
// ---------------------------------------------------------------------------

/// System columns that are always added to every table.
const SYSTEM_COLUMNS: &[&str] = &["id", "created_at", "session_id", "node_id"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub table: String,
    pub columns: Vec<ColumnDef>,
}

impl TableSchema {
    /// Build TableSchema from a db_write node config dict.
    ///
    /// Returns `Ok(None)` if no table name is defined.
    pub fn from_config(
        config: &HashMap<String, serde_json::Value>,
    ) -> Result<Option<Self>, SchemaError> {
        let table = config
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if table.is_empty() {
            return Ok(None);
        }

        let columns = match config.get("schema") {
            Some(serde_json::Value::Array(arr)) => {
                let mut cols = Vec::new();
                for item in arr {
                    if let serde_json::Value::Object(map) = item {
                        let name = map
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let col_type_str = map
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("text");

                        let nullable = map
                            .get("nullable")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        let default = map
                            .get("default")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        cols.push(ColumnDef {
                            name,
                            col_type: ColumnType::from_str_loose(col_type_str),
                            nullable,
                            default,
                        });
                    }
                }
                cols
            }
            _ => Vec::new(),
        };

        Ok(Some(Self {
            table: table.to_string(),
            columns,
        }))
    }

    /// Convert to the format expected by ensure_table(), excluding system columns.
    pub fn to_columns_list(&self) -> Vec<HashMap<String, serde_json::Value>> {
        self.columns
            .iter()
            .filter(|col| !SYSTEM_COLUMNS.contains(&col.name.as_str()))
            .map(|col| {
                let mut m = HashMap::new();
                m.insert(
                    "name".to_string(),
                    serde_json::Value::String(col.name.clone()),
                );
                m.insert(
                    "type".to_string(),
                    serde_json::Value::String(format!("{:?}", col.col_type).to_lowercase()),
                );
                m.insert(
                    "nullable".to_string(),
                    serde_json::Value::Bool(col.nullable),
                );
                m
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

/// Map an abstract column type to a concrete SQL type string for the given dialect.
///
/// Supported dialects: "sqlite", "postgres".
pub fn sql_type(col_type: ColumnType, dialect: &str) -> &'static str {
    match (col_type, dialect) {
        (ColumnType::Text, "sqlite") => "TEXT",
        (ColumnType::Text, _) => "TEXT",

        (ColumnType::Integer, "sqlite") => "INTEGER",
        (ColumnType::Integer, _) => "INTEGER",

        (ColumnType::Float, "sqlite") => "REAL",
        (ColumnType::Float, _) => "DOUBLE PRECISION",

        (ColumnType::Boolean, "sqlite") => "INTEGER",
        (ColumnType::Boolean, _) => "BOOLEAN",

        (ColumnType::Json, "sqlite") => "TEXT",
        (ColumnType::Json, _) => "JSONB",

        (ColumnType::Timestamp, "sqlite") => "TEXT",
        (ColumnType::Timestamp, _) => "TIMESTAMPTZ",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_type_from_str_loose() {
        assert_eq!(ColumnType::from_str_loose("text"), ColumnType::Text);
        assert_eq!(ColumnType::from_str_loose("string"), ColumnType::Text);
        assert_eq!(ColumnType::from_str_loose("integer"), ColumnType::Integer);
        assert_eq!(ColumnType::from_str_loose("int"), ColumnType::Integer);
        assert_eq!(ColumnType::from_str_loose("float"), ColumnType::Float);
        assert_eq!(ColumnType::from_str_loose("number"), ColumnType::Float);
        assert_eq!(ColumnType::from_str_loose("boolean"), ColumnType::Boolean);
        assert_eq!(ColumnType::from_str_loose("bool"), ColumnType::Boolean);
        assert_eq!(ColumnType::from_str_loose("json"), ColumnType::Json);
        assert_eq!(ColumnType::from_str_loose("object"), ColumnType::Json);
        assert_eq!(ColumnType::from_str_loose("timestamp"), ColumnType::Timestamp);
        assert_eq!(ColumnType::from_str_loose("datetime"), ColumnType::Timestamp);
        assert_eq!(ColumnType::from_str_loose("unknown"), ColumnType::Text); // fallback
    }

    #[test]
    fn sql_type_sqlite() {
        assert_eq!(sql_type(ColumnType::Text, "sqlite"), "TEXT");
        assert_eq!(sql_type(ColumnType::Integer, "sqlite"), "INTEGER");
        assert_eq!(sql_type(ColumnType::Float, "sqlite"), "REAL");
        assert_eq!(sql_type(ColumnType::Boolean, "sqlite"), "INTEGER");
        assert_eq!(sql_type(ColumnType::Json, "sqlite"), "TEXT");
        assert_eq!(sql_type(ColumnType::Timestamp, "sqlite"), "TEXT");
    }

    #[test]
    fn sql_type_postgres() {
        assert_eq!(sql_type(ColumnType::Text, "postgres"), "TEXT");
        assert_eq!(sql_type(ColumnType::Integer, "postgres"), "INTEGER");
        assert_eq!(sql_type(ColumnType::Float, "postgres"), "DOUBLE PRECISION");
        assert_eq!(sql_type(ColumnType::Boolean, "postgres"), "BOOLEAN");
        assert_eq!(sql_type(ColumnType::Json, "postgres"), "JSONB");
        assert_eq!(sql_type(ColumnType::Timestamp, "postgres"), "TIMESTAMPTZ");
    }

    #[test]
    fn from_config_empty_table() {
        let config = HashMap::new();
        let result = TableSchema::from_config(&config).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_config_table_no_schema() {
        let mut config = HashMap::new();
        config.insert(
            "table".to_string(),
            serde_json::Value::String("users".to_string()),
        );
        let result = TableSchema::from_config(&config).unwrap().unwrap();
        assert_eq!(result.table, "users");
        assert!(result.columns.is_empty());
    }

    #[test]
    fn from_config_with_columns() {
        let mut config = HashMap::new();
        config.insert(
            "table".to_string(),
            serde_json::Value::String("events".to_string()),
        );
        config.insert(
            "schema".to_string(),
            serde_json::json!([
                {"name": "title", "type": "text", "nullable": false},
                {"name": "count", "type": "integer"},
                {"name": "payload", "type": "json", "nullable": true, "default": "{}"},
            ]),
        );

        let schema = TableSchema::from_config(&config).unwrap().unwrap();
        assert_eq!(schema.table, "events");
        assert_eq!(schema.columns.len(), 3);

        assert_eq!(schema.columns[0].name, "title");
        assert_eq!(schema.columns[0].col_type, ColumnType::Text);
        assert!(!schema.columns[0].nullable);

        assert_eq!(schema.columns[1].name, "count");
        assert_eq!(schema.columns[1].col_type, ColumnType::Integer);
        assert!(schema.columns[1].nullable);

        assert_eq!(schema.columns[2].name, "payload");
        assert_eq!(schema.columns[2].col_type, ColumnType::Json);
        assert_eq!(schema.columns[2].default.as_deref(), Some("{}"));
    }

    #[test]
    fn to_columns_list_excludes_system() {
        let schema = TableSchema {
            table: "data".to_string(),
            columns: vec![
                ColumnDef {
                    name: "id".to_string(),
                    col_type: ColumnType::Text,
                    nullable: false,
                    default: None,
                },
                ColumnDef {
                    name: "title".to_string(),
                    col_type: ColumnType::Text,
                    nullable: true,
                    default: None,
                },
                ColumnDef {
                    name: "created_at".to_string(),
                    col_type: ColumnType::Timestamp,
                    nullable: false,
                    default: None,
                },
                ColumnDef {
                    name: "value".to_string(),
                    col_type: ColumnType::Float,
                    nullable: true,
                    default: None,
                },
            ],
        };

        let list = schema.to_columns_list();
        // "id" and "created_at" are system columns -- filtered out
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"], "title");
        assert_eq!(list[1]["name"], "value");
    }

    #[test]
    fn column_type_serde_roundtrip() {
        let ct = ColumnType::Timestamp;
        let json = serde_json::to_string(&ct).unwrap();
        assert_eq!(json, "\"timestamp\"");
        let back: ColumnType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ColumnType::Timestamp);
    }
}
