//! FleetSubscriber — a filtering pub-sub consumer over the fleet event stream.
//!
//! The fleet SoT is the **publisher**: every mutation broadcasts a [`FleetEvent`]
//! on its tokio `broadcast` channel (the same feed the `/fleet/events` SSE
//! endpoint drains). This is the in-process **subscriber** side — it wraps a
//! `broadcast::Receiver<FleetEvent>` and yields only the events matching an
//! [`EventFilter`].
//!
//! It is the natural consumer of the reactive `objective_complete` notification:
//! *"wake me when THIS parent's objective is done"* — without polling the SoT.

use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use super::store::FleetStore;
use super::types::{FleetEvent, FleetEventKind};

/// Predicate over fleet events. Every unset field matches everything, so the
/// default filter is a pass-through.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Keep only these event kinds. `None` → every kind.
    pub kinds: Option<Vec<FleetEventKind>>,
    /// Keep only events whose `member.id` equals this. For `objective_complete`
    /// the `member` is the **parent**, so this filters by parent id.
    pub member_id: Option<String>,
    /// Keep only events whose `member.parent_id` equals this (a member's own
    /// parent link — e.g. watch every child of one parent).
    pub parent_id: Option<String>,
}

impl EventFilter {
    /// Does `event` pass this filter?
    pub fn matches(&self, event: &FleetEvent) -> bool {
        if let Some(kinds) = &self.kinds {
            if !kinds.contains(&event.kind) {
                return false;
            }
        }
        if let Some(id) = &self.member_id {
            if &event.member.id != id {
                return false;
            }
        }
        if let Some(pid) = &self.parent_id {
            if &event.member.parent_id != pid {
                return false;
            }
        }
        true
    }
}

/// A filtering subscriber over the fleet event broadcast.
///
/// Created against a live [`FleetStore`]; only events published *after*
/// construction are delivered (tokio broadcast has no replay), so build the
/// subscriber before triggering the writes you want to observe.
///
/// It keeps a handle to the store so it can **self-heal on lag**: the
/// `objective_complete` event is edge-triggered (fired once, on the completing
/// transition, never repeated), so if the bounded broadcast buffer overflows
/// and drops it, [`recv`](Self::recv) re-derives the completion from the SoT
/// instead of losing it silently.
pub struct FleetSubscriber {
    store: Arc<FleetStore>,
    rx: Receiver<FleetEvent>,
    filter: EventFilter,
}

impl FleetSubscriber {
    /// Subscribe to `store`, keeping only events that match `filter`.
    pub fn new(store: Arc<FleetStore>, filter: EventFilter) -> Self {
        let rx = store.subscribe();
        Self { store, rx, filter }
    }

    /// Subscribe for `objective_complete` events, optionally scoped to a single
    /// parent id. `None` → every objective that completes. A parent-scoped
    /// subscriber additionally recovers a dropped completion after a lag.
    pub fn objective_complete(store: Arc<FleetStore>, parent_id: Option<String>) -> Self {
        Self::new(
            store,
            EventFilter {
                kinds: Some(vec![FleetEventKind::ObjectiveComplete]),
                member_id: parent_id,
                ..Default::default()
            },
        )
    }

