use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// ToolField
// ---------------------------------------------------------------------------

/// Describes a single input, output, or config field of a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolField {
    pub name: String,
    /// Logical type: `"string"`, `"number"`, `"boolean"`, `"object"`, `"array"`.
    pub field_type: String,
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
            field_type: "string".into(),
            required: true,
            description: Some("The user prompt".into()),
            default: None,
        };
        let json = serde_json::to_string(&field).unwrap();
        let back: ToolField = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "prompt");
        assert!(back.required);
        assert!(back.default.is_none());
    }

    #[test]
    fn tool_field_with_default() {
        let field = ToolField {
            name: "temperature".into(),
            field_type: "number".into(),
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
    fn tool_spec_serde_roundtrip() {
        let spec = ToolSpec {
            tool_type: "logic/condition".into(),
            name: "Condition".into(),
            description: "Evaluate a boolean condition".into(),
            version: "1.0.0".into(),
            category: "logic".into(),
            inputs: vec![ToolField {
                name: "field".into(),
                field_type: "string".into(),
                required: true,
                description: None,
                default: None,
            }],
            outputs: vec![ToolField {
                name: "result".into(),
                field_type: "boolean".into(),
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
