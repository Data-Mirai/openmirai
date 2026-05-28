//! Free helper functions used across the runner module.

use std::time::Duration;

use serde_json::Value;
use tracing::warn;

use super::types::HookResult;

pub(crate) use crate::utils::now_epoch as now_ts;

/// Try to extract f64 from two JSON values and apply a comparator.
pub(crate) fn compare_numbers(a: &Value, b: &Value, cmp: fn(f64, f64) -> bool) -> bool {
    match (as_f64(a), as_f64(b)) {
        (Some(x), Some(y)) => cmp(x, y),
        _ => false,
    }
}

pub(crate) fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// Run a hook future with a 30-second timeout. On timeout, return Continue.
pub(crate) async fn run_hook_with_timeout<F>(fut: F) -> HookResult
where
    F: std::future::Future<Output = HookResult>,
{
    match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Ok(result) => result,
        Err(_) => {
            warn!("hook timed out after 30s — continuing");
            HookResult::Continue
        }
    }
}
