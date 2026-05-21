use std::collections::HashMap;

use async_trait::async_trait;
use chrono;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: &str, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type: field_type.into(),
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
// Macro: simplify the boilerplate for struct + factory + spec
// ---------------------------------------------------------------------------

macro_rules! logic_tool {
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
                        category: "logic".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Box<dyn Tool> {
                Box::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ===========================================================================
// ConditionTool
// ===========================================================================

logic_tool! {
    struct ConditionTool, factory ConditionFactory;
    tool_type = "logic/condition",
    name = "Condition",
    description = "Evaluate a boolean condition on a field value",
    inputs = [
        field("field", "string", true, "Field name to evaluate"),
        field("operator", "string", true, "Comparison operator: eq, neq, gt, lt, gte, lte, in, contains"),
        field("value", "object", true, "Value to compare against"),
    ],
    outputs = [
        field("result", "boolean", true, "Evaluation result"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for ConditionTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let field_val = inputs.get("field").cloned().unwrap_or(Value::Null);
        let operator = inputs
            .get("operator")
            .and_then(|v| v.as_str())
            .unwrap_or("eq");
        let compare_val = inputs.get("value").cloned().unwrap_or(Value::Null);

        let result = evaluate_condition(&field_val, operator, &compare_val);

        let mut out = HashMap::new();
        out.insert("result".to_string(), json!(result));
        Ok(out)
    }
}

fn evaluate_condition(field_val: &Value, operator: &str, compare_val: &Value) -> bool {
    match operator {
        "eq" => field_val == compare_val,
        "neq" => field_val != compare_val,
        "gt" | "lt" | "gte" | "lte" => {
            let a = as_f64(field_val);
            let b = as_f64(compare_val);
            match (a, b) {
                (Some(a), Some(b)) => match operator {
                    "gt" => a > b,
                    "lt" => a < b,
                    "gte" => a >= b,
                    "lte" => a <= b,
                    _ => false,
                },
                _ => false,
            }
        }
        "in" => {
            if let Some(arr) = compare_val.as_array() {
                arr.contains(field_val)
            } else {
                false
            }
        }
        "contains" => {
            if let (Some(haystack), Some(needle)) = (field_val.as_str(), compare_val.as_str()) {
                haystack.contains(needle)
            } else if let Some(arr) = field_val.as_array() {
                arr.contains(compare_val)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_u64().map(|u| u as f64))
}

// ===========================================================================
// WaitTool
// ===========================================================================

logic_tool! {
    struct WaitTool, factory WaitFactory;
    tool_type = "logic/wait",
    name = "Wait",
    description = "Pause execution for a specified duration",
    inputs = [],
    outputs = [
        field("waited_seconds", "number", true, "Actual seconds waited"),
    ],
    config_fields = [
        field("delay_seconds", "number", true, "How long to wait"),
    ]
}

#[async_trait]
impl Tool for WaitTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let delay = config
            .get("delay_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        if delay > 0.0 {
            tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
        }

        let mut out = HashMap::new();
        out.insert("waited_seconds".to_string(), json!(delay));
        Ok(out)
    }
}

// ===========================================================================
// MergeTool
// ===========================================================================

logic_tool! {
    struct MergeTool, factory MergeFactory;
    tool_type = "logic/merge",
    name = "Merge",
    description = "Pass-through node that forwards its input data",
    inputs = [
        field("data", "object", false, "Data to forward"),
    ],
    outputs = [
        field("data", "object", true, "Forwarded data"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for MergeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let data = inputs.get("data").cloned().unwrap_or(Value::Null);
        let mut out = HashMap::new();
        out.insert("data".to_string(), data);
        Ok(out)
    }
}

// ===========================================================================
// LoopTool
// ===========================================================================

logic_tool! {
    struct LoopTool, factory LoopFactory;
    tool_type = "logic/loop",
    name = "Loop",
    description = "Iterate over an array of items one at a time",
    inputs = [
        field("items", "array", true, "Array of items to iterate"),
        field("current_index", "number", false, "Current iteration index (default 0)"),
    ],
    outputs = [
        field("current_item", "object", true, "Item at current index"),
        field("current_index", "number", true, "Index that was processed"),
        field("done", "boolean", true, "Whether iteration is complete"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for LoopTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let items = inputs
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let index = inputs
            .get("current_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let done = index >= items.len();
        let current_item = if done {
            Value::Null
        } else {
            items[index].clone()
        };

        let mut out = HashMap::new();
        out.insert("current_item".to_string(), current_item);
        out.insert("current_index".to_string(), json!(index));
        out.insert("done".to_string(), json!(done));
        Ok(out)
    }
}

// ===========================================================================
// SwitchTool
// ===========================================================================

logic_tool! {
    struct SwitchTool, factory SwitchFactory;
    tool_type = "logic/switch",
    name = "Switch (Router)",
    description = "Evaluates value against N cases. Routes to matched case or default",
    inputs = [
        field("value", "object", true, "Value to match against cases"),
    ],
    outputs = [
        field("matched_case", "object", true, "The matched case value or default"),
        field("case_index", "number", true, "Index of matched case (-1 if default)"),
    ],
    config_fields = [
        field("cases", "array", false, "Array of case values to match against"),
        field("default_case", "object", false, "Default value if no case matches"),
    ]
}

#[async_trait]
impl Tool for SwitchTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        let cases = config
            .get("cases")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let default_case = config.get("default_case").cloned().unwrap_or(Value::Null);

        for (i, case) in cases.iter().enumerate() {
            if value == *case {
                let mut out = HashMap::new();
                out.insert("matched_case".to_string(), case.clone());
                out.insert("case_index".to_string(), json!(i));
                return Ok(out);
            }
        }

        let mut out = HashMap::new();
        out.insert("matched_case".to_string(), default_case);
        out.insert("case_index".to_string(), json!(-1));
        Ok(out)
    }
}

// ===========================================================================
// HumanInputTool
// ===========================================================================

logic_tool! {
    struct HumanInputTool, factory HumanInputFactory;
    tool_type = "logic/human_input",
    name = "Human Input",
    description = "Pauses execution and waits for human decision. GraphRunner intercepts this tool type.",
    inputs = [
        field("prompt", "string", false, "Question or context for the user"),
        field("options", "array", false, "Predefined options if applicable"),
    ],
    outputs = [
        field("response", "object", true, "User response"),
        field("responded_by", "string", true, "Identifier of user who responded"),
        field("response_time_ms", "number", true, "Time taken to respond in ms"),
    ],
    config_fields = [
        field("prompt", "string", false, "Default prompt text"),
        field("timeout_minutes", "number", false, "Optional timeout in minutes"),
    ]
}

#[async_trait]
impl Tool for HumanInputTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // In real execution, GraphRunner intercepts this tool_type before
        // execute() is called and creates an interrupt. This execute() is
        // called when the interrupt is resolved with a response.
        let response = inputs.get("response").cloned().unwrap_or(json!("pending"));
        let responded_by = inputs
            .get("responded_by")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let response_time_ms = inputs
            .get("response_time_ms")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let mut out = HashMap::new();
        out.insert("response".to_string(), response);
        out.insert("responded_by".to_string(), json!(responded_by));
        out.insert("response_time_ms".to_string(), json!(response_time_ms));
        Ok(out)
    }
}

// ===========================================================================
// DeadlineTool
// ===========================================================================

logic_tool! {
    struct DeadlineTool, factory DeadlineFactory;
    tool_type = "logic/deadline",
    name = "Deadline",
    description = "Evaluates if a deadline timestamp has been exceeded",
    inputs = [
        field("reference_time", "string", false, "ISO timestamp from which to count"),
    ],
    outputs = [
        field("expired", "boolean", true, "True if deadline has passed"),
        field("remaining_hours", "number", true, "Hours remaining (negative if expired)"),
        field("deadline_at", "string", true, "ISO timestamp of when the deadline expires"),
    ],
    config_fields = [
        field("hours", "number", false, "Deadline duration in hours (default 48)"),
        field("static_reference_time", "string", false, "Static reference timestamp (ISO)"),
    ]
}

#[async_trait]
impl Tool for DeadlineTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let hours = config
            .get("hours")
            .and_then(|v| v.as_f64())
            .unwrap_or(48.0);

        // Determine reference time: input > config > now
        let ref_time_str = inputs
            .get("reference_time")
            .and_then(|v| v.as_str())
            .or_else(|| config.get("static_reference_time").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty());

        let now = chrono::Utc::now();

        let ref_time = match ref_time_str {
            Some(s) => {
                // Parse ISO timestamp, tolerant of common variations
                let s = s.trim().replace("Z", "+00:00");
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
                            .map(|naive| naive.and_utc())
                    })
                    .unwrap_or(now)
            }
            None => now,
        };

        let deadline_at = ref_time + chrono::Duration::milliseconds((hours * 3600.0 * 1000.0) as i64);
        let remaining_secs = (deadline_at - now).num_milliseconds() as f64 / 1000.0;
        let remaining_hours = remaining_secs / 3600.0;
        let expired = remaining_hours <= 0.0;

        let mut out = HashMap::new();
        out.insert("expired".to_string(), json!(expired));
        out.insert(
            "remaining_hours".to_string(),
            json!((remaining_hours * 100.0).round() / 100.0),
        );
        out.insert(
            "deadline_at".to_string(),
            json!(deadline_at.to_rfc3339()),
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all logic tools into the given registry.
pub fn register_logic_tools(registry: &mut ToolRegistry) {
    registry.register("logic/condition", Box::new(ConditionFactory::new()));
    registry.register("logic/wait", Box::new(WaitFactory::new()));
    registry.register("logic/merge", Box::new(MergeFactory::new()));
    registry.register("logic/loop", Box::new(LoopFactory::new()));
    registry.register("logic/switch", Box::new(SwitchFactory::new()));
    registry.register("logic/human_input", Box::new(HumanInputFactory::new()));
    registry.register("logic/deadline", Box::new(DeadlineFactory::new()));
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

    // -- Condition ---------------------------------------------------------

    #[test]
    fn condition_eq_true() {
        assert!(evaluate_condition(&json!("hello"), "eq", &json!("hello")));
    }

    #[test]
    fn condition_eq_false() {
        assert!(!evaluate_condition(&json!("a"), "eq", &json!("b")));
    }

    #[test]
    fn condition_neq() {
        assert!(evaluate_condition(&json!(1), "neq", &json!(2)));
    }

    #[test]
    fn condition_gt() {
        assert!(evaluate_condition(&json!(10), "gt", &json!(5)));
        assert!(!evaluate_condition(&json!(3), "gt", &json!(5)));
    }

    #[test]
    fn condition_lt() {
        assert!(evaluate_condition(&json!(2), "lt", &json!(5)));
    }

    #[test]
    fn condition_gte() {
        assert!(evaluate_condition(&json!(5), "gte", &json!(5)));
        assert!(evaluate_condition(&json!(6), "gte", &json!(5)));
    }

    #[test]
    fn condition_lte() {
        assert!(evaluate_condition(&json!(5), "lte", &json!(5)));
        assert!(evaluate_condition(&json!(4), "lte", &json!(5)));
    }

    #[test]
    fn condition_in() {
        assert!(evaluate_condition(
            &json!("b"),
            "in",
            &json!(["a", "b", "c"])
        ));
        assert!(!evaluate_condition(
            &json!("z"),
            "in",
            &json!(["a", "b"])
        ));
    }

    #[test]
    fn condition_contains_string() {
        assert!(evaluate_condition(&json!("hello world"), "contains", &json!("world")));
        assert!(!evaluate_condition(&json!("hello"), "contains", &json!("xyz")));
    }

    #[test]
    fn condition_contains_array() {
        assert!(evaluate_condition(&json!([1, 2, 3]), "contains", &json!(2)));
    }

    // -- Switch -----------------------------------------------------------

    #[tokio::test]
    async fn switch_match_found() {
        let tool = SwitchTool;
        let mut inputs = HashMap::new();
        inputs.insert("value".to_string(), json!("b"));
        let mut config = HashMap::new();
        config.insert("cases".to_string(), json!(["a", "b", "c"]));
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();
        assert_eq!(result["matched_case"], json!("b"));
        assert_eq!(result["case_index"], json!(1));
    }

    #[tokio::test]
    async fn switch_no_match_uses_default() {
        let tool = SwitchTool;
        let mut inputs = HashMap::new();
        inputs.insert("value".to_string(), json!("z"));
        let mut config = HashMap::new();
        config.insert("cases".to_string(), json!(["a", "b"]));
        config.insert("default_case".to_string(), json!("fallback"));
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();
        assert_eq!(result["matched_case"], json!("fallback"));
        assert_eq!(result["case_index"], json!(-1));
    }

    // -- HumanInput -------------------------------------------------------

    #[tokio::test]
    async fn human_input_returns_pending() {
        let tool = HumanInputTool;
        let inputs = HashMap::new();
        let result = tool.execute(inputs, HashMap::new(), &ctx()).await.unwrap();
        assert_eq!(result["response"], json!("pending"));
        assert_eq!(result["responded_by"], json!("unknown"));
    }

    // -- Deadline ---------------------------------------------------------

    #[tokio::test]
    async fn deadline_not_expired() {
        let tool = DeadlineTool;
        let mut inputs = HashMap::new();
        // Reference time = now, so 48h deadline is not expired
        let now = chrono::Utc::now().to_rfc3339();
        inputs.insert("reference_time".to_string(), json!(now));
        let mut config = HashMap::new();
        config.insert("hours".to_string(), json!(48));
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();
        assert_eq!(result["expired"], json!(false));
        assert!(result["remaining_hours"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn deadline_expired() {
        let tool = DeadlineTool;
        let mut inputs = HashMap::new();
        // Reference time = 100 hours ago
        let past = (chrono::Utc::now() - chrono::Duration::hours(100)).to_rfc3339();
        inputs.insert("reference_time".to_string(), json!(past));
        let mut config = HashMap::new();
        config.insert("hours".to_string(), json!(48));
        let result = tool.execute(inputs, config, &ctx()).await.unwrap();
        assert_eq!(result["expired"], json!(true));
        assert!(result["remaining_hours"].as_f64().unwrap() < 0.0);
    }

    // -- Registration -----------------------------------------------------

    #[test]
    fn register_logic_tools_adds_seven() {
        let mut reg = ToolRegistry::new();
        register_logic_tools(&mut reg);
        assert!(reg.get("logic/condition").is_some());
        assert!(reg.get("logic/wait").is_some());
        assert!(reg.get("logic/merge").is_some());
        assert!(reg.get("logic/loop").is_some());
        assert!(reg.get("logic/switch").is_some());
        assert!(reg.get("logic/human_input").is_some());
        assert!(reg.get("logic/deadline").is_some());
        assert_eq!(reg.list_tools().len(), 7);
    }
}
