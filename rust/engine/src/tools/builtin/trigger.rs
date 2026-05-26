use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{FieldType, ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
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
    }
}

// ---------------------------------------------------------------------------
// Macro
// ---------------------------------------------------------------------------

macro_rules! trigger_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: "trigger".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ===========================================================================
// WebhookTriggerTool
// ===========================================================================

trigger_tool! {
    struct WebhookTriggerTool, factory WebhookTriggerFactory;
    tool_type = "trigger/webhook",
    name = "Webhook",
    description = "HTTP webhook entry point for graph execution",
    inputs = [],
    outputs = [
        field("body", FieldType::Object, true, "Request body"),
        field("headers", FieldType::Object, true, "Request headers"),
        field("query_params", FieldType::Object, true, "Query parameters"),
    ],
    config_fields = [
        field("mock_payload", FieldType::Object, false, "Mock payload for testing"),
    ]
}

#[async_trait]
impl Tool for WebhookTriggerTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let body = config
            .get("body")
            .or_else(|| config.get("mock_payload"))
            .cloned()
            .unwrap_or(json!({}));
        let headers = config
            .get("headers")
            .cloned()
            .unwrap_or(json!({}));
        let query_params = config
            .get("query_params")
            .cloned()
            .unwrap_or(json!({}));

        let mut out = HashMap::new();
        out.insert("body".to_string(), body);
        out.insert("headers".to_string(), headers);
        out.insert("query_params".to_string(), query_params);
        Ok(out)
    }
}

// ===========================================================================
// ManualTriggerTool
// ===========================================================================

trigger_tool! {
    struct ManualTriggerTool, factory ManualTriggerFactory;
    tool_type = "trigger/manual",
    name = "Manual",
    description = "Manual execution entry point",
    inputs = [],
    outputs = [
        field("user_input", FieldType::Object, true, "User input data"),
        field("triggered_by", FieldType::String, true, "Who triggered the execution"),
        field("timestamp", FieldType::Number, true, "Trigger timestamp"),
    ],
    config_fields = [
        field("mock_payload", FieldType::Object, false, "Mock payload for testing"),
    ]
}

#[async_trait]
impl Tool for ManualTriggerTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let user_input = config
            .get("user_input")
            .or_else(|| config.get("mock_payload"))
            .cloned()
            .unwrap_or(json!({}));
        let triggered_by = config
            .get("triggered_by")
            .and_then(|v| v.as_str())
            .unwrap_or("manual");

        let now = Utc::now();
        let timestamp = now.timestamp() as f64
            + now.timestamp_subsec_millis() as f64 / 1000.0;

        let mut out = HashMap::new();
        out.insert("user_input".to_string(), user_input);
        out.insert("triggered_by".to_string(), json!(triggered_by));
        out.insert("timestamp".to_string(), json!(timestamp));
        Ok(out)
    }
}

// ===========================================================================
// ScheduleTriggerTool
// ===========================================================================

trigger_tool! {
    struct ScheduleTriggerTool, factory ScheduleTriggerFactory;
    tool_type = "trigger/schedule",
    name = "Schedule",
    description = "Cron/interval trigger for scheduled execution",
    inputs = [],
    outputs = [
        field("triggered_at", FieldType::Number, true, "Trigger timestamp"),
        field("run_count", FieldType::Number, true, "Number of runs so far"),
    ],
    config_fields = [
        field("run_count", FieldType::Number, false, "Current run count"),
    ]
}

#[async_trait]
impl Tool for ScheduleTriggerTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let now = Utc::now();
        let triggered_at = now.timestamp() as f64
            + now.timestamp_subsec_millis() as f64 / 1000.0;
        let run_count = config
            .get("run_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let mut out = HashMap::new();
        out.insert("triggered_at".to_string(), json!(triggered_at));
        out.insert("run_count".to_string(), json!(run_count));
        Ok(out)
    }
}

// ===========================================================================
// EventTriggerTool
// ===========================================================================

trigger_tool! {
    struct EventTriggerTool, factory EventTriggerFactory;
    tool_type = "trigger/event",
    name = "Event",
    description = "Resource event trigger that reacts to system events",
    inputs = [],
    outputs = [
        field("source", FieldType::String, true, "Event source"),
        field("event_type", FieldType::String, true, "Type of event"),
        field("event_data", FieldType::Object, true, "Event payload data"),
    ],
    config_fields = [
        field("source", FieldType::String, false, "Event source identifier"),
        field("event_type", FieldType::String, false, "Expected event type"),
    ]
}

