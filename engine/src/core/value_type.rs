//! Unified value type system used across the engine.
//!
//! Replaces the dual `InputType` (agent specs) / `FieldType` (tool specs)
//! with a single enum and a single validation function.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// ValueType
// ---------------------------------------------------------------------------

/// Logical type of a value flowing through the engine.
///
/// Used by both agent input/output schemas and tool field specifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    /// String / text values.
    Text,
    /// Any numeric value (float or integer).
    Number,
    /// Integer-only numeric values.
    Integer,
    /// Boolean values.
    Boolean,
    /// JSON arrays.
    Array,
    /// JSON objects.
    Object,
    /// File path (stored as string).
    File,
}

/// Custom deserializer that accepts aliases for backward compat:
/// - "string" → Text
/// - "json" → Object (legacy InputType::Json accepted as Object)
impl<'de> Deserialize<'de> for ValueType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "text" | "string" => Ok(ValueType::Text),
            "number" => Ok(ValueType::Number),
            "integer" => Ok(ValueType::Integer),
            "boolean" => Ok(ValueType::Boolean),
            "array" => Ok(ValueType::Array),
            "object" => Ok(ValueType::Object),
            "json" => Ok(ValueType::Object), // legacy alias
            "file" => Ok(ValueType::File),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["text", "string", "number", "integer", "boolean", "array", "object", "json", "file"],
            )),
        }
    }
}

impl ValueType {
    /// Check if a serde_json::Value matches this expected type.
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            ValueType::Text => value.is_string(),
            ValueType::Number => value.is_number(),
            ValueType::Integer => value.is_i64() || value.is_u64(),
            ValueType::Boolean => value.is_boolean(),
            ValueType::Array => value.is_array(),
            ValueType::Object => value.is_object(),
            ValueType::File => value.is_string(),
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text => write!(f, "text"),
            Self::Number => write!(f, "number"),
            Self::Integer => write!(f, "integer"),
            Self::Boolean => write!(f, "boolean"),
            Self::Array => write!(f, "array"),
            Self::Object => write!(f, "object"),
            Self::File => write!(f, "file"),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Human-readable label for a JSON value's actual type.
pub fn value_type_label(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Validate a set of inputs against a field schema.
///
/// Checks required fields are present and types match.  Applies defaults
/// for optional missing fields.  Returns enriched inputs or all errors
/// (not fail-fast — reports every error at once).
pub fn validate_inputs(
    inputs: &HashMap<String, Value>,
    schema: &[(String, ValueType, bool, Option<Value>)], // (name, type, required, default)
    context: &str,
) -> Result<HashMap<String, Value>, Vec<String>> {
    let mut enriched = inputs.clone();
    let mut errors = Vec::new();

    for (name, expected_type, required, default) in schema {
        match inputs.get(name) {
            Some(value) => {
                if !expected_type.matches(value) {
                    errors.push(format!(
                        "{context}: input '{name}' expected {expected_type}, got {}",
                        value_type_label(value),
                    ));
                }
            }
            None => {
                if *required {
                    errors.push(format!("{context}: missing required input '{name}'"));
                } else if let Some(ref default_val) = default {
                    enriched.insert(name.clone(), default_val.clone());
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_all_variants() {
        assert!(ValueType::Text.matches(&json!("hello")));
        assert!(!ValueType::Text.matches(&json!(42)));
        assert!(ValueType::Number.matches(&json!(1.5)));
        assert!(ValueType::Number.matches(&json!(42)));
        assert!(!ValueType::Number.matches(&json!("three")));
        assert!(ValueType::Integer.matches(&json!(42)));
        assert!(!ValueType::Integer.matches(&json!(1.5)));
        assert!(ValueType::Boolean.matches(&json!(true)));
        assert!(!ValueType::Boolean.matches(&json!(1)));
        assert!(ValueType::Array.matches(&json!([1, 2, 3])));
        assert!(!ValueType::Array.matches(&json!("string")));
        assert!(ValueType::Object.matches(&json!({"key": "val"})));
        assert!(!ValueType::Object.matches(&json!("string")));
        assert!(ValueType::File.matches(&json!("/path/to/file.txt")));
        assert!(!ValueType::File.matches(&json!(123)));
    }

    #[test]
    fn serde_roundtrip() {
        for (variant, expected) in [
            (ValueType::Text, "\"text\""),
            (ValueType::Number, "\"number\""),
            (ValueType::Integer, "\"integer\""),
            (ValueType::Boolean, "\"boolean\""),
            (ValueType::Array, "\"array\""),
            (ValueType::Object, "\"object\""),
            (ValueType::File, "\"file\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let back: ValueType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn deserialize_aliases() {
        // "string" → Text
        let t: ValueType = serde_json::from_str("\"string\"").unwrap();
        assert_eq!(t, ValueType::Text);
        // "json" → Object (legacy)
        let t: ValueType = serde_json::from_str("\"json\"").unwrap();
        assert_eq!(t, ValueType::Object);
    }

    #[test]
    fn rejects_invalid_type() {
        let result: Result<ValueType, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
    }

    #[test]
    fn validate_inputs_happy_path() {
        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), json!("hello"));

        let schema = vec![
            ("name".to_string(), ValueType::Text, true, None),
            ("count".to_string(), ValueType::Number, false, Some(json!(10))),
        ];

        let result = validate_inputs(&inputs, &schema, "test").unwrap();
        assert_eq!(result["name"], json!("hello"));
        assert_eq!(result["count"], json!(10)); // default applied
    }

    #[test]
    fn validate_inputs_missing_required() {
        let inputs = HashMap::new();
        let schema = vec![("name".to_string(), ValueType::Text, true, None)];
        let errors = validate_inputs(&inputs, &schema, "test").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing required"));
    }

    #[test]
    fn validate_inputs_type_mismatch() {
        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), json!(42));

        let schema = vec![("name".to_string(), ValueType::Text, true, None)];
        let errors = validate_inputs(&inputs, &schema, "test").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("expected text"));
        assert!(errors[0].contains("got number"));
    }
}
