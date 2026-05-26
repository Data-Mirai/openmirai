//! EnergyRecorder -- creates energy events and persists them atomically.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::warn;

use crate::energy::calculator::{EnergyCalculator, RateStore};
use crate::energy::models::{CostCategory, EnergyEvent, EnergyType, QuantityUnit};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the recording pipeline.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("event store error: {0}")]
    EventStore(String),

    #[error("balance store error: {0}")]
    BalanceStore(String),

    #[error("insufficient balance: available={available}, required={required}")]
    InsufficientBalance { available: f64, required: f64 },
}

// ---------------------------------------------------------------------------
// EventStore trait
// ---------------------------------------------------------------------------

/// Async interface for persisting energy events.
#[async_trait]
pub trait EventStore: Send + Sync {
    async fn save(&self, event: &EnergyEvent) -> Result<(), RecorderError>;
}

// ---------------------------------------------------------------------------
// BalanceStore trait
// ---------------------------------------------------------------------------

/// Async interface for reading/updating energy balances.
#[async_trait]
pub trait BalanceStore: Send + Sync {
    /// Deduct `amount` from the balance of the given session. Returns new balance.
    async fn deduct(&self, amount: f64, session_id: &str) -> Result<f64, RecorderError>;

    /// Get current balance for a session.
    async fn get_balance(&self, session_id: &str) -> Result<f64, RecorderError>;
}

// ---------------------------------------------------------------------------
// InMemoryEventStore
// ---------------------------------------------------------------------------

/// Simple in-memory event store for testing and development.
pub struct InMemoryEventStore {
    events: Arc<RwLock<Vec<EnergyEvent>>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Return a snapshot of all recorded events.
    pub async fn get_all(&self) -> Vec<EnergyEvent> {
        self.events.read().await.clone()
    }

    /// Return the number of stored events.
    pub async fn len(&self) -> usize {
        self.events.read().await.len()
    }

    /// Check if the store is empty.
    pub async fn is_empty(&self) -> bool {
        self.events.read().await.is_empty()
    }
}

