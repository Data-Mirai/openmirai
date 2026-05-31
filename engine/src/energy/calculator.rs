//! EnergyCalculator -- resolves the best matching rate for an operation and computes cost.

use glob::Pattern;
use tracing::warn;

use crate::energy::models::{EnergyRate, EnergyType, QuantityUnit};

// ---------------------------------------------------------------------------
// RateStore trait
// ---------------------------------------------------------------------------

/// Read-only access to energy rates.
pub trait RateStore: Send + Sync {
    fn get_rates(&self) -> &[EnergyRate];
}

// ---------------------------------------------------------------------------
// InMemoryRateStore
// ---------------------------------------------------------------------------

/// Simple in-memory rate store backed by a `Vec<EnergyRate>`.
pub struct InMemoryRateStore {
    rates: Vec<EnergyRate>,
}

impl InMemoryRateStore {
    pub fn new(rates: Vec<EnergyRate>) -> Self {
        Self { rates }
    }
}

impl RateStore for InMemoryRateStore {
    fn get_rates(&self) -> &[EnergyRate] {
        &self.rates
    }
}

// ---------------------------------------------------------------------------
// EnergyCalculator
// ---------------------------------------------------------------------------

/// Resolves the best-matching rate for an operation and computes `quantity * rate_per_unit`.
///
/// Matching priority (specificity scoring):
/// - `energy_type` must match (hard filter).
/// - Provider pattern match = +1 point.
/// - Model pattern match = +2 points.
/// - Highest total score wins. Ties go to whichever rate was checked first.
pub struct EnergyCalculator<S: RateStore> {
    rate_store: S,
}

impl<S: RateStore> EnergyCalculator<S> {
    pub fn new(rate_store: S) -> Self {
        Self { rate_store }
    }

