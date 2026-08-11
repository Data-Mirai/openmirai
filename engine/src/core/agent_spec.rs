use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use super::graph::{EdgeCondition, EdgeDef, GraphDef, NodeDef};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AgentSpecError {
    #[error("duplicate node IDs: {0:?}")]
    DuplicateNodeIds(Vec<String>),

    #[error("duplicate edge IDs: {0:?}")]
    DuplicateEdgeIds(Vec<String>),

    #[error("edge '{edge_id}' references non-existent source node '{node_id}'")]
    UnknownSourceNode { edge_id: String, node_id: String },

    #[error("edge '{edge_id}' references non-existent target node '{node_id}'")]
    UnknownTargetNode { edge_id: String, node_id: String },

    #[error("graph error: {0}")]
    Graph(#[from] super::graph::GraphError),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("yaml must be a mapping at the top level")]
    InvalidYamlStructure,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),

    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
}

// ---------------------------------------------------------------------------
// AgentType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentType {
    #[default]
    Managed,
    Live,
}

// ---------------------------------------------------------------------------
// Memory Persistence Mode (PRD-008)
// ---------------------------------------------------------------------------

/// Controls how long agent memory persists between cycles/executions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MemoryPersistMode {
    /// Each cycle/execution starts with initial values. No carry-over.
    None,
    /// Live: carries between cycles within a play session. Resets on stop→play.
    /// Managed: equivalent to None (each execute is independent).
    #[default]
    Cycle,
    /// Persists across everything: cycles, stop/play, separate executions.
    /// Only resets with explicit clear_memory.
    Execution,
}

// ---------------------------------------------------------------------------
// AgentMemorySpec (PRD-008)
// ---------------------------------------------------------------------------

/// Declares persistent memory for an agent graph.
///
/// Memory keys are injected into SharedState as a virtual node `memory`
/// at the start of each execution. The `state/memory` tool persists
/// values back according to the `persist` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemorySpec {
    /// When to flush/reset memory.
    #[serde(default)]
    pub persist: MemoryPersistMode,
    /// Key-value pairs with initial values.
    pub keys: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// AgentScheduleSpec (PRD-008)
// ---------------------------------------------------------------------------

/// Schedule configuration for live agents.
///
/// Defines when and how often the agent's graph cycles.
/// Only meaningful when `agent_type = live`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentScheduleSpec {
    /// Seconds between cycles. Mutually exclusive with `cron`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    /// Cron expression (5 fields). Mutually exclusive with `interval_seconds`.
    /// Note: cron parsing not yet implemented in v1 — use interval_seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// Maximum cycles before auto-stop. None = infinite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cycles: Option<u64>,
    /// What to do when a cycle fails.
    #[serde(default)]
    pub on_cycle_error: CycleErrorMode,
}

/// Behavior when a live agent cycle fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum CycleErrorMode {
    /// Log the error and continue to the next cycle.
    #[default]
    Continue,
    /// Stop the agent (transition to Error state).
    Stop,
}

// ---------------------------------------------------------------------------
// Sub-models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNodeSpec {
    pub id: String,
    pub tool_type: String,
    #[serde(default = "super::graph::default_version")]
    pub version: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    #[serde(default = "default_position")]
    pub position: HashMap<String, f64>,
}

