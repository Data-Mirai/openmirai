use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use super::graph::{EdgeDef, GraphDef, NodeDef};

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

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("yaml must be a mapping at the top level")]
    InvalidYamlStructure,
}

// ---------------------------------------------------------------------------
// AgentType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Managed,
    Live,
}

impl Default for AgentType {
    fn default() -> Self {
        Self::Managed
    }
}

// ---------------------------------------------------------------------------
// Sub-models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNodeSpec {
    pub id: String,
    pub tool_type: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    #[serde(default = "default_position")]
    pub position: HashMap<String, f64>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

fn default_position() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("x".to_string(), 0.0);
    m.insert("y".to_string(), 0.0);
    m
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEdgeSpec {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_map: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentGraphSpec {
    #[serde(default)]
    pub nodes: Vec<AgentNodeSpec>,
    #[serde(default)]
    pub edges: Vec<AgentEdgeSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRetryConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_backoff")]
    pub backoff: String,
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
}

fn default_max_retries() -> u32 {
    3
}
fn default_backoff() -> String {
    "exponential".to_string()
}
fn default_on_failure() -> String {
    "stop".to_string()
}

impl Default for AgentRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: "exponential".to_string(),
            on_failure: "stop".to_string(),
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
// AgentSpec
// ---------------------------------------------------------------------------

/// Complete, self-contained agent definition.
///
/// Includes the graph inline (nodes + edges), triggers, config,
/// resource references, and metadata. Serializes to/from YAML and JSON.
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
    #[serde(default)]
    pub graph: AgentGraphSpec,
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
    /// Validate graph references (duplicate IDs, dangling edges).
    pub fn validate(&self) -> Result<(), AgentSpecError> {
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

        // Duplicate edge IDs
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

        Ok(())
    }

    /// Serialize to YAML string.
    pub fn to_yaml(&self) -> Result<String, AgentSpecError> {
        serde_yaml::to_string(self).map_err(AgentSpecError::from)
    }

    /// Parse YAML string into AgentSpec.
    pub fn from_yaml(yaml: &str) -> Result<Self, AgentSpecError> {
        let spec: Self = serde_yaml::from_str(yaml)?;
        spec.validate()?;
        Ok(spec)
    }

    /// Convert the inline graph to a GraphDef ready for execution.
    pub fn to_graph(&self, graph_id: Option<&str>) -> GraphDef {
        let gid = graph_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string());

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
                condition: None,
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

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            version: "v1".to_string(),
            agent_type: AgentType::Managed,
            system_prompt: Some("You are helpful.".to_string()),
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
            },
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
        assert_eq!(config.retry.backoff, "exponential");
        assert_eq!(config.retry.on_failure, "stop");
    }

    #[test]
    fn agent_type_serde() {
        let t = AgentType::Live;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "\"live\"");
        let back: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AgentType::Live);
    }
}
