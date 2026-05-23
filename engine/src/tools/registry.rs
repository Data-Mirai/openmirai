use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::core::context::ExecutionContext;
use crate::core::graph::NodeDef;
use crate::core::runner::{ToolError, ToolExecutor};
use crate::tools::base::ToolSpec;

// ---------------------------------------------------------------------------
// Tool trait
// ---------------------------------------------------------------------------

/// A concrete, executable tool instance.
///
/// Created by a [`ToolFactory`] and invoked by the graph runner for each node.
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}

// ---------------------------------------------------------------------------
// ToolFactory trait
// ---------------------------------------------------------------------------

/// Factory for creating [`Tool`] instances.
///
/// Each registered tool type has exactly one factory. The factory owns the
/// [`ToolSpec`] and can stamp out fresh `Tool` instances on demand.
pub trait ToolFactory: Send + Sync {
    fn create(&self) -> Arc<dyn Tool>;
    fn spec(&self) -> &ToolSpec;
}

// ---------------------------------------------------------------------------
// ToolRegistry
// ---------------------------------------------------------------------------

/// Central registry mapping `tool_type` strings to their factories.
///
/// Supports aliases for backward compatibility: legacy tool_type strings
/// (e.g. `fs/read_file`) can map to the canonical name (e.g. `filesystem/read_file`).
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolFactory>>,
    /// Maps legacy tool_type → canonical tool_type.
    aliases: HashMap<String, String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Register a factory under the given `tool_type` key.
    pub fn register(&mut self, tool_type: &str, factory: Box<dyn ToolFactory>) {
        self.tools.insert(tool_type.to_string(), factory);
    }

    /// Register a legacy alias that resolves to a canonical tool_type.
    pub fn register_alias(&mut self, alias: &str, canonical: &str) {
        self.aliases.insert(alias.to_string(), canonical.to_string());
    }

    /// Look up a factory by `tool_type`. Falls back to aliases if the direct
    /// lookup misses.
    pub fn get(&self, tool_type: &str) -> Option<&dyn ToolFactory> {
        self.tools
            .get(tool_type)
            .or_else(|| {
                self.aliases
                    .get(tool_type)
                    .and_then(|canonical| self.tools.get(canonical))
            })
            .map(|b| b.as_ref())
    }

    /// List the specs of every registered tool (aliases are not listed).
    pub fn list_tools(&self) -> Vec<&ToolSpec> {
        self.tools.values().map(|f| f.spec()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// RegistryExecutor
// ---------------------------------------------------------------------------

/// [`ToolExecutor`] implementation backed by a [`ToolRegistry`].
///
/// Looks up the node's `tool_type` in the registry, creates a fresh tool
/// instance, and delegates execution.
pub struct RegistryExecutor {
    registry: Arc<ToolRegistry>,
}

impl RegistryExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolExecutor for RegistryExecutor {
    async fn execute(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let factory = self
            .registry
            .get(&node.tool_type)
            .ok_or_else(|| ToolError::NotFound {
                tool_type: node.tool_type.clone(),
            })?;

        let tool = factory.create();
        tool.execute(inputs, &node.config, context).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::base::ToolField;

    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        async fn execute(
            &self,
            _inputs: HashMap<String, Value>,
            _config: &HashMap<String, Value>,
            _context: &dyn ExecutionContext,
        ) -> Result<HashMap<String, Value>, ToolError> {
            let mut out = HashMap::new();
            out.insert("ok".to_string(), Value::Bool(true));
            Ok(out)
        }
    }

    struct DummyFactory {
        spec: ToolSpec,
    }

    impl ToolFactory for DummyFactory {
        fn create(&self) -> Arc<dyn Tool> {
            Arc::new(DummyTool)
        }
        fn spec(&self) -> &ToolSpec {
            &self.spec
        }
    }

    fn make_spec(tool_type: &str) -> ToolSpec {
        ToolSpec {
            tool_type: tool_type.into(),
            name: tool_type.into(),
            description: "test".into(),
            version: "1.0.0".into(),
            category: "test".into(),
            inputs: vec![],
            outputs: vec![ToolField {
                name: "ok".into(),
                field_type: crate::tools::base::FieldType::Boolean,
                required: true,
                description: None,
                default: None,
            }],
            config_fields: vec![],
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ToolRegistry::new();
        reg.register(
            "test/dummy",
            Box::new(DummyFactory {
                spec: make_spec("test/dummy"),
            }),
        );
        assert!(reg.get("test/dummy").is_some());
        assert!(reg.get("test/missing").is_none());
    }

    #[test]
    fn list_tools_returns_all_specs() {
        let mut reg = ToolRegistry::new();
        reg.register(
            "a/one",
            Box::new(DummyFactory {
                spec: make_spec("a/one"),
            }),
        );
        reg.register(
            "b/two",
            Box::new(DummyFactory {
                spec: make_spec("b/two"),
            }),
        );
        let specs = reg.list_tools();
        assert_eq!(specs.len(), 2);
    }
}
