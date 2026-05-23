use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// EventType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    SessionCompleted,
    SessionFailed,
    SessionInterrupted,
    SessionResumed,

    BlockStarted,
    BlockCompleted,
    BlockError,

    LlmToken,
    LlmCompleted,

    CheckpointCreated,

    HookFired,
    HookBlocked,

    InterruptCreated,
    InterruptResolved,

    BrowserScreenshot,
    BrowserAction,
    BrowserNavigation,
    BrowserCompleted,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::SessionStarted => "session_started",
            Self::SessionCompleted => "session_completed",
            Self::SessionFailed => "session_failed",
            Self::SessionInterrupted => "session_interrupted",
            Self::SessionResumed => "session_resumed",
            Self::BlockStarted => "block_started",
            Self::BlockCompleted => "block_completed",
            Self::BlockError => "block_error",
            Self::LlmToken => "llm_token",
            Self::LlmCompleted => "llm_completed",
            Self::CheckpointCreated => "checkpoint_created",
            Self::HookFired => "hook_fired",
            Self::HookBlocked => "hook_blocked",
            Self::InterruptCreated => "interrupt_created",
            Self::InterruptResolved => "interrupt_resolved",
            Self::BrowserScreenshot => "browser_screenshot",
            Self::BrowserAction => "browser_action",
            Self::BrowserNavigation => "browser_navigation",
            Self::BrowserCompleted => "browser_completed",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// ExecutionEvent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub event_type: EventType,
    pub timestamp: f64,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub data: HashMap<String, serde_json::Value>,
    pub event_id: u64,
}

impl ExecutionEvent {
    /// Format the event as a Server-Sent Event string.
    ///
    /// Output follows the SSE spec:
    /// ```text
    /// event: <event_type>
    /// data: <json payload>
    /// ```
    pub fn to_sse(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        format!("event: {}\ndata: {}\n\n", self.event_type, json)
    }
}

// ---------------------------------------------------------------------------
// EventEmitter
// ---------------------------------------------------------------------------

/// Broadcast-based event emitter backed by `tokio::sync::broadcast`.
///
/// Cloning an `EventEmitter` shares the same underlying channel and counter,
/// so all clones emit into / subscribe from the same stream.
#[derive(Clone)]
pub struct EventEmitter {
    sender: broadcast::Sender<ExecutionEvent>,
    counter: Arc<AtomicU64>,
}

impl EventEmitter {
    /// Create a new emitter with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Emit an event, automatically assigning the next event_id and the
    /// current unix timestamp.
    ///
    /// Returns the assigned `event_id`.
    pub fn emit(
        &self,
        event_type: EventType,
        session_id: String,
        node_id: Option<String>,
        data: HashMap<String, serde_json::Value>,
    ) -> u64 {
        let event_id = self.counter.fetch_add(1, Ordering::Relaxed);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let event = ExecutionEvent {
            event_type,
            timestamp,
            session_id,
            node_id,
            data,
            event_id,
        };

        // If there are no active receivers the send will "fail" — that is fine.
        let _ = self.sender.send(event);

        event_id
    }

    /// Emit a pre-built event, overwriting its `event_id` with the next
    /// auto-incremented value (timestamp is left as-is).
    pub fn emit_raw(&self, mut event: ExecutionEvent) -> u64 {
        let event_id = self.counter.fetch_add(1, Ordering::Relaxed);
        event.event_id = event_id;
        let _ = self.sender.send(event);
        event_id
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.sender.subscribe()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_and_receive() {
        let emitter = EventEmitter::new(16);
        let mut rx = emitter.subscribe();

        let id = emitter.emit(
            EventType::SessionStarted,
            "sess-1".into(),
            None,
            HashMap::new(),
        );
        assert_eq!(id, 0);

        let event = rx.recv().await.unwrap();
        assert_eq!(event.event_id, 0);
        assert_eq!(event.event_type, EventType::SessionStarted);
        assert_eq!(event.session_id, "sess-1");
    }

    #[test]
    fn event_to_sse_format() {
        let event = ExecutionEvent {
            event_type: EventType::LlmToken,
            timestamp: 1700000000.123,
            session_id: "s1".into(),
            node_id: Some("n1".into()),
            data: HashMap::new(),
            event_id: 42,
        };

        let sse = event.to_sse();
        assert!(sse.starts_with("event: llm_token\n"));
        assert!(sse.contains("data: {"));
        assert!(sse.ends_with("\n\n"));
    }

    #[test]
    fn auto_increment_ids() {
        let emitter = EventEmitter::new(16);
        let id0 = emitter.emit(EventType::BlockStarted, "s".into(), None, HashMap::new());
        let id1 = emitter.emit(EventType::BlockCompleted, "s".into(), None, HashMap::new());
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
    }

    #[test]
    fn serde_roundtrip() {
        let event = ExecutionEvent {
            event_type: EventType::BrowserScreenshot,
            timestamp: 1.0,
            session_id: "s".into(),
            node_id: None,
            data: HashMap::new(),
            event_id: 0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ExecutionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_type, EventType::BrowserScreenshot);
        assert!(back.node_id.is_none());
    }
}
