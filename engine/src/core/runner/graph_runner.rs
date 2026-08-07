//! GraphRunner — the core execution engine.
//!
//! Traverses a validated DAG, executing nodes one at a time with support
//! for conditional branching, retry with backoff, hooks, checkpoints,
//! fan-out/fan-in, human-input interrupts, and SSE streaming.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;
use tracing::{debug, error, info, warn};

use crate::core::context::ExecutionContext;
use crate::core::events::{EventEmitter, EventType};
use crate::core::graph::{ComparisonOp, EdgeCondition, GraphDef, NodeDef};
use crate::core::state::SharedState;
use crate::core::well_known as wk;

use super::helpers::{compare_numbers, now_ts, run_hook_with_timeout};
use super::traits::{CheckpointCallback, HookHandler, ToolExecutor};
use super::types::*;

/// Outcome of handling a node failure — used by `handle_node_failure`.
enum NodeFailureOutcome {
    /// Execution stopped — return this result immediately.
    Stop(ExecutionResult),
    /// Hook requested retry — re-execute the same node.
    Retry,
    /// Advance cursor to the next node (may be None = end of graph).
    Advance(Option<usize>),
}

// ---------------------------------------------------------------------------
// GraphRunner
// ---------------------------------------------------------------------------

/// Sequential cursor that traverses a validated DAG, executing nodes one at a
/// time.  Supports conditional branching, retry with backoff, template-based
/// input resolution, real-time event emission, hooks, checkpoints,
/// human_input interrupts, pause/resume, and transcript generation.
///
/// `GraphRunner` is `Clone` — cloning shares the executor, hook handler,
/// and checkpoint callback via `Arc`, enabling reuse across sessions.
#[derive(Clone)]
pub struct GraphRunner {
    executor: Arc<dyn ToolExecutor>,
    /// **Bus de observabilidad** (`ExecutionEvent`, `broadcast` de proceso).
    /// Lo alimenta [`GraphRunner::emit_event`]. Se cablea en `AppState::new` y
    /// se consume en `GET /api/v1/events`. `None` → `emit_event` es no-op
    /// (así estuvo hasta PRD-021-E: el bus existía y no lo llenaba nadie).
    event_emitter: Option<EventEmitter>,
    max_iterations: u32,
    default_retry_policy: RetryPolicy,
    hook_handler: Option<Arc<dyn HookHandler>>,
    checkpoint_cb: Option<Arc<dyn CheckpointCallback>>,
    pause_requested: Arc<AtomicBool>,
    /// Cancelación **cooperativa** (PRD-021-B): quien quiera parar el run
    /// levanta esta bandera y el bucle se detiene en la próxima frontera de
    /// nodo. No es un kill: el nodo en curso termina, nada queda a medias.
    cancel_requested: Arc<AtomicBool>,

    /// **Stream del cliente** (`StreamEvent`, `mpsc` por request). Lo alimenta
    /// [`GraphRunner::stream_event`]. Lo ata `stream_agent` por run y lo lee un
    /// único consumidor: quien hizo `POST /api/v1/agents/{id}/stream`.
    ///
    /// Es OTRO flujo, no una variante del anterior — ver la tabla en
    /// `server/events.rs`.
    stream_tx: Option<tokio::sync::mpsc::Sender<crate::streaming::StreamEvent>>,
}

impl GraphRunner {
    /// Create a runner with the given tool executor.
    pub fn new(executor: Box<dyn ToolExecutor>) -> Self {
        Self {
            executor: Arc::from(executor),
            event_emitter: None,
            max_iterations: 100,
            default_retry_policy: RetryPolicy::default(),
            hook_handler: None,
            checkpoint_cb: None,
            pause_requested: Arc::new(AtomicBool::new(false)),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            stream_tx: None,
        }
    }

    /// Atar el bus de observabilidad (`ExecutionEvent`). **Obligatorio para
    /// que `emit_event` haga algo**: sin esto el runner corre igual pero mudo.
    ///
    /// Call-site de producción: `AppState::new` (PRD-021-E). Cualquier host
    /// nuevo del runner que quiera ser observable tiene que llamarlo.
    pub fn with_event_emitter(mut self, emitter: EventEmitter) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Attach a streaming channel sender for real-time SSE events.
    pub fn with_stream_tx(
        mut self,
        tx: tokio::sync::mpsc::Sender<crate::streaming::StreamEvent>,
    ) -> Self {
        self.stream_tx = Some(tx);
        self
    }