    /// Calculate the cost for the given operation.
    ///
    /// Returns `quantity * best_matching_rate.rate_per_unit`, or `0.0` if no rate matches.
    pub fn calculate(
        &self,
        energy_type: EnergyType,
        quantity: f64,
        unit: QuantityUnit,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> f64 {
        let mut best_rate: Option<&EnergyRate> = None;
        let mut best_score: i32 = -1;

        for rate in self.rate_store.get_rates() {
            // Hard filter: energy_type and unit must match.
            if rate.energy_type != energy_type {
                continue;
            }
            if rate.unit != unit {
                continue;
            }

            let mut score: i32 = 0;

            // Provider pattern match (+1).
            if let Some(ref pat_str) = rate.provider_pattern {
                match provider {
                    Some(prov) => {
                        if glob_match(pat_str, prov) {
                            score += 1;
                        } else {
                            continue; // provider mismatch -- skip
                        }
                    }
                    None => continue, // rate requires provider but none given
                }
            }

            // Model pattern match (+2).
            if let Some(ref pat_str) = rate.model_pattern {
                match model {
                    Some(mdl) => {
                        if glob_match(pat_str, mdl) {
                            score += 2;
                        } else {
                            continue; // model mismatch -- skip
                        }
                    }
                    None => continue, // rate requires model but none given
                }
            }

            if score > best_score {
                best_score = score;
                best_rate = Some(rate);
            }
        }

        match best_rate {
            Some(rate) => quantity * rate.rate_per_unit,
            None => {
                warn!(
                    energy_type = %energy_type,
                    unit = %unit,
                    provider = ?provider,
                    model = ?model,
                    "no energy rate found -- returning 0.0"
                );
                0.0
            }
        }
    }
}

/// Case-insensitive glob match.
fn glob_match(pattern: &str, value: &str) -> bool {
    match Pattern::new(&pattern.to_lowercase()) {
        Ok(pat) => pat.matches(&value.to_lowercase()),
        Err(_) => pattern.to_lowercase() == value.to_lowercase(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::energy::models::CostCategory;

    fn make_store(rates: Vec<EnergyRate>) -> InMemoryRateStore {
        InMemoryRateStore::new(rates)
    }

    #[test]
    fn test_no_rates_returns_zero() {
        let calc = EnergyCalculator::new(make_store(vec![]));
        let cost = calc.calculate(
            EnergyType::LlmCall,
            1000.0,
            QuantityUnit::TokensIn,
            None,
            None,
        );
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_exact_match_no_patterns() {
        let rates = vec![EnergyRate {
            energy_type: EnergyType::LlmCall,
            provider_pattern: None,
            model_pattern: None,
            rate_per_unit: 0.00001,
            unit: QuantityUnit::TokensIn,
            cost_category: CostCategory::ExternalService,
        }];

        let calc = EnergyCalculator::new(make_store(rates));
        let cost = calc.calculate(
            EnergyType::LlmCall,
            1000.0,
            QuantityUnit::TokensIn,
            None,
            None,
        );
        assert!((cost - 0.01).abs() < 1e-10);
    }

    #[test]
    fn test_provider_match() {
        let rates = vec![
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: None,
                model_pattern: None,
                rate_per_unit: 0.001,
                unit: QuantityUnit::TokensIn,
                cost_category: CostCategory::ExternalService,
            },
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: Some("openai".into()),
                model_pattern: None,
                rate_per_unit: 0.002,
                unit: QuantityUnit::TokensIn,
                cost_category: CostCategory::ExternalService,
            },
        ];

        let calc = EnergyCalculator::new(make_store(rates));

        // With provider=openai: should pick the more specific rate (0.002).
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensIn,
            Some("openai"),
            None,
        );
        assert!((cost - 0.2).abs() < 1e-10);

        // Without provider: falls back to generic rate (0.001).
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensIn,
            None,
            None,
        );
        assert!((cost - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_model_pattern_glob() {
        let rates = vec![
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: None,
                model_pattern: None,
                rate_per_unit: 0.001,
                unit: QuantityUnit::TokensOut,
                cost_category: CostCategory::ExternalService,
            },
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: None,
                model_pattern: Some("gpt-4*".into()),
                rate_per_unit: 0.003,
                unit: QuantityUnit::TokensOut,
                cost_category: CostCategory::ExternalService,
            },
        ];

        let calc = EnergyCalculator::new(make_store(rates));

        // model=gpt-4o matches "gpt-4*" (+2 specificity).
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensOut,
            None,
            Some("gpt-4o"),
        );
        assert!((cost - 0.3).abs() < 1e-10);

        // model=claude-3: no match, falls back to generic.
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensOut,
            None,
            Some("claude-3"),
        );
        assert!((cost - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_specificity_provider_plus_model_wins() {
        let rates = vec![
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: Some("openai".into()),
                model_pattern: None,
                rate_per_unit: 0.002,
                unit: QuantityUnit::TokensIn,
                cost_category: CostCategory::ExternalService,
            },
            EnergyRate {
                energy_type: EnergyType::LlmCall,
                provider_pattern: Some("openai".into()),
                model_pattern: Some("gpt-4*".into()),
                rate_per_unit: 0.005,
                unit: QuantityUnit::TokensIn,
                cost_category: CostCategory::ExternalService,
            },
        ];

        let calc = EnergyCalculator::new(make_store(rates));

        // provider+model match (score 3) beats provider-only (score 1).
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensIn,
            Some("openai"),
            Some("gpt-4-turbo"),
        );
        assert!((cost - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_unit_mismatch_skipped() {
        let rates = vec![EnergyRate {
            energy_type: EnergyType::LlmCall,
            provider_pattern: None,
            model_pattern: None,
            rate_per_unit: 0.01,
            unit: QuantityUnit::TokensOut,
            cost_category: CostCategory::ExternalService,
        }];

        let calc = EnergyCalculator::new(make_store(rates));

        // Asking for TokensIn but rate is for TokensOut -- no match.
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensIn,
            None,
            None,
        );
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_case_insensitive_glob() {
        let rates = vec![EnergyRate {
            energy_type: EnergyType::LlmCall,
            provider_pattern: Some("OpenAI".into()),
            model_pattern: None,
            rate_per_unit: 0.01,
            unit: QuantityUnit::TokensIn,
            cost_category: CostCategory::ExternalService,
        }];

        let calc = EnergyCalculator::new(make_store(rates));
        let cost = calc.calculate(
            EnergyType::LlmCall,
            100.0,
            QuantityUnit::TokensIn,
            Some("openai"),
            None,
        );
        assert!((cost - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_different_energy_type_not_matched() {
        let rates = vec![EnergyRate {
            energy_type: EnergyType::McpCall,
            provider_pattern: None,
            model_pattern: None,
            rate_per_unit: 0.01,
            unit: QuantityUnit::Invocations,
            cost_category: CostCategory::ExternalService,
        }];

        let calc = EnergyCalculator::new(make_store(rates));
        let cost = calc.calculate(
            EnergyType::LlmCall,
            1.0,
            QuantityUnit::Invocations,
            None,
            None,
        );
        assert_eq!(cost, 0.0);
    }
}
