//! Background scheduler for recurring agent execution.
//!
//! Spawns one tokio task per scheduled agent that sleeps for the configured
//! interval, then calls back into `AgentRuntime::execute_agent`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};

use super::agent_runtime::AgentRuntime;

// ---------------------------------------------------------------------------
// ScheduledEntry (internal bookkeeping)
// ---------------------------------------------------------------------------

struct ScheduledEntry {
    interval_seconds: u64,
    handle: JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Manages periodic execution of agents via tokio background tasks.
pub struct Scheduler {
    runtime: Arc<AgentRuntime>,
    entries: Arc<RwLock<HashMap<String, ScheduledEntry>>>,
    running: Arc<AtomicBool>,
}

impl Scheduler {
    /// Create a new scheduler bound to the given `AgentRuntime`.
    pub fn new(runtime: Arc<AgentRuntime>) -> Self {
        Self {
            runtime,
            entries: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Mark the scheduler as running.
    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
        info!("Scheduler started");
    }

    /// Stop the scheduler and abort every active handle.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let mut entries = self.entries.write().await;
        for (agent_id, entry) in entries.drain() {
            entry.handle.abort();
            info!(agent_id = %agent_id, "Unscheduled agent (scheduler stop)");
        }
        info!("Scheduler stopped");
    }

    /// Schedule an agent for periodic execution.
    ///
    /// If the agent was already scheduled, the previous task is aborted and
    /// replaced.
    pub async fn schedule_agent(&self, agent_id: &str, interval_seconds: u64) {
        // Cancel existing schedule if present.
        self.unschedule_agent(agent_id).await;

        let rt = Arc::clone(&self.runtime);
        let running = Arc::clone(&self.running);
        let aid = agent_id.to_string();
        let interval = interval_seconds;

        let handle = tokio::spawn(async move {
            // Wait one interval before the first execution (matches Python).
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

            while running.load(Ordering::SeqCst) {
                info!(agent_id = %aid, "Scheduler firing agent");
                if let Err(e) = rt.execute_agent(&aid, HashMap::new()).await {
                    error!(agent_id = %aid, error = %e, "Scheduled execution failed");
                }

                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        });

        let mut entries = self.entries.write().await;
        entries.insert(
            agent_id.to_string(),
            ScheduledEntry {
                interval_seconds,
                handle,
            },
        );
        info!(agent_id = %agent_id, interval_seconds = interval_seconds, "Agent scheduled");
    }

    /// Remove a scheduled agent and abort its background task.
    pub async fn unschedule_agent(&self, agent_id: &str) {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.remove(agent_id) {
            entry.handle.abort();
            info!(agent_id = %agent_id, "Agent unscheduled");
        }
    }

    /// Return `(agent_id, interval_seconds)` for all currently scheduled
    /// agents.
    pub async fn list_scheduled(&self) -> Vec<(String, u64)> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .map(|(id, e)| (id.clone(), e.interval_seconds))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::EventEmitter;
    use crate::runtime::agent_runtime::AgentRuntime;

    fn make_runtime() -> Arc<AgentRuntime> {
        Arc::new(AgentRuntime::new(EventEmitter::new(16)))
    }

    #[tokio::test]
    async fn schedule_and_list() {
        let rt = make_runtime();
        let sched = Scheduler::new(rt);
        sched.start();

        sched.schedule_agent("a1", 60).await;
        sched.schedule_agent("a2", 120).await;

        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 2);

        sched.stop().await;
        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 0);
    }

    #[tokio::test]
    async fn unschedule_agent() {
        let rt = make_runtime();
        let sched = Scheduler::new(rt);
        sched.start();

        sched.schedule_agent("a1", 60).await;
        assert_eq!(sched.list_scheduled().await.len(), 1);

        sched.unschedule_agent("a1").await;
        assert_eq!(sched.list_scheduled().await.len(), 0);

        sched.stop().await;
    }

    #[tokio::test]
    async fn reschedule_replaces() {
        let rt = make_runtime();
        let sched = Scheduler::new(rt);
        sched.start();

        sched.schedule_agent("a1", 60).await;
        sched.schedule_agent("a1", 120).await;

        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, 120);

        sched.stop().await;
    }

    #[tokio::test]
    async fn unschedule_nonexistent_is_noop() {
        let rt = make_runtime();
        let sched = Scheduler::new(rt);
        sched.unschedule_agent("ghost").await;
    }
}
