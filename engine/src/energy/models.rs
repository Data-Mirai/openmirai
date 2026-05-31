//! Energy data models -- atomic metering events and rate configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Type of energy-consuming operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnergyType {
    LlmCall,
    McpCall,
    ToolExec,
    ComputeTime,
    StorageOp,
    DbOp,
}

impl std::fmt::Display for EnergyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LlmCall => write!(f, "LLM_CALL"),
            Self::McpCall => write!(f, "MCP_CALL"),
            Self::ToolExec => write!(f, "TOOL_EXEC"),
            Self::ComputeTime => write!(f, "COMPUTE_TIME"),
            Self::StorageOp => write!(f, "STORAGE_OP"),
            Self::DbOp => write!(f, "DB_OP"),
        }
    }
}

/// Broad cost classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CostCategory {
    InternalInfra,
    ExternalService,
    PlatformFee,
}

impl std::fmt::Display for CostCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InternalInfra => write!(f, "INTERNAL_INFRA"),
            Self::ExternalService => write!(f, "EXTERNAL_SERVICE"),
            Self::PlatformFee => write!(f, "PLATFORM_FEE"),
        }
    }
}

/// Unit of the measured quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QuantityUnit {
    TokensIn,
    TokensOut,
    Bytes,
    Seconds,
    Invocations,
}

impl std::fmt::Display for QuantityUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokensIn => write!(f, "TOKENS_IN"),
            Self::TokensOut => write!(f, "TOKENS_OUT"),
            Self::Bytes => write!(f, "BYTES"),
            Self::Seconds => write!(f, "SECONDS"),
            Self::Invocations => write!(f, "INVOCATIONS"),
        }
    }
}

// ---------------------------------------------------------------------------
// EnergyEvent (immutable record)
// ---------------------------------------------------------------------------

/// Atomic record of a single energy-consuming operation. Immutable once created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyEvent {
    pub id: String,
    pub energy_type: EnergyType,
    pub quantity: f64,
    pub unit: QuantityUnit,
    pub rate_per_unit: f64,
    pub total_cost: f64,
    pub cost_category: CostCategory,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
}

// ---------------------------------------------------------------------------
// EnergyRate
// ---------------------------------------------------------------------------

/// Conversion rate: maps an operation type + provider/model pattern to a unit cost.
///
/// `provider_pattern` and `model_pattern` support glob matching (e.g. `"openai"`, `"gpt-4*"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyRate {
    pub energy_type: EnergyType,
    pub provider_pattern: Option<String>,
    pub model_pattern: Option<String>,
    pub rate_per_unit: f64,
    pub unit: QuantityUnit,
    pub cost_category: CostCategory,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_type_display() {
        assert_eq!(EnergyType::LlmCall.to_string(), "LLM_CALL");
        assert_eq!(EnergyType::McpCall.to_string(), "MCP_CALL");
        assert_eq!(EnergyType::ToolExec.to_string(), "TOOL_EXEC");
        assert_eq!(EnergyType::ComputeTime.to_string(), "COMPUTE_TIME");
        assert_eq!(EnergyType::StorageOp.to_string(), "STORAGE_OP");
        assert_eq!(EnergyType::DbOp.to_string(), "DB_OP");
    }

    #[test]
    fn test_cost_category_display() {
        assert_eq!(CostCategory::InternalInfra.to_string(), "INTERNAL_INFRA");
        assert_eq!(
            CostCategory::ExternalService.to_string(),
            "EXTERNAL_SERVICE"
        );
        assert_eq!(CostCategory::PlatformFee.to_string(), "PLATFORM_FEE");
    }

    #[test]
    fn test_quantity_unit_display() {
        assert_eq!(QuantityUnit::TokensIn.to_string(), "TOKENS_IN");
        assert_eq!(QuantityUnit::TokensOut.to_string(), "TOKENS_OUT");
        assert_eq!(QuantityUnit::Bytes.to_string(), "BYTES");
        assert_eq!(QuantityUnit::Seconds.to_string(), "SECONDS");
        assert_eq!(QuantityUnit::Invocations.to_string(), "INVOCATIONS");
    }

    #[test]
    fn test_energy_event_serde_roundtrip() {
        let event = EnergyEvent {
            id: "ev-001".to_string(),
            energy_type: EnergyType::LlmCall,
            quantity: 1500.0,
            unit: QuantityUnit::TokensIn,
            rate_per_unit: 0.00001,
            total_cost: 0.015,
            cost_category: CostCategory::ExternalService,
            provider: Some("openai".into()),
            model: Some("gpt-4o".into()),
            metadata: HashMap::new(),
            timestamp: "2025-01-01T00:00:00Z".into(),
            session_id: Some("sess-1".into()),
            node_id: Some("node-1".into()),
        };

        let json = serde_json::to_string(&event).unwrap();
        let decoded: EnergyEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "ev-001");
        assert_eq!(decoded.energy_type, EnergyType::LlmCall);
        assert_eq!(decoded.total_cost, 0.015);
    }

    #[test]
    fn test_energy_rate_serde() {
        let rate = EnergyRate {
            energy_type: EnergyType::LlmCall,
            provider_pattern: Some("openai".into()),
            model_pattern: Some("gpt-4*".into()),
            rate_per_unit: 0.00003,
            unit: QuantityUnit::TokensOut,
            cost_category: CostCategory::ExternalService,
        };

        let json = serde_json::to_string(&rate).unwrap();
        let decoded: EnergyRate = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.rate_per_unit, 0.00003);
        assert_eq!(decoded.provider_pattern.as_deref(), Some("openai"));
    }
}