    /// Await the next event that passes the filter.
    ///
    /// Non-matching events are dropped and `None` is returned once the publisher
    /// (the store) is gone. On a broadcast **lag** the buffer may have evicted
    /// the one-shot `objective_complete`; a parent-scoped objective subscriber
    /// re-reads the SoT and yields a reconstructed completion so the reactive
    /// notification is never lost (at-least-once under lag, exactly-once on the
    /// happy path).
    pub async fn recv(&mut self) -> Option<FleetEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) if self.filter.matches(&event) => return Some(event),
                Ok(_) => continue, // published but filtered out
                Err(RecvError::Lagged(_)) => {
                    if let Some(recovered) = self.resync_after_lag() {
                        return Some(recovered);
                    }
                    continue;
                }
                Err(RecvError::Closed) => return None,
            }
        }
    }

    /// After a lag, reconstruct a completion the ring may have dropped.
    ///
    /// Only recoverable for a parent-scoped `objective_complete` filter: the
    /// target parent is known and its progress is a pure function of the SoT.
    /// Returns the synthesized event iff that parent's objective is currently
    /// complete; `None` otherwise (nothing to recover, or an unrecoverable
    /// general filter).
    fn resync_after_lag(&self) -> Option<FleetEvent> {
        let wants_complete = self
            .filter
            .kinds
            .as_ref()
            .map(|k| k.contains(&FleetEventKind::ObjectiveComplete))
            .unwrap_or(true);
        if !wants_complete {
            return None;
        }
        let parent = self.filter.member_id.as_deref()?;
        let progress = self.store.objective_progress(parent).ok()?;
        if !progress.complete {
            return None;
        }
        let member = self.store.get(parent).ok()??;
        Some(FleetEvent {
            kind: FleetEventKind::ObjectiveComplete,
            member,
            progress: Some(progress),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::{FleetStatus, StatusUpdate};
    use std::time::Duration;

    fn child(id: &str, parent: &str, status: FleetStatus) -> StatusUpdate {
        StatusUpdate {
            id: id.to_string(),
            status,
            name: None,
            kind: None,
            host: None,
            activity: None,
            objective: None,
            parent_id: Some(parent.to_string()),
            project_dir: None,
            metadata: None,
        }
    }

    fn bare(id: &str, status: FleetStatus) -> StatusUpdate {
        StatusUpdate {
            id: id.to_string(),
            status,
            name: None,
            kind: None,
            host: None,
            activity: None,
            objective: None,
            parent_id: None,
            project_dir: None,
            metadata: None,
        }
    }

    #[test]
    fn filter_matches_by_kind_and_parent() {
        let store = FleetStore::in_memory().unwrap();
        store.apply_status(bare("p", FleetStatus::Working)).unwrap();
        let (member, _) = store.apply_status(child("c", "p", FleetStatus::Working)).unwrap();
        let ev = FleetEvent {
            kind: FleetEventKind::Updated,
            member,
            progress: None,
        };
        assert!(EventFilter::default().matches(&ev), "empty filter passes all");
        assert!(EventFilter {
            parent_id: Some("p".into()),
            ..Default::default()
        }
        .matches(&ev));
        assert!(!EventFilter {
            kinds: Some(vec![FleetEventKind::ObjectiveComplete]),
            ..Default::default()
        }
        .matches(&ev));
    }

    #[tokio::test]
    async fn subscriber_yields_only_objective_complete_for_its_parent() {
        let store = Arc::new(FleetStore::in_memory().unwrap());
        // Parent + two children must exist before we start watching.
        store.apply_status(bare("parent", FleetStatus::Working)).unwrap();
        store.apply_status(child("a", "parent", FleetStatus::Working)).unwrap();
        store.apply_status(child("b", "parent", FleetStatus::Working)).unwrap();
        // A decoy parent whose completion must NOT reach our subscriber.
        store.apply_status(bare("other", FleetStatus::Working)).unwrap();
        store.apply_status(child("x", "other", FleetStatus::Working)).unwrap();

        let mut sub = FleetSubscriber::objective_complete(store.clone(), Some("parent".into()));

        // Complete the decoy first — filtered out (wrong parent).
        store.apply_status(child("x", "other", FleetStatus::Done)).unwrap();
        // Partial progress on our parent — not a completion, skipped.
        store.apply_status(child("a", "parent", FleetStatus::Done)).unwrap();
        // The completing write.
        store.apply_status(child("b", "parent", FleetStatus::Done)).unwrap();

        let ev = tokio::time::timeout(Duration::from_secs(2), sub.recv())
            .await
            .expect("subscriber timed out")
            .expect("stream closed");
        assert_eq!(ev.kind, FleetEventKind::ObjectiveComplete);
        assert_eq!(ev.member.id, "parent", "member is the parent");
        let progress = ev.progress.expect("progress attached");
        assert_eq!(progress.parent_id, "parent");
        assert_eq!(progress.total, 2);
        assert_eq!(progress.done, 2);
        assert!(progress.complete);
    }

    #[tokio::test]
    async fn subscriber_recovers_completion_dropped_by_broadcast_lag() {
        // A completion that overflows the bounded broadcast ring before the
        // subscriber polls must still be delivered — re-derived from the SoT —
        // not silently lost (the reactive notification is edge-triggered and
        // never re-fires on its own).
        let store = Arc::new(FleetStore::in_memory().unwrap());
        store.apply_status(bare("p", FleetStatus::Working)).unwrap();
        store.apply_status(child("only", "p", FleetStatus::Working)).unwrap();

        let mut sub = FleetSubscriber::objective_complete(store.clone(), Some("p".into()));

        // Complete the objective — this broadcasts the one-shot objective_complete.
        store.apply_status(child("only", "p", FleetStatus::Done)).unwrap();
        // Now bury it: churn far more than the 256-slot ring capacity WITHOUT
        // polling the subscriber, so the completion frame is evicted.
        for i in 0..400 {
            let status = if i % 2 == 0 {
                FleetStatus::Working
            } else {
                FleetStatus::Idle
            };
            store.apply_status(bare("noise", status)).unwrap();
        }

        // The next recv sees Lagged; instead of losing the completion, it
        // resyncs from the SoT and yields the reconstructed event.
        let ev = tokio::time::timeout(Duration::from_secs(2), sub.recv())
            .await
            .expect("subscriber timed out")
            .expect("stream closed");
        assert_eq!(ev.kind, FleetEventKind::ObjectiveComplete);
        assert_eq!(ev.member.id, "p");
        assert!(ev.progress.unwrap().complete);
    }
}
