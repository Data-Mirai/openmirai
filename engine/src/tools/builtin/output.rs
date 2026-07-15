use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType};
use crate::tools::registry::{Tool, ToolRegistry};


// ===========================================================================
// ResponseTool
// ===========================================================================

output_tool! {
    struct ResponseTool, factory ResponseFactory;
    tool_type = "output/response",
    name = "Response",
    description = "Terminal node that formats and returns the final session result",
    inputs = [
        field("data", FieldType::Object, false, "Data from previous step (auto-connected via data_map)"),
    ],
    outputs = [
        field("result", FieldType::String, true, "Formatted result"),
        field("format", FieldType::String, true, "Output format used"),
        field("raw_data", FieldType::Object, true, "Raw data received"),
    ],
    config_fields = [
        field("format", FieldType::String, false, "Output format: text, json, markdown, bullets (default: markdown)"),
        field("template", FieldType::String, false, "Template with ${key} placeholders"),
        field("title", FieldType::String, false, "Optional title for the result"),
    ]
}

#[async_trait]
impl Tool for ResponseTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_data = inputs.get("data").cloned().unwrap_or_else(|| json!(inputs));
        let template = config
            .get("template")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let fmt = config
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("markdown");
        let title = config.get("title").and_then(|v| v.as_str()).unwrap_or("");

        let result = if !template.is_empty() {
            apply_template(template, &raw_data)
        } else {
            auto_format(&raw_data, fmt, title)
        };

        let raw_out = if raw_data.is_object() {
            raw_data
        } else {
            json!({"data": raw_data})
        };

        let mut out = HashMap::new();
        out.insert("result".to_string(), json!(result));
        out.insert("format".to_string(), json!(fmt));
        out.insert("raw_data".to_string(), raw_out);
        Ok(out)
    }
}

/// Replace ${key} placeholders with values from data.
fn apply_template(template: &str, data: &Value) -> String {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return template.replace("${data}", &data.to_string()),
    };

    let re = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        match obj.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => caps[0].to_string(),
        }
    })
    .to_string()
}

/// Auto-format data based on format type.
fn auto_format(data: &Value, fmt: &str, title: &str) -> String {
    let header = if title.is_empty() {
        String::new()
    } else {
        format!("# {}\n\n", title)
    };

    match fmt {
        "json" => {
            let content = serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string());
            format!("{}{}", header, content)
        }
        "bullets" => {
            let content = extract_content(data);
            let lines: Vec<String> = content
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(|l| format!("- {}", l))
                .collect();
            format!("{}{}", header, lines.join("\n"))
        }
        "text" => {
            let content = extract_content(data);
            if title.is_empty() {
                content
            } else {
                format!("{}\n\n{}", title, content)
            }
        }
        _ => {
            // markdown (default)
            let content = extract_content(data);
            format!("{}{}", header, content)
        }
    }
}

/// Extract the most meaningful string from a value.
fn extract_content(data: &Value) -> String {
    match data {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            for key in ["result", "output", "text", "content", "response", "summary"] {
                if let Some(Value::String(s)) = map.get(key) {
                    return s.clone();
                }
            }
            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        }
        Value::Array(arr) => arr
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => data.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all output tools into the given registry.
pub fn register_output_tools(registry: &mut ToolRegistry) {
    registry.register("output/response", Box::new(ResponseFactory::new()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::InMemoryContext;

    fn ctx() -> InMemoryContext {
        InMemoryContext::new("test-run")
    }

    #[tokio::test]
    async fn response_json_format() {
        let tool = ResponseTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!({"key": "value"}));
        let mut config = HashMap::new();
        config.insert("format".to_string(), json!("json"));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        assert_eq!(result["format"], json!("json"));
        let text = result["result"].as_str().unwrap();
        assert!(text.contains("key"));
    }

    #[tokio::test]
    async fn response_bullets_format() {
        let tool = ResponseTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!("line1\nline2\nline3"));
        let mut config = HashMap::new();
        config.insert("format".to_string(), json!("bullets"));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        let text = result["result"].as_str().unwrap();
        assert!(text.contains("- line1"));
        assert!(text.contains("- line2"));
    }

    #[tokio::test]
    async fn response_template() {
        let tool = ResponseTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!({"name": "Alice", "score": 95}));
        let mut config = HashMap::new();
        config.insert(
            "template".to_string(),
            json!("Hello ${name}, your score is ${score}"),
        );
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();
        let text = result["result"].as_str().unwrap();
        assert!(text.contains("Hello Alice"));
        assert!(text.contains("95"));
    }

    #[test]
    fn register_output_tools_adds_one() {
        let mut reg = ToolRegistry::new();
        register_output_tools(&mut reg);
        assert!(reg.get("output/response").is_some());
        assert_eq!(reg.list_tools().len(), 1);
    }
}
