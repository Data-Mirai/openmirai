//! Ciclo de vida de un run: reanudar y cancelar (PRD-021-B/C).
//!
//! Hasta 0.7.0 `GraphRunner::resume()` existía, estaba testeado y **no tenía un
//! solo call-site de producción**: un run que se detenía esperando a un humano
//! moría ahí. Estos dos endpoints son ese call-site.
//!
//! - `POST /api/v1/sessions/{id}/resume` — acepta la respuesta humana y sigue
//!   **desde el nodo siguiente**, sin repetir lo ya hecho.
//! - `POST /api/v1/sessions/{id}/cancel` — levanta la bandera de cancelación
//!   del run en vuelo; el runner se detiene en la próxima frontera de nodo.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::{DefaultExecutionContext, InMemoryDBResource, InMemoryStorageResource};
use crate::core::agent_spec::AgentSpec;
use crate::core::graph::GraphDef;
use crate::core::runner::ExecutionResult;
use crate::core::well_known as wk;
use crate::db::{CheckpointRecord, Repository, SessionRecord, SessionStatus, SqliteSessionRepo};

use super::state::{AppState, ErrorResponse};

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Cuerpo de `POST /sessions/{id}/resume`.
///
/// Todo opcional: reanudar un run **fallido** no necesita respuesta humana
/// (W5), solo reanudar uno **pausado** en un `logic/human_input` la usa (W2).
#[derive(Debug, Default, Deserialize)]
pub struct ResumeRequest {
    /// Lo que contestó el humano. Se guarda como salida del nodo que preguntó.
    #[serde(default)]
    pub response: Option<Value>,
    /// Quién contestó (auditoría). Por defecto `"human"`.
    #[serde(default)]
    pub responded_by: Option<String>,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/sessions/{id}/cancel
// ---------------------------------------------------------------------------

/// Cancela un run **en vuelo**. Cooperativo: no mata nada.
///
/// Responde `202 Accepted` en cuanto la bandera queda levantada — el run se
/// detiene solo, en la siguiente frontera de nodo, y ahí queda registrado como
/// `cancelled` con el nodo en el que iba. Devolver `200` sería mentir: cuando
/// el cliente lee la respuesta el run todavía está terminando su nodo.
pub(crate) async fn cancel_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    if state.request_cancel(&id).await {
        return Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "session_id": id,
                "status": "cancelling",
                "detail": "cancelación cooperativa: el run se detiene en la próxima frontera de nodo",
            })),
        ));
    }

    // No está en vuelo AQUÍ: o no existe, o ya terminó, o corre en otro proceso.
    let repo = state.session_repo.as_ref();
    let registrado = match repo {
        Some(r) => r.get(&id).await.ok().flatten(),
        None => None,
    };
    match registrado {
        Some(rec) => Err(err(
            StatusCode::CONFLICT,
            format!(
                "el run '{id}' no está en vuelo en este proceso (estado: {})",
                rec.status
            ),
        )),
        None => Err(err(StatusCode::NOT_FOUND, format!("run '{id}' no existe"))),
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/sessions/{id}/resume
// ---------------------------------------------------------------------------

/// Reanuda un run detenido, desde donde quedó y sin repetir lo ya hecho.
///
/// Camino: checkpoint (PRD-021-A) → spec del agente → estado rearmado →
/// `GraphRunner::resume_skipping()` desde el cursor guardado.
pub(crate) async fn resume_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    cuerpo: Option<Json<ResumeRequest>>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let req = cuerpo.map(|Json(r)| r).unwrap_or_default();

    let repo = state.session_repo.clone().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "reanudar necesita persistencia de runs y este server arrancó sin DB",
        )
    })?;

    // 1. ¿El run existe y está en un estado reanudable?
    let registro = repo
        .get(&id)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("run '{id}' no existe")))?;
    if !registro.status.is_resumable() {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "el run '{id}' no se puede reanudar (estado: {})",
                registro.status
            ),
        ));
    }

    // 2. Su estado guardado: sin checkpoint no hay desde dónde retomar.
    let cp = repo
        .get_checkpoint(&id)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            err(
                StatusCode::CONFLICT,
                format!("el run '{id}' no dejó estado guardado: no hay desde dónde retomar"),
            )
        })?;

    // 3. El agente: es el que sabe QUÉ grafo se estaba corriendo.
    //
    //    Se busca en memoria y, si no está, en la DB (PRD-021-F): tras
    //    reiniciar el proceso el `HashMap` arranca vacío, y antes eso mataba el
    //    reanudar aunque el checkpoint hubiera sobrevivido intacto. Solo queda
    //    el 409 si la definición nunca se guardó (agente de una DB pre-v3).
    let spec = state.agent_spec(&cp.agent_id).await.ok_or_else(|| {
        err(
            StatusCode::CONFLICT,
            format!(
                "el agente '{}' del run '{id}' ya no está registrado: no hay grafo que reanudar",
                cp.agent_id
            ),
        )
    })?;
    let graph = grafo_del_spec(&spec);

    // 4. El estado tal cual quedó.
    let shared = cp
        .to_shared_state()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 5. ¿Dónde retomar? Si quedó en un `human_input`, ese nodo se resuelve con
    //    la respuesta humana y se sigue por el SIGUIENTE (nunca se re-pregunta).
    let parada = cp.resume_node_id();
    let mut ya_ejecutados = cp.executed_nodes.clone();
    let desde = match nodo_por_id(&graph, &parada) {
        Some(n) if n.tool_type == wk::HUMAN_INPUT_TOOL => {
            let salida = respuesta_humana(&req);
            shared
                .set(&parada, salida.clone(), true)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            ya_ejecutados.push(parada.clone());
            state
                .runner
                .resolve_all_next_nodes(&parada, &salida, &graph)
                .into_iter()
                .next()
        }
        _ => Some(parada.clone()),
    };

    // 6. El run vuelve a estar vivo ANTES de correr: si alguien lo consulta o
    //    lo cancela mientras corre, lo ve `running`, no `failed`/`paused`.
    marcar_en_vuelo(&repo, &registro, desde.as_deref()).await;

    let started_at = registro.created_at;
    let resultado = match desde.clone() {
        // El `human_input` era el último nodo: contestarlo cierra el run.
        None => cierre_sin_nodos_pendientes(&cp, &shared),
        Some(nodo) => {
            let runner = state
                .runner_with_checkpoints(&id, &cp.agent_id, &spec.name, started_at)
                .await;
            let llm = (state.llm_factory)();
            let mut ctx = DefaultExecutionContext::builder(llm)
                .with_db(Box::new(InMemoryDBResource::new()))
                .with_storage(Box::new(InMemoryStorageResource::new()))
                .with_session_id(&id);
            if let Some(prompt) = &spec.system_prompt {
                ctx = ctx.with_system_prompt(prompt);
            }
            let context = ctx.build();

            match runner
                .resume_skipping(&graph, &context, shared, &nodo, &ya_ejecutados, cp.step)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return Err(err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("no se pudo reanudar: {e}"),
                    ))
                }
            }
        }
    };

    let cuerpo = json!({
        "session_id": id,
        "agent_id": cp.agent_id,
        "agent_name": spec.name,
        "resumed_from": desde.clone().unwrap_or_else(|| parada.clone()),
        "skipped_nodes": ya_ejecutados,
        "status": resultado.status,
        "run_status": SessionStatus::from_execution(&resultado.status).to_string(),
        "trace": resultado.trace,
        "transcript": resultado.transcript,
        "state": resultado.state.snapshot(),
        "error": resultado.error,
    });

    state
        .record_session(id, &cp.agent_id, &spec.name, started_at, resultado)
        .await;

    Ok(Json(cuerpo))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mismo grafo que arma el camino de ejecución normal (`helpers.rs`), para que