#[async_trait]
impl Tool for EventTriggerTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let source = config
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let event_type = config
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let event_data = config
            .get("event_data")
            .cloned()
            .unwrap_or(json!({}));

        let mut out = HashMap::new();
        out.insert("source".to_string(), json!(source));
        out.insert("event_type".to_string(), json!(event_type));
        out.insert("event_data".to_string(), event_data);
        Ok(out)
    }
}

// ===========================================================================
// HeartbeatTriggerTool
// ===========================================================================

trigger_tool! {
    struct HeartbeatTriggerTool, factory HeartbeatTriggerFactory;
    tool_type = "trigger/heartbeat",
    name = "Heartbeat",
    description = "Periodic trigger that evaluates a condition at configurable intervals. Used for polling, health checks, and scheduled re-evaluations.",
    inputs = [],
    outputs = [
        field("triggered", FieldType::Boolean, true, "Whether the condition evaluated to true"),
        field("triggered_at", FieldType::String, true, "ISO 8601 timestamp of evaluation"),
        field("evaluation_count", FieldType::Number, true, "Number of evaluations performed"),
        field("condition_result", FieldType::Object, false, "Result details from the condition evaluator"),
    ],
    config_fields = [
        field("interval_seconds", FieldType::Number, false, "Evaluation interval in seconds (default: 300)"),
        field("condition_type", FieldType::String, false, "Condition evaluator: always_true, always_false, custom_expression (default: always_true)"),
        field("condition_config", FieldType::String, false, "Configuration for the condition evaluator"),
    ]
}

#[async_trait]
impl Tool for HeartbeatTriggerTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let condition_type = config
            .get("condition_type")
            .and_then(|v| v.as_str())
            .unwrap_or("always_true");

        let condition_config = config
            .get("condition_config")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let evaluation_count = config
            .get("evaluation_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            + 1;

        let now = Utc::now();
        let triggered_at = now.to_rfc3339();

        // Evaluate condition
        let (triggered, condition_result) = match condition_type {
            "always_true" => (true, json!({"type": "always_true"})),
            "always_false" => (false, json!({"type": "always_false"})),
            "custom_expression" => {
                // Safe evaluation: only support simple boolean expressions
                // with comparison operators on numeric literals
                let result = evaluate_simple_expression(condition_config);
                (result, json!({"type": "custom_expression", "expression": condition_config, "result": result}))
            }
            other => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "trigger/heartbeat".into(),
                    message: format!("Unknown condition_type: {other}. Use: always_true, always_false, custom_expression"),
                });
            }
        };

        let mut out = HashMap::new();
        out.insert("triggered".to_string(), json!(triggered));
        out.insert("triggered_at".to_string(), json!(triggered_at));
        out.insert("evaluation_count".to_string(), json!(evaluation_count));
        out.insert("condition_result".to_string(), condition_result);
        Ok(out)
    }
}