    /// Override the default maximum visits per node (default: 100).
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    /// Override the default retry policy applied when a node has no explicit
    /// `retry_policy` in its config.
    pub fn with_default_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.default_retry_policy = policy;
        self
    }

    /// Attach a hook handler for execution interception.
    pub fn with_hook_handler(mut self, handler: Box<dyn HookHandler>) -> Self {
        self.hook_handler = Some(Arc::from(handler));
        self
    }

    /// Attach a checkpoint callback for state persistence.
    pub fn with_checkpoint_callback(mut self, cb: Box<dyn CheckpointCallback>) -> Self {
        self.checkpoint_cb = Some(Arc::from(cb));
        self
    }

    /// Request a pause at the next safe point (after current block finishes).
    pub fn request_pause(&self) {
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    /// Get a clone of the pause flag for external sharing.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.pause_requested.clone()
    }

    /// Pide **cancelar** el run (PRD-021-B).
    ///
    /// Cooperativo a propósito: no mata nada. El nodo que esté corriendo
    /// termina y el bucle se detiene en la siguiente frontera de nodo, deja su
    /// checkpoint y devuelve [`ExecutionStatus::Cancelled`] con el nodo en el
    /// que iba. Un kill dejaría recursos a medias (archivos a mitad de
    /// escritura, requests colgados) y ningún registro de dónde quedó.
    pub fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
    }

    /// Bandera de cancelación, para que el dueño del run (el server) la
    /// levante desde otra tarea. **Esta es la única forma de cancelar desde
    /// fuera**: el runner que corre vive dentro de la tarea de ejecución.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel_requested.clone()
    }

    /// Ata una bandera de cancelación creada afuera (builder).
    ///
    /// El server registra `session_id → flag` ANTES de arrancar el run, así el
    /// endpoint `cancel` tiene a quién levantarle la mano.
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_requested = flag;
        self
    }

    // -----------------------------------------------------------------------
    // Core execution loop
    // -----------------------------------------------------------------------

    /// Execute a graph to completion.
    ///
    /// # Algorithm
    ///
    /// 1. Validate graph structure.
    /// 2. Find entry node (first node with no incoming edges).
    /// 3. Walk the graph following edges, executing each node via the
    ///    `ToolExecutor`, until there are no more outgoing edges or an error
    ///    halts execution.
    pub async fn run(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
    ) -> Result<ExecutionResult, RunnerError> {
        self.run_with_state(graph, context, SharedState::new())
            .await
    }

    /// Run with a pre-initialized state (e.g., with memory injected).
    ///
    /// PRD-008: callers inject memory into state before calling this.
    pub async fn run_with_state(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        state: SharedState,
    ) -> Result<ExecutionResult, RunnerError> {
        // 1. Validate graph structure.
        graph.validate()?;

        // 2. Find entry node (no incoming edges).  Use first if multiple.
        let entry_nodes = graph.entry_nodes();
        let entry_node_id = entry_nodes
            .first()
            .ok_or_else(|| RunnerError::Internal {
                context: "entry node".into(),
                message: "no entry nodes found after validation".into(),
            })?
            .id
            .clone();

        self.run_from(
            graph,
            context,
            state,
            &entry_node_id,
            0,
            std::collections::HashSet::new(),
        )
        .await
    }

    /// Resume execution from a previously saved state.
    ///
    /// Starts the main loop from `resume_node_id` with the given state.
    pub async fn resume(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        resume_state: SharedState,
        resume_node_id: &str,
    ) -> Result<ExecutionResult, RunnerError> {
        self.resume_skipping(graph, context, resume_state, resume_node_id, &[], 0)
            .await
    }

    /// Reanuda **sin repetir los efectos** de lo ya ejecutado (PRD-021-C).
    ///
    /// `already_executed` son los nodos que el checkpoint registra como
    /// corridos. La garantía principal es estructural —el cursor arranca en el
    /// primer nodo NO ejecutado, así que lo anterior queda detrás— pero un
    /// grafo con ciclos puede volver a pasar por ellos. Por eso cada nodo de
    /// esa lista se **salta UNA vez**: se avanza con la salida que ya está en
    /// el estado, sin llamar a la tool. "Una vez" y no "siempre" a propósito:
    /// si el grafo tiene un bucle legítimo, la segunda vuelta sí ejecuta.
    ///
    /// `start_step` continúa la numeración de bloques del run original, para
    /// que los checkpoints del tramo reanudado no pisen la cuenta anterior.
    pub async fn resume_skipping(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        resume_state: SharedState,
        resume_node_id: &str,
        already_executed: &[String],
        start_step: u32,
    ) -> Result<ExecutionResult, RunnerError> {
        graph.validate()?;
        let skip: std::collections::HashSet<String> = already_executed.iter().cloned().collect();
        self.run_from(
            graph,
            context,
            resume_state,
            resume_node_id,
            start_step,
            skip,
        )
        .await
    }

    /// Internal: execute the main loop starting from a specific node/state.
    ///
    /// # Algorithm (readable at a glance)
    ///
    /// 1. Build node index, emit graph-started events.
    /// 2. Call on_graph_start hook (may abort).
    /// 3. Walk the cursor node-by-node:
    ///    a. Check pause / max-iterations / human-input interrupts.
    ///    b. Resolve inputs, call pre_block hook, execute with retry.
    ///    c. On success → record, advance cursor (fan-out or sequential).
    ///    d. On failure → apply failure mode (stop / skip / route-to-error).
    /// 4. Call on_graph_end hook, emit completed, return result.
    async fn run_from(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        state: SharedState,
        entry_node_id: &str,
        start_step: u32,
        mut skip_once: std::collections::HashSet<String>,
    ) -> Result<ExecutionResult, RunnerError> {
        let mut trace: Vec<TraceEntry> = Vec::new();
        let mut transcript: Vec<TranscriptEntry> = Vec::new();
        let mut visit_counts: HashMap<String, u32> = HashMap::new();
        let mut step = start_step;
        let node_idx: HashMap<&str, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut current_idx: Option<usize> = node_idx.get(entry_node_id).copied();
        let session_id = context.session_id().to_string();

        // Phase 1: Start
        self.emit_graph_started(graph, &session_id, entry_node_id, &mut transcript);

        if let Some(result) = self
            .call_hook_graph_start(graph, context, &state, &trace, &transcript)
            .await
        {
            return Ok(result);
        }

        // Phase 2: Walk the graph node by node
        while let Some(idx) = current_idx {
            let node = &graph.nodes[idx];
            let node_id = node.id.as_str();

            // --- Interrupts ---
            // Cancelación cooperativa: se mira ANTES de ejecutar nada, o sea
            // justo en la frontera entre dos nodos (PRD-021-B).
            if let Some(result) = self
                .check_cancel_interrupt(node_id, &session_id, step, &state, &trace, &transcript)
                .await
            {
                return Ok(result);
            }

            if let Some(result) = self
                .check_pause_interrupt(node_id, &session_id, step, &state, &trace, &transcript)
                .await
            {
                return Ok(result);
            }

            // --- Ya corrió antes de la pausa: NO se repite (PRD-021-C) ---
            if skip_once.remove(node_id) {
                let output = state.get(node_id).unwrap_or_default();
                debug!(node_id = %node_id, "nodo ya ejecutado antes de la pausa — se salta, no se re-ejecuta");
                transcript.push(TranscriptEntry {
                    entry_type: wk::TRANSCRIPT_DECISION.to_string(),
                    message: format!(
                        "Nodo '{node_id}' ya ejecutado antes de la pausa — no se repite"
                    ),
                    timestamp: now_ts(),
                    node_id: Some(node_id.to_string()),
                    metadata: HashMap::new(),
                });
                step += 1;
                current_idx = self
                    .resolve_next_node(node_id, &output, graph)
                    .as_deref()
                    .and_then(|nid| node_idx.get(nid).copied());
                continue;
            }

            let visits = visit_counts.entry(node.id.clone()).or_insert(0);
            *visits += 1;
            if *visits > self.max_iterations {
                self.emit_event(
                    EventType::SessionFailed,
                    &session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                return Err(RunnerError::MaxIterationsExceeded {
                    node_id: node_id.to_string(),
                    visits: *visits,
                });
            }

            if let Some(result) = self
                .check_human_input_interrupt(node, &session_id, step, &state, &trace, &transcript)
                .await
            {
                return Ok(result);
            }

            // --- Prepare inputs ---
            let mut inputs = self.resolve_and_merge_inputs(node, graph, &state);

            // --- Pre-block hook ---
            if let Some(ref hook) = self.hook_handler {
                let hr =
                    run_hook_with_timeout(hook.pre_block_exec(node, &mut inputs, context)).await;
                match hr {
                    HookResult::Abort(reason) => {
                        return Ok(self.make_failed_result(
                            state,
                            trace,
                            transcript,
                            format!("Hook pre_block_exec aborted at '{}': {}", node_id, reason),
                        ));
                    }
                    HookResult::Skip => {
                        if let Err(e) = state.set(node_id, HashMap::new(), true) {
                            error!(node_id = %node_id, error = %e, "state.set failed on hook skip");
                            return Ok(self.make_failed_result(
                                state,
                                trace,
                                transcript,
                                format!("state.set failed at '{}': {}", node_id, e),
                            ));
                        }
                        current_idx = self
                            .resolve_next_node(node_id, &HashMap::new(), graph)
                            .as_deref()
                            .and_then(|nid| node_idx.get(nid).copied());
                        step += 1;
                        continue;
                    }
                    HookResult::ModifiedInputs(new) => {
                        inputs = new;
                    }
                    _ => {}
                }
            }

            // --- Execute node ---
            self.emit_block_started(node, &session_id, &mut transcript);
            if node.tool_type.starts_with(wk::AI_TOOL_PREFIX) {
                if let Some(ref hook) = self.hook_handler {
                    let _ = run_hook_with_timeout(hook.pre_llm_call(node, context)).await;
                }
            }

            let start = Instant::now();
            let retry_policy = self.retry_policy_for(node);
            let exec_result = self
                .execute_with_retry(node, inputs, context, &retry_policy)
                .await;
            let elapsed_ms = start.elapsed().as_millis() as u64;

            // --- Handle result ---
            current_idx = match exec_result {
                Ok((output, retries)) => {
                    self.handle_node_success(
                        node,
                        output,
                        retries,
                        elapsed_ms,
                        &session_id,
                        context,
                        graph,
                        &state,
                        &mut trace,
                        &mut transcript,
                        &mut step,
                        &node_idx,
                    )
                    .await?
                }
                Err((tool_err, retries)) => {
                    let next = self
                        .handle_node_failure(
                            node,
                            idx,
                            tool_err,
                            retries,
                            elapsed_ms,
                            &session_id,
                            &retry_policy,
                            context,
                            graph,
                            &state,
                            &mut trace,
                            &mut transcript,
                            &node_idx,
                        )
                        .await;
                    match next {
                        NodeFailureOutcome::Stop(result) => return Ok(result),
                        NodeFailureOutcome::Retry => {
                            current_idx = Some(idx);
                            continue;
                        }
                        NodeFailureOutcome::Advance(next_idx) => next_idx,
                    }
                }
            };
        }

        // Phase 3: Finalize
        Ok(self
            .finalize_graph_execution(graph, context, &session_id, state, trace, transcript)
            .await)
    }

    // -----------------------------------------------------------------------
    // run_from sub-methods — each handles one concern
    // -----------------------------------------------------------------------

    fn emit_graph_started(
        &self,
        graph: &GraphDef,
        session_id: &str,
        entry_node_id: &str,
        transcript: &mut Vec<TranscriptEntry>,
    ) {
        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_STARTED.to_string(),
            message: "Execution started".to_string(),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });
        self.emit_event(EventType::SessionStarted, session_id, None, HashMap::new());
        info!(session_id = %session_id, graph_id = %graph.id, entry = %entry_node_id, "graph execution started");
        self.stream_event(crate::streaming::StreamEvent::GraphStarted {
            graph_name: graph.name.clone(),
            node_count: graph.nodes.len(),
        });
    }

    async fn call_hook_graph_start(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        state: &SharedState,
        trace: &[TraceEntry],
        transcript: &[TranscriptEntry],
    ) -> Option<ExecutionResult> {
        if let Some(ref hook) = self.hook_handler {
            if let HookResult::Abort(reason) =
                run_hook_with_timeout(hook.on_graph_start(graph, context)).await
            {
                return Some(ExecutionResult {
                    status: ExecutionStatus::Failed,
                    state: state.clone(),
                    trace: trace.to_vec(),
                    transcript: transcript.to_vec(),
                    error: Some(format!("Hook on_graph_start aborted: {}", reason)),
                    interrupt_node_id: None,
                    interrupt_info: None,
                });
            }
        }
        None
    }

    async fn check_pause_interrupt(
        &self,
        node_id: &str,
        session_id: &str,
        step: u32,
        state: &SharedState,
        trace: &[TraceEntry],
        transcript: &[TranscriptEntry],
    ) -> Option<ExecutionResult> {
        if self.pause_requested.swap(false, Ordering::SeqCst) {
            self.save_checkpoint(session_id, step, node_id, state, Some(node_id), trace)
                .await;
            self.emit_event(
                EventType::SessionInterrupted,
                session_id,
                Some(node_id),
                HashMap::new(),
            );
            return Some(ExecutionResult {
                status: ExecutionStatus::Interrupted,
                state: state.clone(),
                trace: trace.to_vec(),
                transcript: transcript.to_vec(),
                error: None,
                interrupt_node_id: Some(node_id.to_string()),
                interrupt_info: None,
            });
        }
        None
    }

    /// ¿Alguien pidió cancelar? Si sí, se para AQUÍ (frontera de nodo), se deja
    /// checkpoint y se devuelve `Cancelled` con el nodo en el que iba.
    ///
    /// A diferencia del pause genérico, la bandera **no se limpia**: cancelar
    /// es terminal, y si el flujo volviera a pasar por acá tendría que parar
    /// otra vez.
    async fn check_cancel_interrupt(
        &self,
        node_id: &str,
        session_id: &str,
        step: u32,
        state: &SharedState,
        trace: &[TraceEntry],
        transcript: &[TranscriptEntry],
    ) -> Option<ExecutionResult> {
        if !self.cancel_requested.load(Ordering::SeqCst) {
            return None;
        }
        self.save_checkpoint(session_id, step, node_id, state, Some(node_id), trace)
            .await;
        self.emit_event(
            EventType::SessionInterrupted,
            session_id,
            Some(node_id),
            HashMap::new(),
        );
        info!(session_id = %session_id, node_id = %node_id, "run cancelado en frontera de nodo");
        let mut transcript = transcript.to_vec();
        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_ERROR.to_string(),
            message: format!("Ejecución cancelada antes del nodo '{node_id}'"),
            timestamp: now_ts(),
            node_id: Some(node_id.to_string()),
            metadata: HashMap::new(),
        });
        Some(ExecutionResult {
            status: ExecutionStatus::Cancelled,
            state: state.clone(),
            trace: trace.to_vec(),
            transcript,
            error: None,
            interrupt_node_id: Some(node_id.to_string()),
            interrupt_info: None,
        })
    }

    async fn check_human_input_interrupt(
        &self,
        node: &NodeDef,
        session_id: &str,
        step: u32,
        state: &SharedState,
        trace: &[TraceEntry],
        transcript: &[TranscriptEntry],
    ) -> Option<ExecutionResult> {
        if node.tool_type != wk::HUMAN_INPUT_TOOL {
            return None;
        }
        let prompt = node
            .config
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Decision required")
            .to_string();
        let options: Vec<String> = node
            .config
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let timeout_minutes = node.config.get("timeout_minutes").and_then(|v| v.as_f64());

        let info = InterruptInfo {
            node_id: node.id.clone(),
            prompt,
            options,
            timeout_minutes,
        };

        self.save_checkpoint(session_id, step, &node.id, state, Some(&node.id), trace)
            .await;
        self.emit_event(
            EventType::InterruptCreated,
            session_id,
            Some(&node.id),
            HashMap::new(),
        );

        // PRD-021-C: `Paused`, no `Interrupted`. El run NO murió — está
        // esperando a un humano y su estado quedó en disco, así que
        // `POST /sessions/{id}/resume` lo continúa desde el nodo siguiente.
        Some(ExecutionResult {
            status: ExecutionStatus::Paused,
            state: state.clone(),
            trace: trace.to_vec(),
            transcript: transcript.to_vec(),
            error: None,
            interrupt_node_id: Some(node.id.clone()),
            interrupt_info: Some(info),
        })
    }

    fn resolve_and_merge_inputs(
        &self,
        node: &NodeDef,
        graph: &GraphDef,
        state: &SharedState,
    ) -> HashMap<String, Value> {
        let mut inputs = self.resolve_inputs(&node.id, graph, state);
        for (key, val) in &node.config {
            inputs.entry(key.clone()).or_insert_with(|| val.clone());
        }
        debug!(
            node_id = %node.id, tool_type = %node.tool_type,
            input_keys = ?inputs.keys().collect::<Vec<_>>(), "executing node"
        );
        inputs
    }

    fn emit_block_started(
        &self,
        node: &NodeDef,
        session_id: &str,
        transcript: &mut Vec<TranscriptEntry>,
    ) {
        self.emit_event(
            EventType::BlockStarted,
            session_id,
            Some(&node.id),
            HashMap::new(),
        );
        self.stream_event(crate::streaming::StreamEvent::NodeStarted {
            node_id: node.id.clone(),
            tool_type: node.tool_type.clone(),
        });
        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_BLOCK_START.to_string(),
            message: format!("Executing {}", node.tool_type),
            timestamp: now_ts(),
            node_id: Some(node.id.clone()),
            metadata: HashMap::new(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_node_success(
        &self,
        node: &NodeDef,
        mut output: HashMap<String, Value>,
        retries: u32,
        elapsed_ms: u64,
        session_id: &str,
        context: &dyn ExecutionContext,
        graph: &GraphDef,
        state: &SharedState,
        trace: &mut Vec<TraceEntry>,
        transcript: &mut Vec<TranscriptEntry>,
        step: &mut u32,
        node_idx: &HashMap<&str, usize>,
    ) -> Result<Option<usize>, RunnerError> {
        let node_id = node.id.as_str();

        // Hook: post_llm_call
        if node.tool_type.starts_with(wk::AI_TOOL_PREFIX) {
            if let Some(ref hook) = self.hook_handler {
                let mut rv = serde_json::to_value(&output).unwrap_or(Value::Null);
                let _ = run_hook_with_timeout(hook.post_llm_call(node, &mut rv, context)).await;
            }
        }

        // Store output + trace — fail loudly if state can't persist
        if let Err(e) = state.set(node_id, output.clone(), true) {
            error!(node_id = %node_id, error = %e, "state.set failed — aborting execution");
            return Err(RunnerError::Internal {
                context: format!("state.set at node '{}'", node_id),
                message: e.to_string(),
            });
        }
        let finished_at = now_ts();
        trace.push(TraceEntry {
            node_id: node_id.to_string(),
            tool_type: node.tool_type.clone(),
            status: TraceStatus::Ok,
            duration_ms: elapsed_ms,
            retries,
            started_at: finished_at - (elapsed_ms as f64 / 1000.0),
            finished_at,
            error: None,
        });

        // Hook: post_block_exec
        if let Some(ref hook) = self.hook_handler {
            let _ = run_hook_with_timeout(hook.post_block_exec(node, &mut output, context)).await;
        }

        // Events
        self.emit_event(
            EventType::BlockCompleted,
            session_id,
            Some(node_id),
            HashMap::new(),
        );
        self.stream_event(crate::streaming::StreamEvent::NodeCompleted {
            node_id: node_id.to_string(),
            tool_type: node.tool_type.clone(),
            duration_ms: elapsed_ms,
            output_keys: output.keys().cloned().collect(),
            output: serde_json::to_value(&output).unwrap_or(serde_json::Value::Null),
        });
        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_BLOCK_END.to_string(),
            message: format!("Completed {} in {}ms", node.tool_type, elapsed_ms),
            timestamp: now_ts(),
            node_id: Some(node_id.to_string()),
            metadata: HashMap::new(),
        });

        *step += 1;

        // Advance cursor: fan-out or sequential
        let next_nodes = self.resolve_all_next_nodes(node_id, &output, graph);

        if next_nodes.len() > 1 {
            debug!(from = %node_id, targets = ?next_nodes,
                "fan-out detected — executing {} nodes in parallel", next_nodes.len());
            let (fo_trace, fo_transcript) = self
                .execute_fanout(&next_nodes, graph, context, state, session_id)
                .await?;
            trace.extend(fo_trace);
            transcript.extend(fo_transcript);
            let join_id = self.find_fanout_join(&next_nodes, graph);
            self.save_checkpoint(session_id, *step, node_id, state, join_id.as_deref(), trace)
                .await;
            Ok(join_id
                .as_deref()
                .and_then(|nid| node_idx.get(nid).copied()))
        } else {
            let next = next_nodes.into_iter().next();
            self.save_checkpoint(session_id, *step, node_id, state, next.as_deref(), trace)
                .await;
            self.record_decision_transcript(node_id, &next, graph, transcript);
            Ok(next.as_deref().and_then(|nid| node_idx.get(nid).copied()))
        }
    }

    fn record_decision_transcript(
        &self,
        node_id: &str,
        next: &Option<String>,
        graph: &GraphDef,
        transcript: &mut Vec<TranscriptEntry>,
    ) {
        let outgoing = graph.outgoing_edges(node_id);
        let conditional: Vec<_> = outgoing.iter().filter(|e| e.condition.is_some()).collect();
        if conditional.len() > 1 {
            if let Some(ref next_id) = next {
                let edge_id = outgoing
                    .iter()
                    .find(|e| e.target == *next_id)
                    .map(|e| e.id.as_str())
                    .unwrap_or("unknown");
                let cond_desc = outgoing
                    .iter()
                    .find(|e| e.target == *next_id)
                    .and_then(|e| e.condition.as_ref())
                    .map(|c| format!("{} {:?} {}", c.field, c.op, c.value))
                    .unwrap_or_default();
                transcript.push(TranscriptEntry {
                    entry_type: wk::TRANSCRIPT_DECISION.to_string(),
                    message: format!(
                        "Decision: following edge {} (condition: {})",
                        edge_id, cond_desc
                    ),
                    timestamp: now_ts(),
                    node_id: Some(node_id.to_string()),
                    metadata: HashMap::new(),
                });
            }
        }
        if let Some(ref nid) = next {
            debug!(from = %node_id, to = %nid, "advancing to next node");
        } else {
            debug!(from = %node_id, "no outgoing edge — end of graph");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_node_failure(
        &self,
        node: &NodeDef,
        _idx: usize,
        tool_err: ToolError,
        retries: u32,
        elapsed_ms: u64,
        session_id: &str,
        retry_policy: &RetryPolicy,
        context: &dyn ExecutionContext,
        graph: &GraphDef,
        state: &SharedState,
        trace: &mut Vec<TraceEntry>,
        transcript: &mut Vec<TranscriptEntry>,
        node_idx: &HashMap<&str, usize>,
    ) -> NodeFailureOutcome {
        let node_id = node.id.as_str();
        let err_msg = tool_err.to_string();

        // Hook: on_error
        let hook_action = if let Some(ref hook) = self.hook_handler {
            run_hook_with_timeout(hook.on_error(node, &tool_err, context)).await
        } else {
            HookResult::Continue
        };

        if matches!(hook_action, HookResult::Retry) {
            transcript.push(TranscriptEntry {
                entry_type: wk::TRANSCRIPT_ERROR.to_string(),
                message: format!(
                    "Error in {}: {} (retrying via hook)",
                    node.tool_type, err_msg
                ),
                timestamp: now_ts(),
                node_id: Some(node_id.to_string()),
                metadata: HashMap::new(),
            });
            return NodeFailureOutcome::Retry;
        }

        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_ERROR.to_string(),
            message: format!("Error in {}: {}", node.tool_type, err_msg),
            timestamp: now_ts(),
            node_id: Some(node_id.to_string()),
            metadata: HashMap::new(),
        });

        match retry_policy.on_failure {
            FailureMode::Stop => {
                let finished_at = now_ts();
                trace.push(TraceEntry {
                    node_id: node_id.to_string(),
                    tool_type: node.tool_type.clone(),
                    status: TraceStatus::Error,
                    duration_ms: elapsed_ms,
                    retries,
                    started_at: finished_at - (elapsed_ms as f64 / 1000.0),
                    finished_at,
                    error: Some(err_msg.clone()),
                });
                self.emit_event(
                    EventType::BlockError,
                    session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                self.emit_event(EventType::SessionFailed, session_id, None, HashMap::new());
                error!(node_id = %node_id, error = %err_msg, "node execution failed — stopping");
                NodeFailureOutcome::Stop(ExecutionResult {
                    status: ExecutionStatus::Failed,
                    state: state.clone(),
                    trace: trace.clone(),
                    transcript: transcript.clone(),
                    error: Some(err_msg),
                    interrupt_node_id: None,
                    interrupt_info: None,
                })
            }
            FailureMode::Skip => {
                warn!(node_id = %node_id, error = %err_msg, "node execution failed — skipping");
                let empty: HashMap<String, Value> = HashMap::new();
                if let Err(e) = state.set(node_id, empty.clone(), true) {
                    error!(node_id = %node_id, error = %e, "state.set failed on skip — stopping");
                    return NodeFailureOutcome::Stop(ExecutionResult {
                        status: ExecutionStatus::Failed,
                        state: state.clone(),
                        trace: trace.clone(),
                        transcript: transcript.clone(),
                        error: Some(format!("state.set failed at '{}': {}", node_id, e)),
                        interrupt_node_id: None,
                        interrupt_info: None,
                    });
                }
                let finished_at = now_ts();
                trace.push(TraceEntry {
                    node_id: node_id.to_string(),
                    tool_type: node.tool_type.clone(),
                    status: TraceStatus::Skipped,
                    duration_ms: elapsed_ms,
                    retries,
                    started_at: finished_at - (elapsed_ms as f64 / 1000.0),
                    finished_at,
                    error: Some(err_msg),
                });
                self.emit_event(
                    EventType::BlockCompleted,
                    session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                let next = self
                    .resolve_next_node(node_id, &empty, graph)
                    .as_deref()
                    .and_then(|nid| node_idx.get(nid).copied());
                NodeFailureOutcome::Advance(next)
            }
            FailureMode::RouteToError => {
                warn!(node_id = %node_id, error = %err_msg, "node execution failed — routing to error path");
                let mut err_output: HashMap<String, Value> = HashMap::new();
                err_output.insert(wk::ERROR_FIELD.to_string(), Value::String(err_msg.clone()));
                if let Err(e) = state.set(node_id, err_output.clone(), true) {
                    error!(node_id = %node_id, error = %e, "state.set failed on error route — stopping");
                    return NodeFailureOutcome::Stop(ExecutionResult {
                        status: ExecutionStatus::Failed,
                        state: state.clone(),
                        trace: trace.clone(),
                        transcript: transcript.clone(),
                        error: Some(format!("state.set failed at '{}': {}", node_id, e)),
                        interrupt_node_id: None,
                        interrupt_info: None,
                    });
                }
                let finished_at = now_ts();
                trace.push(TraceEntry {
                    node_id: node_id.to_string(),
                    tool_type: node.tool_type.clone(),
                    status: TraceStatus::Error,
                    duration_ms: elapsed_ms,
                    retries,
                    started_at: finished_at - (elapsed_ms as f64 / 1000.0),
                    finished_at,
                    error: Some(err_msg),
                });
                self.emit_event(
                    EventType::BlockError,
                    session_id,
                    Some(node_id),
                    HashMap::new(),
                );
                let next = self
                    .resolve_next_node(node_id, &err_output, graph)
                    .as_deref()
                    .and_then(|nid| node_idx.get(nid).copied());
                NodeFailureOutcome::Advance(next)
            }
        }
    }

    fn make_failed_result(
        &self,
        state: SharedState,
        trace: Vec<TraceEntry>,
        transcript: Vec<TranscriptEntry>,
        error: String,
    ) -> ExecutionResult {
        ExecutionResult {
            status: ExecutionStatus::Failed,
            state,
            trace,
            transcript,
            error: Some(error),
            interrupt_node_id: None,
            interrupt_info: None,
        }
    }

    async fn finalize_graph_execution(
        &self,
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        session_id: &str,
        state: SharedState,
        trace: Vec<TraceEntry>,
        mut transcript: Vec<TranscriptEntry>,
    ) -> ExecutionResult {
        if let Some(ref hook) = self.hook_handler {
            let _ = run_hook_with_timeout(hook.on_graph_end(graph, &state, context)).await;
        }
        transcript.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_COMPLETED.to_string(),
            message: format!("Execution completed ({} blocks)", trace.len()),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });
        self.emit_event(
            EventType::SessionCompleted,
            session_id,
            None,
            HashMap::new(),
        );
        let total_ms: u64 = trace.iter().map(|t| t.duration_ms).sum();
        self.stream_event(crate::streaming::StreamEvent::GraphCompleted {
            status: format!("{:?}", ExecutionStatus::Completed),
            total_duration_ms: total_ms,
            nodes_executed: trace.len(),
            output: serde_json::to_value(state.snapshot()).unwrap_or(serde_json::Value::Null),
        });
        info!(session_id = %session_id, nodes_executed = trace.len(), "graph execution completed");
        ExecutionResult {
            status: ExecutionStatus::Completed,
            state,
            trace,
            transcript,
            error: None,
            interrupt_node_id: None,
            interrupt_info: None,
        }
    }

    /// Nodos ya ejecutados, en orden y sin repetir, leídos de la traza.
    ///
    /// `Ok` y `Skipped` cuentan como ejecutados: ambos ya escribieron su salida
    /// en el estado y sus efectos ya ocurrieron — reanudar no debe repetirlos.
    /// `Error` NO cuenta: ese nodo es justo por donde hay que retomar.
    fn executed_node_ids(trace: &[TraceEntry]) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        trace
            .iter()
            .filter(|t| matches!(t.status, TraceStatus::Ok | TraceStatus::Skipped))
            .filter(|t| seen.insert(t.node_id.clone()))
            .map(|t| t.node_id.clone())
            .collect()
    }

    /// Internal: save checkpoint if callback is configured.
    async fn save_checkpoint(
        &self,
        session_id: &str,
        step: u32,
        node_id: &str,
        state: &SharedState,
        cursor_node_id: Option<&str>,
        trace: &[TraceEntry],
    ) {
        if let Some(ref cb) = self.checkpoint_cb {
            let checkpoint = Checkpoint {
                session_id: session_id.to_string(),
                step,
                node_id: node_id.to_string(),
                state_snapshot: state.snapshot(),
                cursor_node_id: cursor_node_id.map(String::from),
                executed_nodes: Self::executed_node_ids(trace),
                timestamp: now_ts(),
            };
            match cb.save_checkpoint(checkpoint).await {
                Ok(checkpoint_id) => {
                    self.emit_event(EventType::CheckpointCreated, session_id, Some(node_id), {
                        let mut data = HashMap::new();
                        data.insert("checkpoint_id".to_string(), Value::String(checkpoint_id));
                        data.insert(
                            "step".to_string(),
                            Value::Number(serde_json::Number::from(step)),
                        );
                        data
                    });
                }
                Err(e) => {
                    error!(error = %e, node_id = %node_id, "checkpoint save failed — session may not be resumable");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Input resolution
    // -----------------------------------------------------------------------

    /// Resolve inputs for a target node by inspecting all incoming edges and
    /// their `data_map` definitions.
    ///
    /// Two expression forms are supported:
    ///
    /// * **Direct ref** — `"source_node.field"`: returns the raw `Value` from
    ///   the source node's output in state.
    /// * **Template** — a string containing `${source_node.field}` markers:
    ///   each marker is replaced with the stringified value and the whole
    ///   expression is returned as `Value::String`.
    fn resolve_inputs(
        &self,
        node_id: &str,
        graph: &GraphDef,
        state: &SharedState,
    ) -> HashMap<String, Value> {
        let mut inputs = HashMap::new();

        // Collect incoming edges for this node.
        let incoming: Vec<_> = graph.edges.iter().filter(|e| e.target == node_id).collect();

        for edge in incoming {
            if let Some(ref data_map) = edge.data_map {
                for (target_param, source_expr) in data_map {
                    if let Some(val) = self.resolve_expression(source_expr, state) {
                        inputs.insert(target_param.clone(), val);
                    }
                }
            }
        }

        inputs
    }

    /// Resolve a single data_map expression against the current shared state.
    pub(crate) fn resolve_expression(&self, expr: &str, state: &SharedState) -> Option<Value> {
        // Static regex compiled once via LazyLock (was Regex::new per-call before).
        static TEMPLATE_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\$\{([^.}]+)\.([^}]+)\}").expect("valid regex"));
        let template_re = &*TEMPLATE_RE;

        if template_re.is_match(expr) {
            // Template mode: replace all ${node.field} occurrences.
            let mut result = expr.to_string();
            // Iterate over all captures (we re-find because replace needs owned data).
            for caps in template_re.captures_iter(expr) {
                let full_match = caps.get(0).unwrap().as_str();
                let src_node = &caps[1];
                let src_field = &caps[2];
                if let Some(val) = state.get_field(src_node, src_field) {
                    let replacement = match &val {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    result = result.replace(full_match, &replacement);
                }
            }
            Some(Value::String(result))
        } else if let Some((src_node, src_field)) = expr.split_once('.') {
            // Direct ref: "node_id.field"
            state.get_field(src_node, src_field)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Edge / condition evaluation
    // -----------------------------------------------------------------------

    /// Determine the next node by evaluating outgoing edges from `node_id`.
    ///
    /// 1. First conditional edge whose condition matches wins.
    /// 2. Fallback: first unconditional edge.
    /// 3. `None` if no outgoing edges (end of graph).
    fn resolve_next_node(
        &self,
        node_id: &str,
        output: &HashMap<String, Value>,
        graph: &GraphDef,
    ) -> Option<String> {
        let targets = self.resolve_all_next_nodes(node_id, output, graph);
        targets.into_iter().next()
    }

    /// Returns ALL next nodes. If multiple unconditional edges exist, returns
    /// all of them (fan-out). If conditional edges match, returns only the first
    /// matching one (conditions are mutually exclusive).
    pub(crate) fn resolve_all_next_nodes(
        &self,
        node_id: &str,
        output: &HashMap<String, Value>,
        graph: &GraphDef,
    ) -> Vec<String> {
        let outgoing = graph.outgoing_edges(node_id);

        if outgoing.is_empty() {
            return vec![];
        }

        // First pass: conditional edges — return first match (exclusive routing).
        for edge in &outgoing {
            if let Some(ref condition) = edge.condition {
                if Self::evaluate_condition(output, condition) {
                    return vec![edge.target.clone()];
                }
            }
        }

        // Second pass: collect ALL unconditional edges.
        let unconditional: Vec<String> = outgoing
            .iter()
            .filter(|e| e.condition.is_none())
            .map(|e| e.target.clone())
            .collect();

        unconditional
    }

    /// Execute multiple nodes in parallel (fan-out) using concurrent futures.
    ///
    /// Uses `futures_util::future::join_all` for IO-concurrent execution of all
    /// parallel nodes. Results are stored in SharedState under each node's ID.
    async fn execute_fanout(
        &self,
        node_ids: &[String],
        graph: &GraphDef,
        context: &dyn ExecutionContext,
        state: &SharedState,
        session_id: &str,
    ) -> Result<(Vec<TraceEntry>, Vec<TranscriptEntry>), RunnerError> {
        let mut trace_entries = Vec::new();
        let mut transcript_entries = Vec::new();

        transcript_entries.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_FANOUT_START.to_string(),
            message: format!(
                "Fan-out: executing {} nodes in parallel: [{}]",
                node_ids.len(),
                node_ids.join(", ")
            ),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });

        self.emit_event(EventType::BlockStarted, session_id, None, {
            let mut m = HashMap::new();
            m.insert(
                "fanout_nodes".to_string(),
                Value::String(node_ids.join(",")),
            );
            m
        });

        // Prepare concurrent futures — one per parallel node.
        let futures: Vec<_> = node_ids
            .iter()
            .filter_map(|nid| {
                let node = graph.nodes.iter().find(|n| n.id == *nid)?;
                // NOTE: an earlier version forked the state here (PLAN_000 / Milestone 1),
                // but the executor never receives the fork (`ToolExecutor::execute` only
                // takes inputs + context), so fork/merge was a pure O(state) deep-clone
                // with no real isolation. Inputs are resolved read-only against the shared
                // state *before* any fanout node runs, which is equivalent and cheaper.
                let mut inputs = self.resolve_inputs(nid, graph, state);
                for (key, val) in &node.config {
                    inputs.entry(key.clone()).or_insert_with(|| val.clone());
                }
                let executor = &self.executor;
                Some(async move {
                    let start = Instant::now();
                    let result = executor.execute(node, inputs, context).await;
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    // Fin real capturado DENTRO del branch concurrente: los spans
                    // paralelos quedan con tiempos solapados fieles (no en serie).
                    let finished_at = now_ts();
                    (
                        node.id.clone(),
                        node.tool_type.clone(),
                        result,
                        elapsed_ms,
                        finished_at,
                    )
                })
            })
            .collect();

        // Execute all concurrently via join_all (IO-concurrent, borrows context).
        let results = futures_util::future::join_all(futures).await;

        // Collect results.
        let mut failed_count = 0usize;
        for (node_id, tool_type, exec_result, elapsed_ms, finished_at) in results {
            let started_at = finished_at - (elapsed_ms as f64 / 1000.0);
            match exec_result {
                Ok(output) => {
                    if let Err(e) = state.set(&node_id, output, true) {
                        error!(node_id = %node_id, error = %e, "fanout: state.set failed");
                        failed_count += 1;
                        trace_entries.push(TraceEntry {
                            node_id: node_id.clone(),
                            tool_type: tool_type.clone(),
                            status: TraceStatus::Error,
                            duration_ms: elapsed_ms,
                            retries: 0,
                            started_at,
                            finished_at,
                            error: Some(format!("state.set failed: {}", e)),
                        });
                        continue;
                    }

                    trace_entries.push(TraceEntry {
                        node_id: node_id.clone(),
                        tool_type: tool_type.clone(),
                        status: TraceStatus::Ok,
                        duration_ms: elapsed_ms,
                        retries: 0,
                        started_at,
                        finished_at,
                        error: None,
                    });

                    transcript_entries.push(TranscriptEntry {
                        entry_type: wk::TRANSCRIPT_FANOUT_NODE_DONE.to_string(),
                        message: format!(
                            "Fan-out node '{}' ({}) completed in {}ms",
                            node_id, tool_type, elapsed_ms
                        ),
                        timestamp: now_ts(),
                        node_id: Some(node_id),
                        metadata: HashMap::new(),
                    });
                }
                Err(tool_err) => {
                    failed_count += 1;
                    trace_entries.push(TraceEntry {
                        node_id: node_id.clone(),
                        tool_type: tool_type.clone(),
                        status: TraceStatus::Error,
                        duration_ms: elapsed_ms,
                        retries: 0,
                        started_at,
                        finished_at,
                        error: Some(tool_err.to_string()),
                    });

                    transcript_entries.push(TranscriptEntry {
                        entry_type: wk::TRANSCRIPT_FANOUT_NODE_ERROR.to_string(),
                        message: format!("Fan-out node '{}' failed: {}", node_id, tool_err),
                        timestamp: now_ts(),
                        node_id: Some(node_id),
                        metadata: HashMap::new(),
                    });
                }
            }
        }

        transcript_entries.push(TranscriptEntry {
            entry_type: wk::TRANSCRIPT_FANOUT_END.to_string(),
            message: format!(
                "Fan-out completed: {} succeeded, {} failed",
                node_ids.len() - failed_count,
                failed_count
            ),
            timestamp: now_ts(),
            node_id: None,
            metadata: HashMap::new(),
        });

        self.emit_event(EventType::BlockCompleted, session_id, None, HashMap::new());

        Ok((trace_entries, transcript_entries))
    }

    /// Find the join node after a fan-out: the common successor of all
    /// parallel nodes. Returns None if they don't converge.
    fn find_fanout_join(&self, parallel_node_ids: &[String], graph: &GraphDef) -> Option<String> {
        if parallel_node_ids.is_empty() {
            return None;
        }

        // For each parallel node, find its unconditional targets.
        let mut successor_sets: Vec<std::collections::HashSet<String>> = Vec::new();
        for nid in parallel_node_ids {
            let targets: std::collections::HashSet<String> = graph
                .outgoing_edges(nid)
                .iter()
                .filter(|e| e.condition.is_none())
                .map(|e| e.target.clone())
                .collect();
            successor_sets.push(targets);
        }

        // Find the intersection — nodes that ALL parallel paths lead to.
        if let Some(first) = successor_sets.first() {
            let common: std::collections::HashSet<String> = first
                .iter()
                .filter(|n| successor_sets.iter().all(|s| s.contains(*n)))
                .cloned()
                .collect();
            common.into_iter().next()
        } else {
            None
        }
    }

    /// Evaluate a single edge condition against a node's output map.
    pub(crate) fn evaluate_condition(
        output: &HashMap<String, Value>,
        condition: &EdgeCondition,
    ) -> bool {
        let actual = match output.get(&condition.field) {
            Some(v) => v,
            None => return false,
        };
        let expected = &condition.value;

        match condition.op {
            ComparisonOp::Eq => actual == expected,
            ComparisonOp::Neq => actual != expected,
            ComparisonOp::Gt => compare_numbers(actual, expected, |a, b| a > b),
            ComparisonOp::Lt => compare_numbers(actual, expected, |a, b| a < b),
            ComparisonOp::Gte => compare_numbers(actual, expected, |a, b| a >= b),
            ComparisonOp::Lte => compare_numbers(actual, expected, |a, b| a <= b),
            ComparisonOp::In => {
                // `actual` is IN `expected` (expected must be an array).
                match expected {
                    Value::Array(arr) => arr.contains(actual),
                    _ => false,
                }
            }
            ComparisonOp::Contains => {
                // `actual` string/array contains `expected`.
                match actual {
                    Value::String(s) => match expected {
                        Value::String(sub) => s.contains(sub.as_str()),
                        _ => false,
                    },
                    Value::Array(arr) => arr.contains(expected),
                    _ => false,
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Retry logic
    // -----------------------------------------------------------------------

    /// Execute a node, retrying according to the given policy.
    ///
    /// Returns `Ok((output, retries_used))` on success, or
    /// `Err((last_error, retries_used))` after all attempts are exhausted.
    async fn execute_with_retry(
        &self,
        node: &NodeDef,
        inputs: HashMap<String, Value>,
        context: &dyn ExecutionContext,
        policy: &RetryPolicy,
    ) -> Result<(HashMap<String, Value>, u32), (ToolError, u32)> {
        let total_attempts = 1 + policy.max_retries;
        let mut last_error: Option<ToolError> = None;

        for attempt in 0..total_attempts {
            if attempt > 0 {
                let delay = Self::calculate_backoff(attempt, policy);
                if !delay.is_zero() {
                    debug!(
                        node_id = %node.id,
                        attempt = attempt,
                        delay_ms = delay.as_millis() as u64,
                        "retrying after backoff"
                    );
                    tokio::time::sleep(delay).await;
                }
            }

            match self.executor.execute(node, inputs.clone(), context).await {
                Ok(output) => return Ok((output, attempt)),
                Err(e) => {
                    warn!(
                        node_id = %node.id,
                        attempt = attempt + 1,
                        total = total_attempts,
                        error = %e,
                        "tool execution attempt failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err((
            last_error.expect("at least one attempt must have been made"),
            policy.max_retries,
        ))
    }

    /// Calculate the backoff duration for a given retry attempt number.
    ///
    /// * `None` — always zero.
    /// * `Linear` — `initial_delay * attempt`.
    /// * `Exponential` — `initial_delay * 2^(attempt-1)`.
    pub(crate) fn calculate_backoff(retry: u32, policy: &RetryPolicy) -> Duration {
        let secs = match policy.backoff {
            BackoffStrategy::None => 0.0,
            BackoffStrategy::Linear => policy.initial_delay_secs * f64::from(retry),
            BackoffStrategy::Exponential => {
                policy.initial_delay_secs * 2.0_f64.powi(retry.saturating_sub(1) as i32)
            }
        };
        Duration::from_secs_f64(secs)
    }

    /// Look up a node's retry policy from its config map. Falls back to the
    /// runner-level default when not present or unparseable.
    fn retry_policy_for(&self, node: &NodeDef) -> RetryPolicy {
        node.config
            .get("retry_policy")
            .and_then(|v| serde_json::from_value::<RetryPolicy>(v.clone()).ok())
            .unwrap_or_else(|| self.default_retry_policy.clone())
    }

    // -----------------------------------------------------------------------
    // Event helper
    // -----------------------------------------------------------------------

    /// Publica en el **bus de observabilidad** (`ExecutionEvent`, broadcast de
    /// proceso) — el que ve `GET /api/v1/events`. Sin emisor atado es no-op.
    ///
    /// Aquí nacen los eventos que NO tienen equivalente en el stream del
    /// cliente: `checkpoint_created`, `interrupt_created`,
    /// `session_interrupted`, `session_started/failed`, `block_*`.
    ///
    /// Su gemelo es [`GraphRunner::stream_event`] — otro canal, otro
    /// consumidor. No los unifiques sin leer `server/events.rs`.
    fn emit_event(
        &self,
        event_type: EventType,
        session_id: &str,
        node_id: Option<&str>,
        data: HashMap<String, Value>,
    ) {
        if let Some(ref emitter) = self.event_emitter {
            emitter.emit(
                event_type,
                session_id.to_string(),
                node_id.map(String::from),
                data,
            );
        }
    }

    /// Publica en el **stream del cliente** (`StreamEvent`, mpsc por request)
    /// — el que ve `POST /api/v1/agents/{id}/stream`. No bloquea; avisa si se
    /// cae un evento por canal lleno o cerrado.
    ///
    /// Contrato público desde 0.7.0: los nombres y el sobre no se cambian.
    /// Para observabilidad nueva usa [`GraphRunner::emit_event`], que va al
    /// otro bus y es aditivo.
    fn stream_event(&self, event: crate::streaming::StreamEvent) {
        if let Some(ref tx) = self.stream_tx {
            if let Err(e) = tx.try_send(event) {
                warn!("stream event dropped (channel full or closed): {}", e);
            }
        }
    }
}