fn default_position() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("x".to_string(), 0.0);
    m.insert("y".to_string(), 0.0);
    m
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEdgeSpec {
    /// Edge ID — optional in YAML. Auto-generated during validate() (FEAT-034).
    #[serde(default)]
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<EdgeCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_map: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentGraphSpec {
    #[serde(default)]
    pub nodes: Vec<AgentNodeSpec>,
    #[serde(default)]
    pub edges: Vec<AgentEdgeSpec>,
    /// Persistent memory declaration (PRD-008).
    /// Keys are injected as virtual node "memory" in SharedState.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<AgentMemorySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRetryConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub backoff: super::runner::BackoffStrategy,
    #[serde(default)]
    pub on_failure: super::runner::FailureMode,
}

fn default_max_retries() -> u32 {
    3
}

impl Default for AgentRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: super::runner::BackoffStrategy::Exponential,
            on_failure: super::runner::FailureMode::Stop,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHookSpec {
    pub handler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMcpServerSpec {
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub retry: AgentRetryConfig,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<AgentHookSpec>>,
    #[serde(default)]
    pub mcp_servers: Vec<AgentMcpServerSpec>,
    #[serde(default)]
    pub vault_refs: Vec<HashMap<String, String>>,
}

fn default_max_iterations() -> u32 {
    50
}
fn default_timeout_ms() -> u64 {
    60000
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            retry: AgentRetryConfig::default(),
            timeout_ms: 60000,
            hooks: HashMap::new(),
            mcp_servers: Vec::new(),
            vault_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTriggerSpec {
    #[serde(rename = "type")]
    pub trigger_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_form: Option<Vec<HashMap<String, serde_json::Value>>>,
}

impl AgentTriggerSpec {
    /// Convert to the dict format AgentRuntime expects.
    pub fn to_runtime_map(&self) -> HashMap<String, serde_json::Value> {
        let mut d = HashMap::new();
        d.insert(
            "type".to_string(),
            serde_json::Value::String(self.trigger_type.clone()),
        );
        if let Some(ref v) = self.path {
            d.insert("path".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = self.method {
            d.insert("method".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = self.auth {
            d.insert("auth".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = self.interval_seconds {
            d.insert(
                "interval_seconds".to_string(),
                serde_json::Value::Number(serde_json::Number::from(v)),
            );
        }
        if let Some(ref v) = self.cron {
            d.insert("cron".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = self.event_type {
            d.insert(
                "event_type".to_string(),
                serde_json::Value::String(v.clone()),
            );
        }
        if let Some(ref v) = self.source {
            d.insert("source".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = self.input_form {
            d.insert(
                "input_form".to_string(),
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
            );
        }
        d
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResourceRef {
    pub resource_type: String,
    pub name: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Input/Output Contract (PRD-004)
// ---------------------------------------------------------------------------

/// User-facing type for agent input/output fields in YAML specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    Text,
    Number,
    Boolean,
    Json,
    /// A path to an existing file. A host UI should offer a file picker.
    File,
    /// A path to a directory (usually an output folder). A host UI should offer a
    /// folder picker: making the user type a path by hand is how you get typos in
    /// the one field that decides where the work lands.
    Directory,
    /// One value out of a fixed list, declared in `options`. A host UI should offer
    /// a dropdown — free text on a closed set is an error waiting to happen.
    Choice,
}

impl InputType {
    /// Check if a serde_json::Value matches this expected type.
    pub fn matches(&self, value: &serde_json::Value) -> bool {
        match self {
            InputType::Text => value.is_string(),
            InputType::Number => value.is_number(),
            InputType::Boolean => value.is_boolean(),
            InputType::Json => value.is_object() || value.is_array(),
            InputType::File | InputType::Directory | InputType::Choice => value.is_string(),
        }
    }

    /// Human-readable name for error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            InputType::Text => "text",
            InputType::Number => "number",
            InputType::Boolean => "boolean",
            InputType::Json => "json",
            InputType::File => "file",
            InputType::Directory => "directory",
            InputType::Choice => "choice",
        }
    }
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Describes a single input field in spec.inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFieldSpec {
    #[serde(rename = "type")]
    pub field_type: InputType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Allowed values when `field_type` is `choice`. Ignored otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// Describes a single output field in spec.outputs (declarative — not enforced at runtime v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputFieldSpec {
    #[serde(rename = "type")]
    pub field_type: InputType,
    #[serde(default)]
    pub description: String,
}

/// Validation error for agent inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputValidationError {
    pub field: String,
    pub error_type: String,
    pub message: String,
}

impl std::fmt::Display for InputValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Validate a client payload against the agent's input schema.
///
/// Returns Ok(enriched_payload) with defaults applied, or Err(errors) with all
/// validation failures (not fail-fast — reports all errors at once).
pub fn validate_agent_inputs(
    payload: &HashMap<String, serde_json::Value>,
    inputs_schema: &HashMap<String, InputFieldSpec>,
) -> Result<HashMap<String, serde_json::Value>, Vec<InputValidationError>> {
    let mut enriched = payload.clone();
    let mut errors = Vec::new();

    for (name, spec) in inputs_schema {
        match payload.get(name) {
            Some(value) => {
                if !spec.field_type.matches(value) {
                    let actual = crate::core::value_type::value_type_label(value);
                    errors.push(InputValidationError {
                        field: name.clone(),
                        error_type: "type_mismatch".to_string(),
                        message: format!(
                            "input '{}': expected {}, got {}",
                            name, spec.field_type, actual
                        ),
                    });
                }
            }
            None => {
                if spec.required {
                    let desc = if spec.description.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", spec.description)
                    };
                    errors.push(InputValidationError {
                        field: name.clone(),
                        error_type: "missing_required".to_string(),
                        message: format!("missing required input: {}{}", name, desc),
                    });
                } else if let Some(ref default) = spec.default {
                    enriched.insert(name.clone(), default.clone());
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
// AgentSpec
// ---------------------------------------------------------------------------

/// Complete, self-contained agent definition.
///
/// Includes the graph inline (nodes + edges), triggers, config,
/// resource references, and metadata. YAML is the canonical format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_spec_version")]
    pub version: String,
    #[serde(default)]
    pub agent_type: AgentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Path to a SOUL.md file that defines agent personality.
    /// If set, the Soul's system prompt is used (overrides system_prompt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul: Option<String>,
    /// Input schema — declares what data the client must/can provide.
    /// If None, no validation is performed (backward compat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<HashMap<String, InputFieldSpec>>,
    /// Output schema — declares what the agent produces (declarative, not enforced in v1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<HashMap<String, OutputFieldSpec>>,
    #[serde(default)]
    pub graph: AgentGraphSpec,
    /// Schedule for live agents (PRD-008). Only valid when agent_type = live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<AgentScheduleSpec>,
    #[serde(default)]
    pub triggers: Vec<AgentTriggerSpec>,
    #[serde(default)]
    pub config: AgentConfig,
    #[serde(default)]
    pub resources: Vec<AgentResourceRef>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_spec_version() -> String {
    "v1".to_string()
}

impl AgentSpec {
    /// Whether this agent needs an LLM provider at all.
    ///
    /// A graph made of `system/bash`, `filesystem/*` and `logic/*` is a deterministic
    /// program: same inputs, same steps, same output. Requiring a model to run it makes
    /// the whole pipeline hostage to a quota that has nothing to do with the work.
    /// Only these tool families actually talk to a model.
    pub fn needs_llm(&self) -> bool {
        const IA: [&str; 6] = [
            "ai/llm_call",
            "ai/transcribe",
            "ai/embeddings",
            "ai/image_edit",
            "ai/tts",
            "ai/claude_code",
        ];
        // `agent/run_agent` may call a sub-agent that uses AI; treat it as needing one.
        self.graph
            .nodes
            .iter()
            .any(|n| IA.contains(&n.tool_type.as_str()) || n.tool_type == "agent/run_agent")
            || self.system_prompt.is_some()
            || self.soul.is_some()
    }

    /// Auto-generate IDs for edges with empty `id` (FEAT-034 / API-02).
    pub fn auto_generate_edge_ids(&mut self) {
        let mut pair_counts: HashMap<String, usize> = HashMap::new();
        for edge in &mut self.graph.edges {
            if edge.id.is_empty() {
                let pair_key = format!("{}__{}", edge.source, edge.target);
                let count = pair_counts.entry(pair_key.clone()).or_insert(0);
                *count += 1;
                edge.id = if *count == 1 {
                    pair_key
                } else {
                    format!("{pair_key}__{count}")
                };
            }
        }
    }

    /// Validate graph references, self-loops, cycles, and empty graphs.
    pub fn validate(&mut self) -> Result<(), AgentSpecError> {
        // Auto-generate edge IDs before validation
        self.auto_generate_edge_ids();

        // Duplicate node IDs
        let mut seen_nodes = std::collections::HashSet::new();
        let mut dupe_nodes = Vec::new();
        for n in &self.graph.nodes {
            if !seen_nodes.insert(&n.id) {
                dupe_nodes.push(n.id.clone());
            }
        }
        if !dupe_nodes.is_empty() {
            return Err(AgentSpecError::DuplicateNodeIds(dupe_nodes));
        }

        // Duplicate edge IDs (after auto-gen)
        let mut seen_edges = std::collections::HashSet::new();
        let mut dupe_edges = Vec::new();
        for e in &self.graph.edges {
            if !seen_edges.insert(&e.id) {
                dupe_edges.push(e.id.clone());
            }
        }
        if !dupe_edges.is_empty() {
            return Err(AgentSpecError::DuplicateEdgeIds(dupe_edges));
        }

        // Edge references
        for edge in &self.graph.edges {
            if !seen_nodes.contains(&edge.source) {
                return Err(AgentSpecError::UnknownSourceNode {
                    edge_id: edge.id.clone(),
                    node_id: edge.source.clone(),
                });
            }
            if !seen_nodes.contains(&edge.target) {
                return Err(AgentSpecError::UnknownTargetNode {
                    edge_id: edge.id.clone(),
                    node_id: edge.target.clone(),
                });
            }
        }

        // PRD-008: schedule ↔ live validation
        match (&self.agent_type, &self.schedule) {
            (AgentType::Live, None) => {
                return Err(AgentSpecError::InvalidSchedule(
                    "live agents require a schedule section".to_string(),
                ));
            }
            (AgentType::Managed, Some(_)) => {
                return Err(AgentSpecError::InvalidSchedule(
                    "schedule is only valid for live agents".to_string(),
                ));
            }
            (AgentType::Live, Some(sched)) => {
                // Exactly one of interval_seconds or cron must be set
                match (&sched.interval_seconds, &sched.cron) {
                    (None, None) | (Some(_), Some(_)) => {
                        return Err(AgentSpecError::InvalidSchedule(
                            "schedule must have exactly one of: interval_seconds, cron".to_string(),
                        ));
                    }
                    (Some(0), None) => {
                        return Err(AgentSpecError::InvalidSchedule(
                            "interval_seconds must be >= 1".to_string(),
                        ));
                    }
                    (None, Some(_)) => {
                        return Err(AgentSpecError::InvalidSchedule(
                            "cron support coming soon, use interval_seconds".to_string(),
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Delegate to GraphDef::validate() for self-loops, empty graph.
        // Edge IDs are already generated above — to_graph() copies them, so
        // no need to call graph.auto_generate_edge_ids() again.
        let graph = self.to_graph(None);
        graph.validate().map_err(AgentSpecError::Graph)?;

        Ok(())
    }

    /// Serialize to YAML string.
    pub fn to_yaml(&self) -> Result<String, AgentSpecError> {
        serde_yaml::to_string(self).map_err(AgentSpecError::from)
    }

    /// Parse YAML string into AgentSpec.
    pub fn from_yaml(yaml: &str) -> Result<Self, AgentSpecError> {
        let mut spec: Self = serde_yaml::from_str(yaml)?;
        spec.validate()?;
        Ok(spec)
    }

    /// Load an agent spec from a YAML file.
    ///
    /// Only `.yaml` and `.yml` extensions are accepted.  Agent specs are
    /// YAML-only for readability — JSON is not supported for this format.
    pub fn from_file(path: &str) -> Result<Self, AgentSpecError> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "yaml" | "yml") {
            return Err(AgentSpecError::UnsupportedExtension(format!(
                ".{ext} — agent specs must be YAML (.yaml or .yml)"
            )));
        }
        let content = std::fs::read_to_string(path)?;
        Self::from_yaml(&content)
    }

    /// Save to file — always produces YAML.
    pub fn to_file(&self, path: &str) -> Result<(), AgentSpecError> {
        let content = self.to_yaml()?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Convert the inline graph to a GraphDef ready for execution.
    pub fn to_graph(&self, graph_id: Option<&str>) -> GraphDef {
        let gid = graph_id
            .map(|s| s.to_string())
            .unwrap_or_else(crate::utils::short_id);

        let nodes = self
            .graph
            .nodes
            .iter()
            .map(|n| {
                let position = n
                    .position
                    .get("x")
                    .and_then(|x| n.position.get("y").map(|y| (*x, *y)));

                NodeDef {
                    id: n.id.clone(),
                    tool_type: n.tool_type.clone(),
                    version: n.version.clone(),
                    config: n.config.clone(),
                    position,
                }
            })
            .collect();

        let edges = self
            .graph
            .edges
            .iter()
            .map(|e| EdgeDef {
                id: e.id.clone(),
                source: e.source.clone(),
                target: e.target.clone(),
                condition: e.condition.clone(),
                data_map: e.data_map.clone(),
            })
            .collect();

        GraphDef {
            id: gid,
            name: self.name.clone(),
            version: self.version.clone(),
            nodes,
            edges,
            metadata: self.metadata.clone(),
        }
    }

    /// Convert triggers to the list of maps that AgentRuntime expects.
    pub fn to_triggers_list(&self) -> Vec<HashMap<String, serde_json::Value>> {
        self.triggers.iter().map(|t| t.to_runtime_map()).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::graph::{ComparisonOp, EdgeCondition};
    use serde_json::json;

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            version: "v1".to_string(),
            agent_type: AgentType::Managed,
            system_prompt: Some("You are helpful.".to_string()),
            soul: None,
            inputs: None,
            outputs: None,
            graph: AgentGraphSpec {
                nodes: vec![
                    AgentNodeSpec {
                        id: "n1".to_string(),
                        tool_type: "ai/llm_call".to_string(),
                        version: "1.0.0".to_string(),
                        config: HashMap::new(),
                        position: default_position(),
                    },
                    AgentNodeSpec {
                        id: "n2".to_string(),
                        tool_type: "logic/condition".to_string(),
                        version: "1.0.0".to_string(),
                        config: HashMap::new(),
                        position: default_position(),
                    },
                ],
                edges: vec![AgentEdgeSpec {
                    id: "e1".to_string(),
                    source: "n1".to_string(),
                    target: "n2".to_string(),
                    condition: None,
                    data_map: None,
                }],
                memory: None,
            },
            schedule: None,
            triggers: vec![AgentTriggerSpec {
                trigger_type: "webhook".to_string(),
                path: Some("/api/hook".to_string()),
                method: Some("POST".to_string()),
                auth: None,
                interval_seconds: None,
                cron: None,
                event_type: None,
                source: None,
                input_form: None,
            }],
            config: AgentConfig::default(),
            resources: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn validate_ok() {
        assert!(sample_spec().validate().is_ok());
    }

    #[test]
    fn validate_duplicate_nodes() {
        let mut spec = sample_spec();
        spec.graph.nodes.push(AgentNodeSpec {
            id: "n1".to_string(),
            tool_type: "ai/llm_call".to_string(),
            version: "1.0.0".to_string(),
            config: HashMap::new(),
            position: default_position(),
        });
        assert!(matches!(
            spec.validate(),
            Err(AgentSpecError::DuplicateNodeIds(_))
        ));
    }

    #[test]
    fn validate_duplicate_edges() {
        let mut spec = sample_spec();
        spec.graph.edges.push(AgentEdgeSpec {
            id: "e1".to_string(),
            source: "n1".to_string(),
            target: "n2".to_string(),
            condition: None,
            data_map: None,
        });
        assert!(matches!(
            spec.validate(),
            Err(AgentSpecError::DuplicateEdgeIds(_))
        ));
    }

    #[test]
    fn validate_unknown_source() {
        let mut spec = sample_spec();
        spec.graph.edges.push(AgentEdgeSpec {
            id: "e2".to_string(),
            source: "ghost".to_string(),
            target: "n2".to_string(),
            condition: None,
            data_map: None,
        });
        assert!(matches!(
            spec.validate(),
            Err(AgentSpecError::UnknownSourceNode { .. })
        ));
    }

    #[test]
    fn validate_unknown_target() {
        let mut spec = sample_spec();
        spec.graph.edges.push(AgentEdgeSpec {
            id: "e2".to_string(),
            source: "n1".to_string(),
            target: "ghost".to_string(),
            condition: None,
            data_map: None,
        });
        assert!(matches!(
            spec.validate(),
            Err(AgentSpecError::UnknownTargetNode { .. })
        ));
    }

    #[test]
    fn auto_gen_edge_id() {
        let mut spec = sample_spec();
        spec.graph.edges[0].id = String::new(); // clear ID
        spec.validate().unwrap();
        assert_eq!(spec.graph.edges[0].id, "n1__n2");
    }

    #[test]
    fn auto_gen_edge_id_collision() {
        let mut spec = sample_spec();
        spec.graph.edges[0].id = String::new();
        spec.graph.edges.push(AgentEdgeSpec {
            id: String::new(),
            source: "n1".to_string(),
            target: "n2".to_string(),
            condition: Some(EdgeCondition {
                field: "x".to_string(),
                op: ComparisonOp::Eq,
                value: serde_json::json!(true),
            }),
            data_map: None,
        });
        spec.validate().unwrap();
        let ids: Vec<&str> = spec.graph.edges.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"n1__n2"));
        assert!(ids.contains(&"n1__n2__2"));
    }

    #[test]
    fn from_file_rejects_json_extension() {
        // Agent specs are YAML-only.
        let result = AgentSpec::from_file("agent.json");
        assert!(matches!(
            result,
            Err(AgentSpecError::UnsupportedExtension(_))
        ));
    }

    #[test]
    fn yaml_roundtrip() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        let back = AgentSpec::from_yaml(&yaml).unwrap();
        assert_eq!(back.name, "test-agent");
        assert_eq!(back.graph.nodes.len(), 2);
        assert_eq!(back.graph.edges.len(), 1);
        assert_eq!(back.triggers.len(), 1);
    }

    #[test]
    fn to_graph_conversion() {
        let spec = sample_spec();
        let graph = spec.to_graph(Some("g-test"));
        assert_eq!(graph.id, "g-test");
        assert_eq!(graph.name, "test-agent");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.nodes[0].tool_type, "ai/llm_call");
    }

    #[test]
    fn to_triggers_list_conversion() {
        let spec = sample_spec();
        let triggers = spec.to_triggers_list();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["type"], "webhook");
        assert_eq!(triggers[0]["path"], "/api/hook");
        assert_eq!(triggers[0]["method"], "POST");
    }

    #[test]
    fn trigger_runtime_map_skips_none() {
        let trigger = AgentTriggerSpec {
            trigger_type: "schedule".to_string(),
            path: None,
            method: None,
            auth: None,
            interval_seconds: Some(300),
            cron: None,
            event_type: None,
            source: None,
            input_form: None,
        };
        let map = trigger.to_runtime_map();
        assert_eq!(map.len(), 2); // type + interval_seconds
        assert_eq!(map["type"], "schedule");
        assert_eq!(map["interval_seconds"], 300);
    }

    #[test]
    fn default_config_values() {
        let config = AgentConfig::default();
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.timeout_ms, 60000);
        assert_eq!(config.retry.max_retries, 3);
        assert_eq!(
            config.retry.backoff,
            crate::core::runner::BackoffStrategy::Exponential
        );
        assert_eq!(
            config.retry.on_failure,
            crate::core::runner::FailureMode::Stop
        );
    }

    #[test]
    fn agent_type_serde() {
        let t = AgentType::Live;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "\"live\"");
        let back: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AgentType::Live);
    }

    #[test]
    fn validate_detects_self_loop() {
        let mut spec = sample_spec();
        spec.graph.edges.push(AgentEdgeSpec {
            id: "self-loop".to_string(),
            source: "n1".to_string(),
            target: "n1".to_string(),
            condition: None,
            data_map: None,
        });
        let err = spec.validate().unwrap_err();
        assert!(
            matches!(err, AgentSpecError::Graph(_)),
            "expected Graph error for self-loop, got: {err:?}"
        );
    }

    #[test]
    fn validate_detects_empty_graph() {
        let mut spec = AgentSpec {
            name: "empty".to_string(),
            description: String::new(),
            version: "v1".to_string(),
            agent_type: AgentType::Managed,
            system_prompt: None,
            soul: None,
            inputs: None,
            outputs: None,
            graph: AgentGraphSpec::default(),
            schedule: None,
            triggers: vec![],
            config: AgentConfig::default(),
            resources: Vec::new(),
            metadata: HashMap::new(),
        };
        let err = spec.validate().unwrap_err();
        assert!(
            matches!(err, AgentSpecError::Graph(_)),
            "expected Graph error for empty graph, got: {err:?}"
        );
    }

    // --- PRD-004: Input/Output Contract tests ---

    #[test]
    fn yaml_with_inputs_outputs_roundtrip() {
        let yaml = r#"
name: qa-assistant
version: v1
description: "Responde preguntas"
inputs:
  question:
    type: text
    required: true
    description: "Pregunta del usuario"
  context:
    type: text
    required: false
    description: "Contexto adicional"
    default: "sin contexto"
outputs:
  answer:
    type: text
    description: "Respuesta generada"
  confidence:
    type: number
    description: "Nivel de confianza"
graph:
  nodes:
    - id: n1
      tool_type: trigger/manual
    - id: n2
      tool_type: ai/llm_call
  edges:
    - source: n1
      target: n2
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        assert_eq!(spec.name, "qa-assistant");

        // Inputs
        let inputs = spec.inputs.as_ref().unwrap();
        assert_eq!(inputs.len(), 2);
        let q = &inputs["question"];
        assert_eq!(q.field_type, InputType::Text);
        assert!(q.required);
        assert_eq!(q.description, "Pregunta del usuario");
        let ctx = &inputs["context"];
        assert!(!ctx.required);
        assert_eq!(ctx.default, Some(serde_json::json!("sin contexto")));

        // Outputs
        let outputs = spec.outputs.as_ref().unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs["answer"].field_type, InputType::Text);
        assert_eq!(outputs["confidence"].field_type, InputType::Number);

        // Round-trip
        let yaml2 = spec.to_yaml().unwrap();
        let spec2 = AgentSpec::from_yaml(&yaml2).unwrap();
        assert_eq!(spec2.inputs.as_ref().unwrap().len(), 2);
        assert_eq!(spec2.outputs.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn yaml_without_inputs_outputs_backward_compat() {
        let yaml = r#"
name: old-agent
graph:
  nodes:
    - id: n1
      tool_type: ai/llm_call
    - id: n2
      tool_type: logic/condition
  edges:
    - source: n1
      target: n2
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        assert!(spec.inputs.is_none());
        assert!(spec.outputs.is_none());
    }

    #[test]
    fn validate_inputs_happy_path() {
        let mut schema = HashMap::new();
        schema.insert(
            "question".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: true,
                description: "Pregunta".to_string(),
                default: None,
                options: vec![],
            },
        );
        schema.insert(
            "context".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: false,
                description: "Contexto".to_string(),
                default: Some(serde_json::json!("default ctx")),
                options: vec![],
            },
        );

        let mut payload = HashMap::new();
        payload.insert("question".to_string(), serde_json::json!("hola"));

        let result = validate_agent_inputs(&payload, &schema).unwrap();
        assert_eq!(result["question"], serde_json::json!("hola"));
        assert_eq!(result["context"], serde_json::json!("default ctx")); // default applied
    }

    #[test]
    fn validate_inputs_missing_required() {
        let mut schema = HashMap::new();
        schema.insert(
            "question".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: true,
                description: "Pregunta del usuario".to_string(),
                default: None,
                options: vec![],
            },
        );

        let payload = HashMap::new(); // empty
        let errors = validate_agent_inputs(&payload, &schema).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error_type, "missing_required");
        assert!(errors[0].message.contains("question"));
        assert!(errors[0].message.contains("Pregunta del usuario"));
    }

    #[test]
    fn validate_inputs_type_mismatch() {
        let mut schema = HashMap::new();
        schema.insert(
            "question".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: true,
                description: String::new(),
                default: None,
                options: vec![],
            },
        );

        let mut payload = HashMap::new();
        payload.insert("question".to_string(), serde_json::json!(42)); // number, not text

        let errors = validate_agent_inputs(&payload, &schema).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error_type, "type_mismatch");
        assert!(errors[0].message.contains("expected text"));
        assert!(errors[0].message.contains("got number"));
    }

    #[test]
    fn validate_inputs_extra_fields_allowed() {
        let mut schema = HashMap::new();
        schema.insert(
            "question".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: true,
                description: String::new(),
                default: None,
                options: vec![],
            },
        );

        let mut payload = HashMap::new();
        payload.insert("question".to_string(), serde_json::json!("hola"));
        payload.insert("extra_field".to_string(), serde_json::json!(123));

        let result = validate_agent_inputs(&payload, &schema).unwrap();
        assert_eq!(result.len(), 2); // both fields preserved
        assert_eq!(result["extra_field"], serde_json::json!(123));
    }

    #[test]
    fn validate_inputs_multiple_errors() {
        let mut schema = HashMap::new();
        schema.insert(
            "question".to_string(),
            InputFieldSpec {
                field_type: InputType::Text,
                required: true,
                description: String::new(),
                default: None,
                options: vec![],
            },
        );
        schema.insert(
            "count".to_string(),
            InputFieldSpec {
                field_type: InputType::Number,
                required: true,
                description: String::new(),
                default: None,
                options: vec![],
            },
        );

        let payload = HashMap::new(); // empty — both required missing
        let errors = validate_agent_inputs(&payload, &schema).unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn input_type_matches_all_variants() {
        use serde_json::json;
        assert!(InputType::Text.matches(&json!("hello")));
        assert!(!InputType::Text.matches(&json!(42)));
        assert!(InputType::Number.matches(&json!(1.5)));
        assert!(!InputType::Number.matches(&json!("three")));
        assert!(InputType::Boolean.matches(&json!(true)));
        assert!(!InputType::Boolean.matches(&json!(1)));
        assert!(InputType::Json.matches(&json!({"key": "val"})));
        assert!(InputType::Json.matches(&json!([1, 2, 3])));
        assert!(!InputType::Json.matches(&json!("string")));
        assert!(InputType::File.matches(&json!("/path/to/file.txt")));
        assert!(!InputType::File.matches(&json!(123)));
    }

    // --- Edge condition preservation through to_graph() ---

    #[test]
    fn to_graph_preserves_edge_conditions() {
        let yaml = r#"
name: cond-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
    - id: c
      tool_type: output/response
  edges:
    - source: a
      target: b
      condition:
        field: priority
        op: Eq
        value: high
    - source: a
      target: c
      condition:
        field: priority
        op: Eq
        value: low
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();

        // Verify spec parsed conditions
        assert!(spec.graph.edges[0].condition.is_some());
        assert!(spec.graph.edges[1].condition.is_some());

        // Verify to_graph() preserves them
        let graph = spec.to_graph(Some("test"));
        let cond0 = graph.edges[0]
            .condition
            .as_ref()
            .expect("edge 0 must have condition");
        assert_eq!(cond0.field, "priority");
        assert_eq!(cond0.op, ComparisonOp::Eq);
        assert_eq!(cond0.value, json!("high"));

        let cond1 = graph.edges[1]
            .condition
            .as_ref()
            .expect("edge 1 must have condition");
        assert_eq!(cond1.value, json!("low"));
    }

    #[test]
    fn to_graph_preserves_neq_condition() {
        let yaml = r#"
name: neq-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
      condition:
        field: status
        op: Neq
        value: "done"
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        let cond = graph.edges[0].condition.as_ref().unwrap();
        assert_eq!(cond.op, ComparisonOp::Neq);
    }

    #[test]
    fn to_graph_preserves_numeric_conditions() {
        let yaml = r#"
name: numeric-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: above
      tool_type: output/response
    - id: below
      tool_type: output/response
  edges:
    - source: a
      target: above
      condition:
        field: score
        op: Gt
        value: 80
    - source: a
      target: below
      condition:
        field: score
        op: Lte
        value: 80
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        let cond0 = graph.edges[0].condition.as_ref().unwrap();
        assert_eq!(cond0.op, ComparisonOp::Gt);
        assert_eq!(cond0.value, json!(80));
        let cond1 = graph.edges[1].condition.as_ref().unwrap();
        assert_eq!(cond1.op, ComparisonOp::Lte);
    }

    #[test]
    fn to_graph_preserves_contains_condition() {
        let yaml = r#"
name: contains-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
      condition:
        field: text
        op: Contains
        value: "error"
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        let cond = graph.edges[0].condition.as_ref().unwrap();
        assert_eq!(cond.op, ComparisonOp::Contains);
        assert_eq!(cond.value, json!("error"));
    }

    #[test]
    fn to_graph_preserves_in_condition() {
        let yaml = r#"
name: in-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
      condition:
        field: status
        op: In
        value: ["active", "pending"]
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        let cond = graph.edges[0].condition.as_ref().unwrap();
        assert_eq!(cond.op, ComparisonOp::In);
        assert_eq!(cond.value, json!(["active", "pending"]));
    }

    #[test]
    fn to_graph_unconditional_edges_remain_none() {
        let yaml = r#"
name: fanout-test
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
    - id: c
      tool_type: output/response
  edges:
    - source: a
      target: b
    - source: a
      target: c
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        assert!(graph.edges[0].condition.is_none());
        assert!(graph.edges[1].condition.is_none());
    }

    #[test]
    fn to_graph_mixed_conditional_and_unconditional() {
        let yaml = r#"
name: mix-test
graph:
  nodes:
    - id: trigger
      tool_type: trigger/manual
    - id: always_run
      tool_type: output/response
    - id: maybe_run
      tool_type: output/response
  edges:
    - source: trigger
      target: always_run
    - source: trigger
      target: maybe_run
      condition:
        field: x
        op: Eq
        value: 1
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let graph = spec.to_graph(None);
        assert!(
            graph.edges[0].condition.is_none(),
            "unconditional edge must stay None"
        );
        assert!(
            graph.edges[1].condition.is_some(),
            "conditional edge must be preserved"
        );
        let cond = graph.edges[1].condition.as_ref().unwrap();
        assert_eq!(cond.field, "x");
        assert_eq!(cond.op, ComparisonOp::Eq);
        assert_eq!(cond.value, json!(1));
    }

    #[test]
    fn comparison_op_lowercase_yaml() {
        // YAML users should be able to write "eq" instead of "Eq"
        let yaml = r#"
name: lowercase-op
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
      condition:
        field: x
        op: eq
        value: 1
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let cond = spec.graph.edges[0].condition.as_ref().unwrap();
        assert_eq!(cond.op, ComparisonOp::Eq);
    }

    // --- PRD-008: Schedule + Memory tests ---

    #[test]
    fn live_agent_requires_schedule() {
        let yaml = r#"
name: test
agent_type: live
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let err = AgentSpec::from_yaml(yaml).unwrap_err();
        assert!(err
            .to_string()
            .contains("live agents require a schedule section"));
    }

    #[test]
    fn managed_agent_rejects_schedule() {
        let yaml = r#"
name: test
agent_type: managed
schedule:
  interval_seconds: 60
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let err = AgentSpec::from_yaml(yaml).unwrap_err();
        assert!(err
            .to_string()
            .contains("schedule is only valid for live agents"));
    }

    #[test]
    fn schedule_requires_exactly_one_timing() {
        // Both set → error
        let yaml = r#"
name: test
agent_type: live
schedule:
  interval_seconds: 60
  cron: "* * * * *"
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let err = AgentSpec::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one of"));

        // Neither set → error
        let yaml2 = r#"
name: test
agent_type: live
schedule: {}
graph:
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let err2 = AgentSpec::from_yaml(yaml2).unwrap_err();
        assert!(err2.to_string().contains("exactly one of"));
    }

    #[test]
    fn live_agent_with_valid_schedule_parses() {
        let yaml = r#"
name: monitor
agent_type: live
schedule:
  interval_seconds: 300
  max_cycles: 10
  on_cycle_error: stop
graph:
  nodes:
    - id: a
      tool_type: trigger/schedule
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        assert_eq!(spec.agent_type, AgentType::Live);
        let sched = spec.schedule.as_ref().unwrap();
        assert_eq!(sched.interval_seconds, Some(300));
        assert_eq!(sched.max_cycles, Some(10));
        assert_eq!(sched.on_cycle_error, CycleErrorMode::Stop);
    }

    #[test]
    fn graph_memory_parses() {
        let yaml = r#"
name: bot
graph:
  memory:
    persist: execution
    keys:
      history: []
      count: 0
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let mem = spec.graph.memory.as_ref().unwrap();
        assert_eq!(mem.persist, MemoryPersistMode::Execution);
        assert_eq!(mem.keys.len(), 2);
        assert_eq!(mem.keys["count"], json!(0));
        assert_eq!(mem.keys["history"], json!([]));
    }

    #[test]
    fn graph_memory_default_persist_is_cycle() {
        let yaml = r#"
name: bot
graph:
  memory:
    keys:
      data: null
  nodes:
    - id: a
      tool_type: trigger/manual
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let mem = spec.graph.memory.as_ref().unwrap();
        assert_eq!(mem.persist, MemoryPersistMode::Cycle);
    }

    #[test]
    fn live_agent_yaml_roundtrip() {
        let yaml = r#"
name: collector
agent_type: live
schedule:
  interval_seconds: 60
graph:
  memory:
    persist: execution
    keys:
      total: 0
  nodes:
    - id: a
      tool_type: trigger/schedule
    - id: b
      tool_type: output/response
  edges:
    - source: a
      target: b
"#;
        let spec = AgentSpec::from_yaml(yaml).unwrap();
        let yaml_out = spec.to_yaml().unwrap();
        let spec2 = AgentSpec::from_yaml(&yaml_out).unwrap();
        assert_eq!(spec2.agent_type, AgentType::Live);
        assert_eq!(spec2.schedule.as_ref().unwrap().interval_seconds, Some(60));
        let mem2 = spec2.graph.memory.as_ref().unwrap();
        assert_eq!(mem2.persist, MemoryPersistMode::Execution);
        assert_eq!(mem2.keys["total"], json!(0));
    }

    #[test]
    fn memory_persist_mode_serde() {
        let m = MemoryPersistMode::Execution;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"execution\"");
        let back: MemoryPersistMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MemoryPersistMode::Execution);
    }

    #[test]
    fn cycle_error_mode_serde() {
        let m = CycleErrorMode::Stop;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"stop\"");
        let back: CycleErrorMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CycleErrorMode::Stop);
    }
}
