// Example implementation of a custom OpenMirai tool.
// This is not compiled with the main crate, but serves as a copy-paste template.

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};

use openmirai_engine::core::context::ExecutionContext;
use openmirai_engine::core::runner::ToolError;
use openmirai_engine::tools::base::{field, FieldType, ToolSpec};
use openmirai_engine::tools::registry::{Tool, ToolFactory, ToolRegistry};

// Macro defines struct CustomMathTool and struct CustomMathFactory
logic_tool! {
    struct CustomMathTool, factory CustomMathFactory;
    tool_type = "logic/custom_math",
    name = "Custom Math",
    description = "Performs mathematical calculations on input numbers",
    inputs = [
        field("a", FieldType::Number, true, "First operand"),
        field("b", FieldType::Number, true, "Second operand"),
        field("operation", FieldType::String, false, "Operation to perform: add, subtract, multiply, divide (default: add)"),
    ],
    outputs = [
        field("result", FieldType::Number, true, "Calculated result"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for CustomMathTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let a = inputs.get("a").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "logic/custom_math".into(),
                message: "input 'a' must be a valid number".into(),
            }
        })?;

        let b = inputs.get("b").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "logic/custom_math".into(),
                message: "input 'b' must be a valid number".into(),
            }
        })?;

        let operation = inputs
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("add");

        let result = match operation {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" => {
                if b == 0.0 {
                    return Err(ToolError::ExecutionFailed {
                        tool_type: "logic/custom_math".into(),
                        message: "division by zero is undefined".into(),
                    });
                }
                a / b
            }
            other => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "logic/custom_math".into(),
                    message: format!("unknown operation: {}", other),
                })
            }
        };

        let mut out = HashMap::new();
        out.insert("result".to_string(), json!(result));
        Ok(out)
    }
}
