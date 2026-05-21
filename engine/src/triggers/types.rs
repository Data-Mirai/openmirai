//! Trigger data models -- declarative definitions for all trigger kinds.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// TriggerType enum
// ---------------------------------------------------------------------------

/// The kind of event that activates an agent graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Webhook,
    Schedule,
    Event,
    Manual,
    AgentCall,
}

// ---------------------------------------------------------------------------
// TriggerConfig enum (typed per variant)
// ---------------------------------------------------------------------------

/// Typed configuration for each trigger kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerConfig {
    Webhook {
        path: String,
        method: String,
        #[serde(default)]
        secret: Option<String>,
    },
    Schedule {
        #[serde(default)]
        cron: Option<String>,
        #[serde(default)]
        interval_seconds: Option<u64>,
    },
    Event {
        source: String,
        event_type: String,
        #[serde(default)]
        filter: Option<HashMap<String, Value>>,
    },
    Manual {},
    AgentCall {
        caller_agent_id: String,
        #[serde(default)]
        payload_schema: Option<Value>,
    },
}

// ---------------------------------------------------------------------------
// TriggerDef
// ---------------------------------------------------------------------------

/// A complete trigger definition attached to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDef {
    /// Unique trigger identifier.
    pub id: String,
    /// Discriminant for fast matching.
    pub trigger_type: TriggerType,
    /// The agent this trigger activates.
    pub agent_id: String,
    /// Whether the trigger is currently active.
    pub enabled: bool,
    /// Kind-specific configuration.
    pub config: TriggerConfig,
}

// ---------------------------------------------------------------------------
// TriggerEvent
// ---------------------------------------------------------------------------

/// A concrete event produced when a trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    /// ID of the trigger that fired.
    pub trigger_id: String,
    /// Kind of trigger.
    pub trigger_type: TriggerType,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: f64,
    /// Event payload (body, query params, caller data, etc.).
    #[serde(default)]
    pub payload: HashMap<String, Value>,
    /// Extra context (headers, environment ID, etc.).
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_type_roundtrip() {
        let json = serde_json::to_string(&TriggerType::Webhook).unwrap();
        assert_eq!(json, r#""webhook""#);

        let parsed: TriggerType = serde_json::from_str(r#""agent_call""#).unwrap();
        assert_eq!(parsed, TriggerType::AgentCall);
    }

    #[test]
    fn trigger_config_webhook_serde() {
        let cfg = TriggerConfig::Webhook {
            path: "/hook".into(),
            method: "POST".into(),
            secret: Some("s3cret".into()),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "webhook");
        assert_eq!(json["path"], "/hook");
        assert_eq!(json["secret"], "s3cret");
    }

    #[test]
    fn trigger_config_schedule_serde() {
        let cfg = TriggerConfig::Schedule {
            cron: Some("0 * * * *".into()),
            interval_seconds: None,
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "schedule");
        assert_eq!(json["cron"], "0 * * * *");
    }

    #[test]
    fn trigger_config_event_serde() {
        let cfg = TriggerConfig::Event {
            source: "storage".into(),
            event_type: "file_uploaded".into(),
            filter: None,
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "event");
        assert_eq!(json["source"], "storage");
    }

    #[test]
    fn trigger_config_manual_serde() {
        let cfg = TriggerConfig::Manual {};
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "manual");
    }

    #[test]
    fn trigger_config_agent_call_serde() {
        let cfg = TriggerConfig::AgentCall {
            caller_agent_id: "agent-a".into(),
            payload_schema: None,
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "agent_call");
        assert_eq!(json["caller_agent_id"], "agent-a");
    }

    #[test]
    fn trigger_def_serde() {
        let def = TriggerDef {
            id: "trg-1".into(),
            trigger_type: TriggerType::Webhook,
            agent_id: "agent-x".into(),
            enabled: true,
            config: TriggerConfig::Webhook {
                path: "/api/hook".into(),
                method: "POST".into(),
                secret: None,
            },
        };

        let json = serde_json::to_string(&def).unwrap();
        let parsed: TriggerDef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "trg-1");
        assert_eq!(parsed.trigger_type, TriggerType::Webhook);
        assert!(parsed.enabled);
    }

    #[test]
    fn trigger_event_serde() {
        let evt = TriggerEvent {
            trigger_id: "trg-1".into(),
            trigger_type: TriggerType::Manual,
            timestamp: 1700000000.0,
            payload: HashMap::new(),
            metadata: HashMap::new(),
        };

        let json = serde_json::to_string(&evt).unwrap();
        let parsed: TriggerEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trigger_id, "trg-1");
        assert_eq!(parsed.trigger_type, TriggerType::Manual);
        assert!((parsed.timestamp - 1_700_000_000.0).abs() < f64::EPSILON);
    }
}
