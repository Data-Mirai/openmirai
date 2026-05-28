use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// FieldType
// ---------------------------------------------------------------------------

/// Logical type of a tool field. Replaces free-form strings to prevent typos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Number,
    Boolean,
    Array,
    Object,
    Integer,
    /// PRD-010: A file reference — `Value::Object` with `_type: "file_ref"` and `path`.
    File,
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String => write!(f, "string"),
            Self::Number => write!(f, "number"),
            Self::Boolean => write!(f, "boolean"),
            Self::Array => write!(f, "array"),
            Self::Object => write!(f, "object"),
            Self::Integer => write!(f, "integer"),
            Self::File => write!(f, "file"),
        }
    }
}

// ---------------------------------------------------------------------------
// ToolField
// ---------------------------------------------------------------------------

/// Describes a single input, output, or config field of a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolField {
    pub name: String,
    /// Logical type of this field.
    pub field_type: FieldType,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

// ---------------------------------------------------------------------------
// ToolSpec
// ---------------------------------------------------------------------------

/// Static metadata describing a tool (type, schema, version).
///
/// `ToolSpec` is the serializable "spec sheet" that the editor and registry
/// use to understand what a tool accepts and produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Category-qualified identifier, e.g. `"ai/llm_call"`, `"logic/condition"`.
    pub tool_type: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub category: String,
    pub inputs: Vec<ToolField>,
    pub outputs: Vec<ToolField>,
    pub config_fields: Vec<ToolField>,
}

// ---------------------------------------------------------------------------
// Node Input Validation (PRD-004 Capa 2)
// ---------------------------------------------------------------------------

impl FieldType {
    /// Check if a serde_json::Value matches this FieldType.
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            FieldType::String => value.is_string(),
            FieldType::Number => value.is_number(),
            FieldType::Boolean => value.is_boolean(),
            FieldType::Array => value.is_array(),
            FieldType::Object => value.is_object(),
            FieldType::Integer => value.is_i64() || value.is_u64(),
            FieldType::File => crate::llm::media::is_file_ref(value),
        }
    }
}

/// Validate resolved inputs against a tool's declared ToolSpec.inputs.
///
/// Checks required fields are present and types match. Applies defaults
/// from ToolField.default for optional missing fields. Returns enriched
/// inputs or a list of validation error messages.
pub fn validate_node_inputs(
    inputs: &std::collections::HashMap<String, Value>,
    tool_spec: &ToolSpec,
    node_id: &str,
) -> Result<std::collections::HashMap<String, Value>, Vec<String>> {
    let mut enriched = inputs.clone();
    let mut errors = Vec::new();

    for field in &tool_spec.inputs {
        match inputs.get(&field.name) {
            Some(value) => {
                if !field.field_type.matches(value) {
                    errors.push(format!(
                        "Node '{}': input '{}' expected {}, got {}",
                        node_id, field.name, field.field_type,
                        value_type_label(value),
                    ));
                }
            }
            None => {
                if field.required {
                    errors.push(format!(
                        "Node '{}': missing required input '{}' (type: {})",
                        node_id, field.name, tool_spec.tool_type,
                    ));
                } else if let Some(ref default) = field.default {
                    enriched.insert(field.name.clone(), default.clone());
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(enriched)
    } else {
        Err(errors)
    }
}

fn value_type_label(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_field_serde_roundtrip() {
        let field = ToolField {
            name: "prompt".into(),
            field_type: FieldType::String,
            required: true,
            description: Some("The user prompt".into()),
            default: None,
        };
        let json = serde_json::to_string(&field).unwrap();
        assert!(json.contains("\"field_type\":\"string\""));
        let back: ToolField = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "prompt");
        assert_eq!(back.field_type, FieldType::String);
        assert!(back.required);
        assert!(back.default.is_none());
    }

    #[test]
    fn tool_field_with_default() {
        let field = ToolField {
            name: "temperature".into(),
            field_type: FieldType::Number,
            required: false,
            description: None,
            default: Some(json!(0.7)),
        };
        let json = serde_json::to_string(&field).unwrap();
        assert!(json.contains("0.7"));
        let back: ToolField = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default, Some(json!(0.7)));
    }

    #[test]
    fn field_type_rejects_invalid() {
        let result: Result<FieldType, _> = serde_json::from_str("\"stirng\"");
        assert!(result.is_err());
    }

    #[test]
    fn field_type_all_variants_roundtrip() {
        for (variant, expected) in [
            (FieldType::String, "\"string\""),
            (FieldType::Number, "\"number\""),
            (FieldType::Boolean, "\"boolean\""),
            (FieldType::Array, "\"array\""),
            (FieldType::Object, "\"object\""),
            (FieldType::Integer, "\"integer\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let back: FieldType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn tool_spec_serde_roundtrip() {
        let spec = ToolSpec {
            tool_type: "logic/condition".into(),
            name: "Condition".into(),
            description: "Evaluate a boolean condition".into(),
            version: "1.0.0".into(),
            category: "logic".into(),
            inputs: vec![ToolField {
                name: "field".into(),
                field_type: FieldType::String,
                required: true,
                description: None,
                default: None,
            }],
            outputs: vec![ToolField {
                name: "result".into(),
                field_type: FieldType::Boolean,
                required: true,
                description: None,
                default: None,
            }],
            config_fields: vec![],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: ToolSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_type, "logic/condition");
        assert_eq!(back.inputs.len(), 1);
        assert_eq!(back.outputs.len(), 1);
    }
}