/// Evaluate a simple numeric comparison expression.
/// Supports: "N op M" where op is <, >, <=, >=, ==, !=
/// Returns false for invalid expressions (safe default).
fn evaluate_simple_expression(expr: &str) -> bool {
    let expr = expr.trim();
    if expr.is_empty() {
        return true;
    }

    // Try each operator (longer operators first to avoid prefix conflicts)
    for op in &["<=", ">=", "!=", "==", "<", ">"] {
        if let Some(pos) = expr.find(op) {
            let left = expr[..pos].trim().parse::<f64>();
            let right = expr[pos + op.len()..].trim().parse::<f64>();
            if let (Ok(l), Ok(r)) = (left, right) {
                return match *op {
                    "<" => l < r,
                    ">" => l > r,
                    "<=" => l <= r,
                    ">=" => l >= r,
                    "==" => (l - r).abs() < f64::EPSILON,
                    "!=" => (l - r).abs() >= f64::EPSILON,
                    _ => false,
                };
            }
        }
    }

    // If we can't parse, return false (safe default)
    false
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all trigger tools into the given registry.
pub fn register_trigger_tools(registry: &mut ToolRegistry) {
    registry.register("trigger/webhook", Box::new(WebhookTriggerFactory::new()));
    registry.register("trigger/manual", Box::new(ManualTriggerFactory::new()));
    registry.register("trigger/schedule", Box::new(ScheduleTriggerFactory::new()));
    registry.register("trigger/event", Box::new(EventTriggerFactory::new()));
    registry.register("trigger/heartbeat", Box::new(HeartbeatTriggerFactory::new()));
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
    async fn webhook_trigger_returns_body() {
        let tool = WebhookTriggerTool;
        let mut config = HashMap::new();
        config.insert("body".to_string(), json!({"key": "value"}));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["body"]["key"], json!("value"));
        assert!(result["headers"].is_object());
    }

    #[tokio::test]
    async fn manual_trigger_returns_timestamp() {
        let tool = ManualTriggerTool;
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx())
            .await
            .unwrap();
        assert_eq!(result["triggered_by"], json!("manual"));
        assert!(result["timestamp"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn schedule_trigger_returns_run_count() {
        let tool = ScheduleTriggerTool;
        let mut config = HashMap::new();
        config.insert("run_count".to_string(), json!(5));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["run_count"], json!(5));
        assert!(result["triggered_at"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn event_trigger_returns_source() {
        let tool = EventTriggerTool;
        let mut config = HashMap::new();
        config.insert("source".to_string(), json!("db"));
        config.insert("event_type".to_string(), json!("insert"));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["source"], json!("db"));
        assert_eq!(result["event_type"], json!("insert"));
    }

    #[tokio::test]
    async fn heartbeat_trigger_always_true() {
        let tool = HeartbeatTriggerTool;
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx())
            .await
            .unwrap();
        assert_eq!(result["triggered"], json!(true));
        assert!(result["triggered_at"].as_str().is_some());
        assert_eq!(result["evaluation_count"], json!(1));
    }

    #[tokio::test]
    async fn heartbeat_trigger_always_false() {
        let tool = HeartbeatTriggerTool;
        let mut config = HashMap::new();
        config.insert("condition_type".to_string(), json!("always_false"));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["triggered"], json!(false));
    }

    #[tokio::test]
    async fn heartbeat_trigger_custom_expression() {
        let tool = HeartbeatTriggerTool;
        let mut config = HashMap::new();
        config.insert("condition_type".to_string(), json!("custom_expression"));
        config.insert("condition_config".to_string(), json!("5 > 3"));
        let result = tool
            .execute(HashMap::new(), &config, &ctx())
            .await
            .unwrap();
        assert_eq!(result["triggered"], json!(true));

        let mut config2 = HashMap::new();
        config2.insert("condition_type".to_string(), json!("custom_expression"));
        config2.insert("condition_config".to_string(), json!("1 > 10"));
        let result2 = tool
            .execute(HashMap::new(), &config2, &ctx())
            .await
            .unwrap();
        assert_eq!(result2["triggered"], json!(false));
    }

    #[tokio::test]
    async fn heartbeat_trigger_unknown_condition_errors() {
        let tool = HeartbeatTriggerTool;
        let mut config = HashMap::new();
        config.insert("condition_type".to_string(), json!("nonexistent"));
        let result = tool.execute(HashMap::new(), &config, &ctx()).await;
        assert!(result.is_err());
    }

    #[test]
    fn evaluate_simple_expression_cases() {
        assert!(super::evaluate_simple_expression("5 > 3"));
        assert!(!super::evaluate_simple_expression("3 > 5"));
        assert!(super::evaluate_simple_expression("3 <= 3"));
        assert!(super::evaluate_simple_expression("10 != 5"));
        assert!(super::evaluate_simple_expression("7 == 7"));
        assert!(!super::evaluate_simple_expression("invalid"));
        assert!(super::evaluate_simple_expression(""));
    }

    #[test]
    fn register_trigger_tools_adds_five() {
        let mut reg = ToolRegistry::new();
        register_trigger_tools(&mut reg);
        assert!(reg.get("trigger/webhook").is_some());
        assert!(reg.get("trigger/manual").is_some());
        assert!(reg.get("trigger/schedule").is_some());
        assert!(reg.get("trigger/event").is_some());
        assert!(reg.get("trigger/heartbeat").is_some());
        assert_eq!(reg.list_tools().len(), 5);
    }
}
