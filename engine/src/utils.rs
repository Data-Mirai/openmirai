//! Shared utility functions used across the engine.

/// Current Unix epoch timestamp as f64 seconds.
pub fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Short 8-character UUID (first 8 hex chars of a v4 UUID).
pub fn short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}