/// reanudar corra EXACTAMENTE el grafo que se estaba corriendo.
fn grafo_del_spec(spec: &AgentSpec) -> GraphDef {
    let mut graph = spec.to_graph(Some(&spec.name));
    graph.auto_generate_edge_ids();
    if !spec.config.mcp_servers.is_empty() {
        let mcp = serde_json::to_value(&spec.config.mcp_servers).unwrap_or_default();
        for node in &mut graph.nodes {
            if node.tool_type == "mcp/call" {
                node.config.insert("__mcp_servers".to_string(), mcp.clone());
            }
        }
    }
    graph
}

fn nodo_por_id<'g>(graph: &'g GraphDef, id: &str) -> Option<&'g crate::core::graph::NodeDef> {
    graph.nodes.iter().find(|n| n.id == id)
}

/// Salida del nodo `logic/human_input`, con la misma forma que produce la tool
/// cuando se la ejecuta con una respuesta: quien lea `${ask.response}` río
/// abajo no nota diferencia entre "contestado por API" y "contestado en línea".
fn respuesta_humana(req: &ResumeRequest) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    out.insert(
        "response".to_string(),
        req.response.clone().unwrap_or(Value::Null),
    );
    out.insert(
        "responded_by".to_string(),
        Value::String(
            req.responded_by
                .clone()
                .unwrap_or_else(|| "human".to_string()),
        ),
    );
    out.insert("response_time_ms".to_string(), json!(0.0));
    out
}

/// Deja la fila del run en `running` con su nodo actual antes de reanudar.
/// Best-effort: si falla se loguea, pero el run se reanuda igual (persistir no
/// puede impedir trabajar).
async fn marcar_en_vuelo(repo: &SqliteSessionRepo, registro: &SessionRecord, nodo: Option<&str>) {
    let mut vivo = registro.clone();
    vivo.status = SessionStatus::Running;
    vivo.current_node_id = nodo.map(str::to_string);
    vivo.finished_at = None;
    vivo.duration_ms = None;
    if let Err(e) = repo.save(&vivo).await {
        tracing::warn!(session_id = %vivo.id, error = %e, "no se pudo marcar el run como en vuelo");
    }
}

/// El nodo que preguntaba era el último del grafo: con la respuesta puesta en
/// el estado ya no queda nada por correr, así que el run está completo. No se
/// llama al runner porque no hay nodo que ejecutar (arrancarlo devolvería el
/// grafo entero desde cero).
fn cierre_sin_nodos_pendientes(
    cp: &CheckpointRecord,
    shared: &crate::core::state::SharedState,
) -> ExecutionResult {
    ExecutionResult {
        status: crate::core::runner::ExecutionStatus::Completed,
        state: shared.clone(),
        trace: vec![],
        transcript: vec![crate::core::runner::TranscriptEntry {
            entry_type: wk::TRANSCRIPT_COMPLETED.to_string(),
            message: format!(
                "Reanudado en '{}': era el último nodo, la respuesta humana cierra el run",
                cp.resume_node_id()
            ),
            timestamp: crate::utils::now_epoch(),
            node_id: Some(cp.resume_node_id()),
            metadata: HashMap::new(),
        }],
        error: None,
        interrupt_node_id: None,
        interrupt_info: None,
    }
}
