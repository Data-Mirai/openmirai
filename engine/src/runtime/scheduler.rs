//! Background scheduler for recurring agent execution (PRD-008).
//!
//! Spawns one tokio task per live agent that sleeps for the configured
//! interval, then calls the provided execution callback.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::utils::now_epoch;

use super::agent_runtime::{CycleRecord, CycleStatus};

// ---------------------------------------------------------------------------
// Cycle execution callback
// ---------------------------------------------------------------------------

/// Callback invoked by the scheduler to execute a single cycle.
///
/// Receives (agent_id, cycle_number, is_first_cycle_of_session).
/// Returns Ok(()) on success, Err(error_message) on failure.
pub type CycleCallback =
    Arc<dyn Fn(String, u64, bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> + Send + Sync>;

// ---------------------------------------------------------------------------
// ScheduledEntry (internal bookkeeping)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct ScheduledEntry {
    interval_seconds: u64,
    handle: JoinHandle<()>,
    max_cycles: Option<u64>,
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Manages periodic execution of live agents via tokio background tasks.
pub struct Scheduler {
    entries: Arc<RwLock<HashMap<String, ScheduledEntry>>>,
    running: Arc<AtomicBool>,
    /// Per-agent cycle records (most recent N).
    cycle_history: Arc<RwLock<HashMap<String, Vec<CycleRecord>>>>,
    /// Per-agent cycle counter (total cycles completed in current session).
    cycle_counters: Arc<RwLock<HashMap<String, u64>>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
            cycle_history: Arc::new(RwLock::new(HashMap::new())),
            cycle_counters: Arc::new(RwLock::new(HashMap::new())),
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

    /// Schedule a live agent for periodic execution.
    ///
    /// `on_cycle_error`: "continue" or "stop".
    /// `callback`: async function called for each cycle.
    pub async fn schedule_agent(
        &self,
        agent_id: &str,
        interval_seconds: u64,
        max_cycles: Option<u64>,
        on_cycle_error: &str,
        callback: CycleCallback,
    ) {
        self.unschedule_agent(agent_id).await;

        // Reset cycle counter for this session
        {
            let mut counters = self.cycle_counters.write().await;
            counters.insert(agent_id.to_string(), 0);
        }

        let running = Arc::clone(&self.running);
        let aid = agent_id.to_string();
        let interval = interval_seconds;
        let error_mode = on_cycle_error.to_string();
        let max = max_cycles;
        let history = Arc::clone(&self.cycle_history);
        let counters = Arc::clone(&self.cycle_counters);

        let handle = tokio::spawn(async move {
            let mut cycle_num: u64 = 0;
            let mut is_first = true;

            while running.load(Ordering::SeqCst) {
                // Check max_cycles
                if let Some(max_c) = max {
                    if cycle_num >= max_c {
                        info!(agent_id = %aid, cycles = cycle_num, "max_cycles reached — auto-stopping");
                        break;
                    }
                }

                // Sleep interval (before first cycle too, like cron)
                if !is_first {
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                }

                cycle_num += 1;
                let cycle_id = format!("cyc-{}", crate::utils::short_id());
                let started_at = now_epoch();

                info!(agent_id = %aid, cycle = cycle_num, "Cycle started");

                let result = callback(aid.clone(), cycle_num, is_first).await;
                is_first = false;

                let completed_at = now_epoch();
                let duration_ms = ((completed_at - started_at) * 1000.0) as u64;

                let record = match &result {
                    Ok(()) => {
                        info!(agent_id = %aid, cycle = cycle_num, duration_ms = duration_ms, "Cycle completed");
                        CycleRecord {
                            cycle_id,
                            agent_id: aid.clone(),
                            cycle_number: cycle_num,
                            started_at,
                            completed_at: Some(completed_at),
                            status: CycleStatus::Completed,
                            duration_ms: Some(duration_ms),
                            error: None,
                        }
                    }
                    Err(e) => {
                        warn!(agent_id = %aid, cycle = cycle_num, error = %e, "Cycle failed");
                        CycleRecord {
                            cycle_id,
                            agent_id: aid.clone(),
                            cycle_number: cycle_num,
                            started_at,
                            completed_at: Some(completed_at),
                            status: CycleStatus::Failed,
                            duration_ms: Some(duration_ms),
                            error: Some(e.clone()),
                        }
                    }
                };

                // Store cycle record
                {
                    let mut hist = history.write().await;
                    let agent_hist = hist.entry(aid.clone()).or_default();
                    agent_hist.push(record);
                    // Keep last 1000 records per agent
                    if agent_hist.len() > 1000 {
                        agent_hist.drain(..agent_hist.len() - 1000);
                    }
                }

                // Update counter
                {
                    let mut ctrs = counters.write().await;
                    *ctrs.entry(aid.clone()).or_insert(0) = cycle_num;
                }

                // Handle error mode
                if result.is_err() && error_mode == "stop" {
                    error!(agent_id = %aid, "on_cycle_error=stop — stopping agent");
                    break;
                }

            }
        });

        let mut entries = self.entries.write().await;
        entries.insert(
            agent_id.to_string(),
            ScheduledEntry {
                interval_seconds,
                handle,
                max_cycles,
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

    /// Return `(agent_id, interval_seconds)` for all currently scheduled agents.
    pub async fn list_scheduled(&self) -> Vec<(String, u64)> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .map(|(id, e)| (id.clone(), e.interval_seconds))
            .collect()
    }

    /// Get cycle history for an agent.
    pub async fn get_cycles(&self, agent_id: &str, limit: usize) -> Vec<CycleRecord> {
        let hist = self.cycle_history.read().await;
        match hist.get(agent_id) {
            Some(records) => {
                let start = if records.len() > limit {
                    records.len() - limit
                } else {
                    0
                };
                records[start..].to_vec()
            }
            None => vec![],
        }
    }

    /// Get total cycles completed in current session.
    pub async fn get_cycle_count(&self, agent_id: &str) -> u64 {
        let ctrs = self.cycle_counters.read().await;
        ctrs.get(agent_id).copied().unwrap_or(0)
    }

    /// Check if an agent is currently scheduled.
    pub async fn is_scheduled(&self, agent_id: &str) -> bool {
        let entries = self.entries.read().await;
        entries.contains_key(agent_id)
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn schedule_and_list() {
        let sched = Scheduler::new();
        sched.start();

        let cb: CycleCallback = Arc::new(|_aid, _cycle, _first| {
            Box::pin(async { Ok(()) })
        });

        sched.schedule_agent("a1", 60, None, "continue", cb.clone()).await;
        sched.schedule_agent("a2", 120, None, "continue", cb).await;

        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 2);

        sched.stop().await;
        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 0);
    }

    #[tokio::test]
    async fn unschedule_agent() {
        let sched = Scheduler::new();
        sched.start();

        let cb: CycleCallback = Arc::new(|_aid, _cycle, _first| {
            Box::pin(async { Ok(()) })
        });

        sched.schedule_agent("a1", 60, None, "continue", cb).await;
        assert_eq!(sched.list_scheduled().await.len(), 1);

        sched.unschedule_agent("a1").await;
        assert_eq!(sched.list_scheduled().await.len(), 0);

        sched.stop().await;
    }

    #[tokio::test]
    async fn reschedule_replaces() {
        let sched = Scheduler::new();
        sched.start();

        let cb: CycleCallback = Arc::new(|_aid, _cycle, _first| {
            Box::pin(async { Ok(()) })
        });

        sched.schedule_agent("a1", 60, None, "continue", cb.clone()).await;
        sched.schedule_agent("a1", 120, None, "continue", cb).await;

        let list = sched.list_scheduled().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, 120);

        sched.stop().await;
    }

    #[tokio::test]
    async fn unschedule_nonexistent_is_noop() {
        let sched = Scheduler::new();
        sched.unschedule_agent("ghost").await;
    }

    #[tokio::test]
    async fn is_scheduled_check() {
        let sched = Scheduler::new();
        sched.start();

        let cb: CycleCallback = Arc::new(|_aid, _cycle, _first| {
            Box::pin(async { Ok(()) })
        });

        assert!(!sched.is_scheduled("a1").await);
        sched.schedule_agent("a1", 60, None, "continue", cb).await;
        assert!(sched.is_scheduled("a1").await);

        sched.stop().await;
    }
}