impl Default for InMemoryEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn save(&self, event: &EnergyEvent) -> Result<(), RecorderError> {
        self.events.write().await.push(event.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// EnergyRecorder
// ---------------------------------------------------------------------------

/// Records energy consumption: calculates cost, persists the event, optionally deducts balance.
pub struct EnergyRecorder<S: RateStore> {
    calculator: EnergyCalculator<S>,
    event_store: Box<dyn EventStore>,
    balance_store: Option<Box<dyn BalanceStore>>,
}

impl<S: RateStore> EnergyRecorder<S> {
    pub fn new(calculator: EnergyCalculator<S>, event_store: Box<dyn EventStore>) -> Self {
        Self {
            calculator,
            event_store,
            balance_store: None,
        }
    }

    /// Attach a balance store for deduction on each recorded event.
    pub fn with_balance_store(mut self, store: Box<dyn BalanceStore>) -> Self {
        self.balance_store = Some(store);
        self
    }

    /// Record an energy-consuming operation.
    ///
    /// 1. Calculate cost via the calculator.
    /// 2. Create an [`EnergyEvent`].
    /// 3. Save to the event store.
    /// 4. Deduct from balance store (if present).
    pub async fn record(
        &self,
        energy_type: EnergyType,
        quantity: f64,
        unit: QuantityUnit,
        provider: Option<&str>,
        model: Option<&str>,
        session_id: Option<&str>,
        node_id: Option<&str>,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<EnergyEvent, RecorderError> {
        let total_cost = self.calculator.calculate(energy_type, quantity, unit, provider, model);
        let rate_per_unit = if quantity > 0.0 { total_cost / quantity } else { 0.0 };

        let event = EnergyEvent {
            id: uuid::Uuid::new_v4().to_string(),
            energy_type,
            quantity,
            unit,
            rate_per_unit,
            total_cost,
            cost_category: infer_cost_category(energy_type),
            provider: provider.map(String::from),
            model: model.map(String::from),
            metadata,
            timestamp: chrono::Utc::now().to_rfc3339(),
            session_id: session_id.map(String::from),
            node_id: node_id.map(String::from),
        };

        // Persist.
        self.event_store.save(&event).await?;

        // Deduct balance if a store is configured.
        if let (Some(balance_store), Some(sid)) = (&self.balance_store, session_id) {
            if total_cost > 0.0 {
                match balance_store.deduct(total_cost, sid).await {
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, session_id = sid, "balance deduction failed");
                    }
                }
            }
        }

        Ok(event)
    }
}

/// Infer a default cost category from the energy type.
fn infer_cost_category(energy_type: EnergyType) -> CostCategory {
    match energy_type {
        EnergyType::LlmCall | EnergyType::McpCall => CostCategory::ExternalService,
        EnergyType::ToolExec | EnergyType::ComputeTime | EnergyType::StorageOp | EnergyType::DbOp => {
            CostCategory::InternalInfra
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::energy::calculator::InMemoryRateStore;
    use crate::energy::models::EnergyRate;

    fn test_calculator() -> EnergyCalculator<InMemoryRateStore> {
        let rates = vec![
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: None,
                model_pattern: None,
                rate_per_unit: 0.00001,
                unit: QuantityUnit::TokensIn,
                cost_category: CostCategory::ExternalService,
            },
            EnergyRate {
                energy_type: EnergyType::McpCall,
                provider_pattern: None,
                model_pattern: None,
                rate_per_unit: 0.01,
                unit: QuantityUnit::Invocations,
                cost_category: CostCategory::ExternalService,
            },
        ];
        EnergyCalculator::new(InMemoryRateStore::new(rates))
    }

    #[tokio::test]
    async fn test_record_creates_event() {
        let store = InMemoryEventStore::new();
        // We need a reference to check the store later, but EnergyRecorder takes ownership.
        // Use Arc<InMemoryEventStore> by wrapping in a newtype.
        let events_ref = store.events.clone();

        let recorder = EnergyRecorder::new(test_calculator(), Box::new(store));

        let event = recorder
            .record(
                EnergyType::LlmCall,
                1000.0,
                QuantityUnit::TokensIn,
                Some("openai"),
                Some("gpt-4"),
                Some("sess-1"),
                Some("node-1"),
                HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(event.energy_type, EnergyType::LlmCall);
        assert_eq!(event.quantity, 1000.0);
        assert!((event.total_cost - 0.01).abs() < 1e-10);
        assert_eq!(event.session_id.as_deref(), Some("sess-1"));
        assert_eq!(event.node_id.as_deref(), Some("node-1"));

        // Verify it was saved.
        let saved = events_ref.read().await;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, event.id);
    }

    #[tokio::test]
    async fn test_record_with_no_matching_rate() {
        let calc = EnergyCalculator::new(InMemoryRateStore::new(vec![]));
        let store = InMemoryEventStore::new();
        let recorder = EnergyRecorder::new(calc, Box::new(store));

        let event = recorder
            .record(
                EnergyType::DbOp,
                50.0,
                QuantityUnit::Invocations,
                None,
                None,
                None,
                None,
                HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(event.total_cost, 0.0);
        assert_eq!(event.rate_per_unit, 0.0);
    }

    #[tokio::test]
    async fn test_record_with_balance_store() {
        struct FakeBalance {
            deducted: Arc<RwLock<Vec<(f64, String)>>>,
        }

        #[async_trait]
        impl BalanceStore for FakeBalance {
            async fn deduct(&self, amount: f64, session_id: &str) -> Result<f64, RecorderError> {
                self.deducted
                    .write()
                    .await
                    .push((amount, session_id.to_string()));
                Ok(100.0 - amount)
            }

            async fn get_balance(&self, _session_id: &str) -> Result<f64, RecorderError> {
                Ok(100.0)
            }
        }

        let deducted = Arc::new(RwLock::new(Vec::new()));
        let balance = FakeBalance {
            deducted: deducted.clone(),
        };

        let store = InMemoryEventStore::new();
        let recorder = EnergyRecorder::new(test_calculator(), Box::new(store))
            .with_balance_store(Box::new(balance));

        let _event = recorder
            .record(
                EnergyType::LlmCall,
                2000.0,
                QuantityUnit::TokensIn,
                None,
                None,
                Some("sess-42"),
                None,
                HashMap::new(),
            )
            .await
            .unwrap();

        let deductions = deducted.read().await;
        assert_eq!(deductions.len(), 1);
        assert!((deductions[0].0 - 0.02).abs() < 1e-10);
        assert_eq!(deductions[0].1, "sess-42");
    }

    #[tokio::test]
    async fn test_record_zero_cost_no_deduction() {
        struct PanicBalance;

        #[async_trait]
        impl BalanceStore for PanicBalance {
            async fn deduct(&self, _amount: f64, _session_id: &str) -> Result<f64, RecorderError> {
                panic!("should not be called for zero cost");
            }
            async fn get_balance(&self, _session_id: &str) -> Result<f64, RecorderError> {
                Ok(999.0)
            }
        }

        // No rates -> cost is 0 -> balance should NOT be deducted.
        let calc = EnergyCalculator::new(InMemoryRateStore::new(vec![]));
        let store = InMemoryEventStore::new();
        let recorder =
            EnergyRecorder::new(calc, Box::new(store)).with_balance_store(Box::new(PanicBalance));

        let event = recorder
            .record(
                EnergyType::StorageOp,
                100.0,
                QuantityUnit::Bytes,
                None,
                None,
                Some("sess"),
                None,
                HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(event.total_cost, 0.0);
    }

    #[tokio::test]
    async fn test_infer_cost_category() {
        assert_eq!(infer_cost_category(EnergyType::LlmCall), CostCategory::ExternalService);
        assert_eq!(infer_cost_category(EnergyType::McpCall), CostCategory::ExternalService);
        assert_eq!(infer_cost_category(EnergyType::ToolExec), CostCategory::InternalInfra);
        assert_eq!(infer_cost_category(EnergyType::ComputeTime), CostCategory::InternalInfra);
        assert_eq!(infer_cost_category(EnergyType::StorageOp), CostCategory::InternalInfra);
        assert_eq!(infer_cost_category(EnergyType::DbOp), CostCategory::InternalInfra);
    }

    #[tokio::test]
    async fn test_in_memory_event_store() {
        let store = InMemoryEventStore::new();
        assert!(store.is_empty().await);

        let event = EnergyEvent {
            id: "test-1".into(),
            energy_type: EnergyType::ToolExec,
            quantity: 1.0,
            unit: QuantityUnit::Invocations,
            rate_per_unit: 0.0,
            total_cost: 0.0,
            cost_category: CostCategory::InternalInfra,
            provider: None,
            model: None,
            metadata: HashMap::new(),
            timestamp: "2025-01-01T00:00:00Z".into(),
            session_id: None,
            node_id: None,
        };

        store.save(&event).await.unwrap();
        assert_eq!(store.len().await, 1);

        let all = store.get_all().await;
        assert_eq!(all[0].id, "test-1");
    }
}
