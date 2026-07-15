---
name: openmirai-developer
description: Skill for developing, modifying, and testing OpenMirai agent workflows, engine components, tools, and adapters.
---

# OpenMirai Development Skill

Use this skill to help you build, test, and run OpenMirai engine features, custom tools, LLM adapters, and agent graphs.

## Core Commands

* **Build the engine and CLI**: `cargo build --release`
* **Run unit and integration tests**: `cargo test --workspace`
* **Validate an agent specification**: `./target/debug/mirai validate path/to/agent.yaml`
* **Run an agent specification**: `./target/debug/mirai run path/to/agent.yaml`
* **Run the integration test suite**: `bash test/run_all.sh`
* **Start the HTTP API server**: `./target/debug/mirai serve --port 3000`

---

## Code Templates & Walkthroughs

### 1. Creating a Custom Tool

To add a new tool in the engine, follow this structure:

1. Create/edit a file in `engine/src/tools/builtin/` (e.g., `my_tool.rs`).
2. Define the tool structure and factory using the category macro. Here is a boilerplate:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// Define tool struct, factory and spec
// (Replace logic_tool! with data_tool! or ai_tool! depending on category)
logic_tool! {
    struct MyCustomTool, factory MyCustomFactory;
    tool_type = "logic/my_custom",
    name = "My Custom Tool",
    description = "A custom logic tool explanation",
    inputs = [
        field("input_val", FieldType::String, true, "Description of the input parameter"),
    ],
    outputs = [
        field("output_val", FieldType::String, true, "Description of the output parameter"),
    ],
    config_fields = [
        field("some_config", FieldType::Number, false, "Configuration variable"),
    ]
}

#[async_trait]
impl Tool for MyCustomTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let input_val = inputs
            .get("input_val")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "logic/my_custom".into(),
                message: "input_val is required".into(),
            })?;

        let some_config = config
            .get("some_config")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);

        let output_val = format!("{}-{}", input_val, some_config);

        let mut results = HashMap::new();
        results.insert("output_val".to_string(), json!(output_val));
        Ok(results)
    }
}
```

3. Register the new tool in `engine/src/tools/builtin/mod.rs`:

```rust
pub fn register_all_builtin_tools(registry: &mut ToolRegistry) {
    // ...
    registry.register("logic/my_custom", Box::new(MyCustomFactory::new()));
}
```

4. Test your tool:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::DefaultExecutionContext;
    use crate::adapters::MockLLMResource;

    #[tokio::test]
    async fn test_my_custom_tool() {
        let tool = MyCustomTool;
        let mut inputs = HashMap::new();
        inputs.insert("input_val".to_string(), json!("hello"));

        let mut config = HashMap::new();
        config.insert("some_config".to_string(), json!(42.0));

        let ctx = DefaultExecutionContext::builder(Box::new(MockLLMResource::new())).build();
        let result = tool.execute(inputs, &config, &ctx).await.unwrap();

        assert_eq!(result["output_val"], json!("hello-42"));
    }
}
```

---

## Troubleshooting Guide

* **Duplicate node IDs or edge validation fails**: Run `mirai validate <spec.yaml>` to see structural and syntactical errors before execution.
* **Database errors in tests**: Ensure SQLite databases use in-memory connection pools (`:memory:`) or write to temporary locations (`tempfile`) so they clean up after execution and avoid concurrent file locks.
* **LLM credentials error in integration tests**: By default, `run_all.sh` runs with `--provider mock`. Do not run real providers in CI unless the environment variables (e.g. `ANTHROPIC_API_KEY`) are verified and safe.
