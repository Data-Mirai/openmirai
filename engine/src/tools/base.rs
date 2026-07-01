use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// ToolField constructor (single source — used by all tool modules)
// ---------------------------------------------------------------------------

/// Build a `ToolField` with the given name, type, required flag, and description.
/// This is the canonical constructor — every tool module imports it instead of
/// defining a local copy.
/// Build a `ToolField` with the given name, type, required flag, and description.
/// This is the canonical constructor — every tool module imports it instead of
/// defining a local copy.
pub fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
        required,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.into())
        },
        default: None,
        min_value: None,
        max_value: None,
        min_length: None,
        max_length: None,
        regex_format: None,
    }
}

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

    // Structural validations (Milestone 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex_format: Option<String>,
}

impl ToolField {
    pub fn min_value(mut self, min: f64) -> Self {
        self.min_value = Some(min);
        self
    }

    pub fn max_value(mut self, max: f64) -> Self {
        self.max_value = Some(max);
        self
    }

    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    pub fn regex_format(mut self, pattern: &str) -> Self {
        self.regex_format = Some(pattern.to_string());
        self
    }

    pub fn validate_value(&self, value: &Value, context_label: &str) -> Result<(), String> {
        // First check type matches
        if !self.field_type.matches(value) {
            return Err(format!(
                "{}: expected type {}, got {}",
                context_label,
                self.field_type,
                value_type_label(value)
            ));
        }

        // 1. Numeric bounds (Number or Integer)
        if self.min_value.is_some() || self.max_value.is_some() {
            if let Some(num) = value.as_f64() {
                if let Some(min) = self.min_value {
                    if num < min {
                        return Err(format!(
                            "{}: must be >= {}, got {}",
                            context_label, min, num
                        ));
                    }
                }
                if let Some(max) = self.max_value {
                    if num > max {
                        return Err(format!(
                            "{}: must be <= {}, got {}",
                            context_label, max, num
                        ));
                    }
                }
            }
        }

        // 2. String length
        if self.min_length.is_some() || self.max_length.is_some() {
            if let Some(s) = value.as_str() {
                let len = s.chars().count();
                if let Some(min) = self.min_length {
                    if len < min {
                        return Err(format!(
                            "{}: length must be >= {}, got {}",
                            context_label, min, len
                        ));
                    }
                }
                if let Some(max) = self.max_length {
                    if len > max {
                        return Err(format!(
                            "{}: length must be <= {}, got {}",
                            context_label, max, len
                        ));
                    }
                }
            }
        }

        // 3. String regex format
        if let Some(ref pattern) = self.regex_format {
            if let Some(s) = value.as_str() {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(s) {
                        return Err(format!(
                            "{}: does not match format pattern '{}'",
                            context_label, pattern
                        ));
                    }
                }
            }
        }

        Ok(())
    }
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
// Node Input & Config Validation (PRD-004 Capa 2 / Milestone 3)
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
/// Checks required fields are present and types/structural constraints match.
/// Applies defaults from ToolField.default for optional missing fields.
/// Returns enriched inputs or a list of validation error messages.
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
                if let Err(err) = field.validate_value(value, &format!("Node '{}' input '{}'", node_id, field.name)) {
                    errors.push(err);
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

/// Validate configuration fields against a tool's declared ToolSpec.config_fields.
pub fn validate_node_config(
    config: &std::collections::HashMap<String, Value>,
    tool_spec: &ToolSpec,
    node_id: &str,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    for field in &tool_spec.config_fields {
        match config.get(&field.name) {
            Some(value) => {
                if let Err(err) = field.validate_value(value, &format!("Node '{}' config '{}'", node_id, field.name)) {
                    errors.push(err);
                }
            }
            None => {
                if field.required {
                    errors.push(format!(
                        "Node '{}': missing required configuration field '{}' (type: {})",
                        node_id, field.name, tool_spec.tool_type,
                    ));
                } else if field.default.is_some() {
                    // Config doesn't have an enrichment layer in registry since it is passed as a ref,
                    // but we can flag type/structural mismatches if present.
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// value_type_label: canonical source is core::value_type::value_type_label
use crate::core::value_type::value_type_label;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_field_serde_roundtrip() {
        let field = field("prompt", FieldType::String, true, "The user prompt");
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
        let mut field = field("temperature", FieldType::Number, false, "");
        field.default = Some(json!(0.7));
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
            inputs: vec![field("field", FieldType::String, true, "")],
            outputs: vec![field("result", FieldType::Boolean, true, "")],
            config_fields: vec![],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: ToolSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_type, "logic/condition");
        assert_eq!(back.inputs.len(), 1);
        assert_eq!(back.outputs.len(), 1);
    }

    #[test]
    fn test_structural_validations() {
        // Test numeric bounds
        let num_field = field("num", FieldType::Integer, true, "")
            .min_value(10.0)
            .max_value(20.0);
        assert!(num_field.validate_value(&json!(15), "test").is_ok());
        assert!(num_field.validate_value(&json!(9), "test").is_err());
        assert!(num_field.validate_value(&json!(21), "test").is_err());

        // Test string length bounds
        let str_field = field("str", FieldType::String, true, "")
            .min_length(3)
            .max_length(5);
        assert!(str_field.validate_value(&json!("abc"), "test").is_ok());
        assert!(str_field.validate_value(&json!("ab"), "test").is_err());
        assert!(str_field.validate_value(&json!("abcdef"), "test").is_err());

        // Test regex validation
        let re_field = field("code", FieldType::String, true, "")
            .regex_format(r"^[A-Z]{3}-\d{3}$");
        assert!(re_field.validate_value(&json!("ABC-123"), "test").is_ok());
        assert!(re_field.validate_value(&json!("abc-123"), "test").is_err());
        assert!(re_field.validate_value(&json!("ABCD-123"), "test").is_err());
    }

    #[test]
    fn test_validate_node_config_fields() {
        let spec = ToolSpec {
            tool_type: "test/validator".into(),
            name: "Test Validator".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            category: "test".into(),
            inputs: vec![],
            outputs: vec![],
            config_fields: vec![
                field("port", FieldType::Integer, true, "").min_value(1024.0).max_value(65535.0),
                field("host", FieldType::String, false, "").regex_format(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$"),
            ],
        };

        // Valid configuration
        let mut valid_config = std::collections::HashMap::new();
        valid_config.insert("port".to_string(), json!(8080));
        valid_config.insert("host".to_string(), json!("127.0.0.1"));
        assert!(validate_node_config(&valid_config, &spec, "n1").is_ok());

        // Invalid port
        let mut invalid_port = std::collections::HashMap::new();
        invalid_port.insert("port".to_string(), json!(80));
        assert!(validate_node_config(&invalid_port, &spec, "n1").is_err());

        // Invalid host format
        let mut invalid_host = std::collections::HashMap::new();
        invalid_host.insert("port".to_string(), json!(8080));
        invalid_host.insert("host".to_string(), json!("localhost"));
        assert!(validate_node_config(&invalid_host, &spec, "n1").is_err());
    }
}
