//! PRD-021-E — el bus de observabilidad de la **ejecución**: `GET /api/v1/events`.
//!
//! # Los DOS buses del motor: qué evento nace dónde
//!
//! El motor tiene dos flujos de eventos con nombres parecidos y propósitos
//! distintos. Confundirlos costó que uno de los dos naciera muerto, así que
//! queda escrito aquí (y en `streaming.rs` y en `core/events.rs`):
//!
//! | | `StreamEvent` (`streaming.rs`) | `ExecutionEvent` (`core/events.rs`) |
//! |---|---|---|
//! | **Nace en** | `GraphRunner::stream_event()` | `GraphRunner::emit_event()` |
//! | **Canal** | `mpsc`, **uno por request** | `broadcast`, **uno por proceso** |
//! | **Consumidor** | UNO: el cliente que hizo `POST /agents/{id}/stream` | N: cualquiera suscrito a `GET /api/v1/events` |
//! | **Vive** | lo que dura ESE run | lo que dura el proceso, atravesando TODOS los runs |
//! | **Trae** | el dato de producto (output del nodo, snapshot final) | el envelope de observabilidad (`event_id`, `session_id`, `node_id`, `timestamp`) |
//! | **Cubre** | grafo/nodo/fanout | eso **más** `checkpoint_created`, `interrupt_created`, `session_interrupted`, `hook_*`, `llm_token`… |
//! | **Contrato** | **público desde 0.7.0** — no se toca | nuevo, aditivo |
//!
//! **Decisión (021-E): no se unifican, el bus se EXPONE aparte.** Meter los
//! `ExecutionEvent` dentro del `/stream` obligaba a una de dos cosas malas:
//! reescribir el sobre de `StreamEvent` (rompe a los clientes de 0.7.0, está
//! prohibido) o aplanar cada `ExecutionEvent` en una variante de `StreamEvent`
//! (pierde `event_id` y `session_id`, que son justo lo que un observador
//! necesita para correlacionar, y deja fuera los tipos que no tienen variante).
//! Además los ciclos de vida no coinciden: `/stream` es por-run y single-shot;
//! un observador de flota necesita engancharse **una vez** y ver **todos** los
//! runs. Por eso `/stream` queda **byte por byte igual** que en 0.7.0 y este
//! endpoint es aditivo.
//!
//! # Emisores conectados a este bus
//!
//! - `AppState::runner` (cableado en `state.rs`): TODO run que pase por
//!   `POST /agents/{id}/execute` o `POST /agents/{id}/stream` emite aquí,
//!   porque ambos caminos clonan ese runner.
//! - **NO** cuelga de aquí el emisor del orquestador tmux (`SessionManager`
//!   tiene el suyo, expuesto en `/api/v1/orchestrator/events`): son flotas
//!   distintas y mezclarlas volvería inútil el filtro por run.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use super::state::AppState;

/// Filtros de `GET /api/v1/events`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct EventsQuery {
    /// Solo los eventos de ESE run. Sin él, se ve todo el proceso.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Lista separada por comas de `event_type` (`checkpoint_created,...`).
    /// Sin ella, todos los tipos.
    #[serde(default)]
    pub types: Option<String>,
}

/// GET /api/v1/events — SSE con los `ExecutionEvent` que emite el runner.
///
/// Formato del frame: el **envelope completo** serializado
/// (`ExecutionEvent::to_sse()`), no solo `data` —
/// `/api/v1/orchestrator/events` sí manda solo `data` porque su UI espera esas
/// formas exactas; aquí el `session_id` y el `event_id` SON el producto: sin
/// ellos no se puede correlacionar un evento con su run ni detectar huecos.
///
/// ```text
/// event: checkpoint_created
/// data: {"event_type":"checkpoint_created","timestamp":1.7e9,"session_id":"ab12","node_id":"ask","data":{...},"event_id":3}
/// ```
///
/// Es un bus en vivo, no un histórico: entrega desde el momento en que el
/// cliente se suscribe. Para poder engancharse ANTES de lanzar un run, la
/// suscripción se toma en el handler (no dentro del `spawn`), así que en
/// cuanto la respuesta tiene cabeceras el receptor ya existe y no hay carrera.
///
/// Auth: además del header `X-API-Key`, acepta `?api_key=` (EventSource no
/// puede poner cabeceras) — ver `auth_middleware`.
pub(crate) async fn execution_events(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    use axum::body::Body;
    use tokio_stream::wrappers::ReceiverStream;

    // Suscripción ANTES de responder: lo que ocurra desde este instante se ve.
    let mut events = state.events.subscribe();

    let want_session = q.session_id.clone();
    let want_types: Option<Vec<String>> = q.types.as_ref().map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256);

    tokio::spawn(async move {
        // Primer byte inmediato: el cliente sabe que el stream está abierto
        // aunque todavía no haya ningún run en vuelo.
        if tx.send(Ok(": connected\n\n".to_string())).await.is_err() {
            return;
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(ref sid) = want_session {
                        if &event.session_id != sid {
                            continue;
                        }
                    }
                    if let Some(ref types) = want_types {
                        if !types.iter().any(|t| t == &event.event_type.to_string()) {
                            continue;
                        }
                    }
                    if tx.send(Ok(event.to_sse())).await.is_err() {
                        break; // cliente desconectado
                    }
                }
                // Consumidor lento: se perdieron `n` eventos. No se calla — el
                // cliente recibe un comentario SSE (que EventSource ignora pero
                // un consumidor crudo puede leer) y queda en el log.
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(perdidos = n, "suscriptor lento del bus de ejecución");
                    if tx.send(Ok(format!(": lagged {n}\n\n"))).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("connection", "keep-alive"),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
}

// ---------------------------------------------------------------------------
// Tests — el contrato de PRD-021-E (E1, E2, E3)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::core::events::{EventType, ExecutionEvent};
    use crate::server::create_router;
    use crate::server::state::{AppState, LLMFactory};
    use crate::tools::builtin::register_all_builtin_tools;
    use crate::tools::registry::ToolRegistry;

    /// LLM mock: SOLO para unit tests (`#[cfg(test)]`), igual que
    /// `server/tests.rs`. Los grafos de estos tests no tienen nodos de LLM
    /// (`trigger/manual`, `logic/human_input`, `output/response`), así que la
    /// factory nunca se ejerce en el camino que se está probando.
    fn test_llm_factory() -> LLMFactory {
        use crate::adapters::MockLLMResource;
        Arc::new(|| Box::new(MockLLMResource::new()))
    }

    /// AppState con SQLite REAL (in-memory) para que el `CheckpointCallback`
    /// de 021-A esté cableado y `checkpoint_created` tenga de dónde nacer.
    fn app_con_db() -> (AppState, Router) {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let mut state = AppState::new(registry, test_llm_factory(), None);
        state.session_repo = Some(Arc::new(
            crate::db::SqliteSessionRepo::open_in_memory().unwrap(),
        ));
        let app = create_router(state.clone());
        (state, app)
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Agente `trigger → ask(human_input) → out`: se detiene esperando a un
    /// humano, que es justo cuando 021-A escribe checkpoint.
    async fn crear_agente_con_pausa(app: &Router, name: &str) -> String {
        let spec = json!({
            "name": name,
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hola"}}},
                    {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "¿seguimos?"}},
                    {"id": "out", "tool_type": "output/response", "config": {"message": "done"}}
                ],
                "edges": [
                    {"source": "trigger", "target": "ask"},
                    {"source": "ask", "target": "out"}
                ]
            }
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/agents/from-spec")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        body_json(resp.into_body()).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn ejecutar(app: &Router, agent_id: &str) -> String {
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/agents/{agent_id}/execute"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "trigger_data": {} })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        body_json(resp.into_body()).await["session_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Drena hasta `max` frames SSE del body, rindiéndose tras `espera` sin
    /// novedad. Un stream SSE no termina solo: sin timeout el test cuelga.
    async fn drenar_sse(body: Body, max: usize, espera: Duration) -> Vec<String> {
        let mut body = body;
        let mut frames = Vec::new();
        while frames.len() < max {
            match tokio::time::timeout(espera, body.frame()).await {
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        frames.push(String::from_utf8_lossy(data).to_string());
                    }
                }
                // Fin del body o error → no hay más que leer.
                Ok(_) => break,
                // Silencio: el stream sigue abierto pero ya no llega nada.
                Err(_) => break,
            }
        }
        frames
    }

    /// Parsea frames SSE crudos a `ExecutionEvent` (ignora comentarios `:`).
    fn eventos_de(frames: &[String]) -> Vec<ExecutionEvent> {
        frames
            .iter()
            .flat_map(|f| f.split("\n\n"))
            .filter_map(|bloque| {
                bloque
                    .lines()
                    .find_map(|l| l.strip_prefix("data: "))
                    .and_then(|json| serde_json::from_str::<ExecutionEvent>(json).ok())
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // E1 — los ExecutionEvent del runner llegan a un suscriptor real
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn e1_ejecutar_por_la_api_alimenta_el_bus_de_ejecucion() {
        // Antes de 021-E esto daba CERO eventos: `with_event_emitter` no se
        // llamaba en producción, así que `emit_event` era un no-op.
        let (state, app) = app_con_db();
        let mut bus = state.events.subscribe();

        let agent_id = crear_agente_con_pausa(&app, "e1-bus").await;
        let session_id = ejecutar(&app, &agent_id).await;

        let mut recibidos = Vec::new();
        while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(200), bus.recv()).await {
            recibidos.push(ev);
        }

        assert!(
            !recibidos.is_empty(),
            "el bus de ejecución no entregó NINGÚN evento — el emisor sigue sin cablear"
        );

        let tipos: Vec<EventType> = recibidos.iter().map(|e| e.event_type.clone()).collect();
        assert!(
            tipos.contains(&EventType::SessionStarted),
            "falta session_started; llegaron: {tipos:?}"
        );
        // El evento que 021-A genera y que hasta hoy se perdía.
        assert!(
            tipos.contains(&EventType::CheckpointCreated),
            "falta checkpoint_created (021-A); llegaron: {tipos:?}"
        );
        assert!(
            tipos.contains(&EventType::InterruptCreated),
            "falta interrupt_created; llegaron: {tipos:?}"
        );

        // El checkpoint dice en qué nodo se guardó y con qué id.
        let cp = recibidos
            .iter()
            .find(|e| e.event_type == EventType::CheckpointCreated)
            .unwrap();
        assert_eq!(cp.node_id.as_deref(), Some("ask"));
        assert!(cp.data.contains_key("checkpoint_id"));
        assert_eq!(cp.session_id, session_id);
    }

    #[tokio::test]
    async fn e1_el_endpoint_sse_entrega_los_eventos_a_un_cliente_http() {
        // El bus se consume desde FUERA, por HTTP, no solo desde el proceso.
        let (_state, app) = app_con_db();
        let agent_id = crear_agente_con_pausa(&app, "e1-sse").await;

        // Suscribirse primero: el handler toma el receptor antes de responder,
        // así que al tener cabeceras ya no se pierde nada de lo que siga.
        let resp = app
            .clone()
            .oneshot(Request::get("/api/v1/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let body = resp.into_body();

        let session_id = ejecutar(&app, &agent_id).await;

        let frames = drenar_sse(body, 40, Duration::from_millis(300)).await;
        let crudo = frames.concat();
        assert!(crudo.contains(": connected"), "falta el frame de apertura");
        assert!(
            crudo.contains("event: checkpoint_created"),
            "el SSE no trajo checkpoint_created; crudo:\n{crudo}"
        );

        let eventos = eventos_de(&frames);
        assert!(
            eventos.iter().any(|e| e.session_id == session_id),
            "ningún evento del run {session_id} salió por el SSE"
        );
    }

    // -----------------------------------------------------------------------
    // E2 — cada evento se correlaciona con SU run
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn e2_todo_evento_lleva_el_session_id_de_su_run() {
        let (state, app) = app_con_db();
        let mut bus = state.events.subscribe();

        let agent_id = crear_agente_con_pausa(&app, "e2-corr").await;
        let run_a = ejecutar(&app, &agent_id).await;
        let run_b = ejecutar(&app, &agent_id).await;
        assert_ne!(run_a, run_b, "cada ejecución es un run distinto");

        let mut recibidos = Vec::new();
        while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(200), bus.recv()).await {
            recibidos.push(ev);
        }

        // Ni un evento huérfano: todos pertenecen a uno de los dos runs.
        assert!(!recibidos.is_empty());
        for ev in &recibidos {
            assert!(
                ev.session_id == run_a || ev.session_id == run_b,
                "evento {:?} con session_id '{}' — no corresponde a ningún run",
                ev.event_type,
                ev.session_id
            );
        }
        // Y los dos runs quedaron representados, cada uno con su propio id
        // (el mismo que devolvió el POST /execute → correlacionable con
        // GET /api/v1/sessions/{id}).
        assert!(recibidos.iter().any(|e| e.session_id == run_a));
        assert!(recibidos.iter().any(|e| e.session_id == run_b));

        // event_id monótono: un consumidor puede detectar huecos.
        let ids: Vec<u64> = recibidos.iter().map(|e| e.event_id).collect();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "los event_id deben crecer para poder detectar pérdidas: {ids:?}"
        );
    }

    #[tokio::test]
    async fn e2_el_filtro_por_session_id_aisla_un_run() {
        // El id de un run sólo se conoce DESPUÉS de lanzarlo, y para entonces
        // ya terminó: no hay forma de suscribirse "a ese run" a tiempo con un
        // grafo real. Así que aquí se ejercita el endpoint (bus real, filtro
        // real, serialización real) publicando en el bus desde el test.
        // La correlación con runs de verdad la cubre el test de arriba.
        let (state, app) = app_con_db();

        let resp = app
            .oneshot(
                Request::get("/api/v1/events?session_id=run-mio")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body();

        state.events.emit(
            EventType::SessionStarted,
            "run-ajeno".into(),
            None,
            Default::default(),
        );
        state.events.emit(
            EventType::SessionStarted,
            "run-mio".into(),
            None,
            Default::default(),
        );
        state.events.emit(
            EventType::CheckpointCreated,
            "run-ajeno".into(),
            Some("x".into()),
            Default::default(),
        );
        state.events.emit(
            EventType::CheckpointCreated,
            "run-mio".into(),
            Some("ask".into()),
            Default::default(),
        );

        let frames = drenar_sse(body, 20, Duration::from_millis(250)).await;
        let eventos = eventos_de(&frames);
        assert_eq!(
            eventos.len(),
            2,
            "debían pasar exactamente los 2 eventos de run-mio, llegaron {}",
            eventos.len()
        );
        assert!(eventos.iter().all(|e| e.session_id == "run-mio"));
    }

    #[tokio::test]
    async fn el_filtro_por_types_deja_pasar_solo_los_tipos_pedidos() {
        // Un observador que sólo quiere checkpoints no debería tragarse cada
        // token del LLM.
        let (state, app) = app_con_db();

        let resp = app
            .oneshot(
                Request::get("/api/v1/events?types=checkpoint_created,session_failed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body();

        for tipo in [
            EventType::SessionStarted,
            EventType::CheckpointCreated,
            EventType::LlmToken,
            EventType::SessionFailed,
        ] {
            state
                .events
                .emit(tipo, "r1".into(), None, Default::default());
        }

        let frames = drenar_sse(body, 20, Duration::from_millis(250)).await;
        let tipos: Vec<EventType> = eventos_de(&frames)
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert_eq!(
            tipos,
            vec![EventType::CheckpointCreated, EventType::SessionFailed]
        );
    }

    // -----------------------------------------------------------------------
    // E3 — /stream sigue exactamente igual (contrato 0.7.0 intacto)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn e3_el_stream_conserva_su_formato_de_070() {
        // El bus nuevo NO se cuela en `/stream`: los clientes vivos siguen
        // viendo los mismos nombres de evento y el mismo sobre `{event, data}`.
        let (_state, app) = app_con_db();

        let spec = json!({
            "name": "e3-stream",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hola"}}},
                    {"id": "out", "tool_type": "output/response", "config": {"message": "done"}}
                ],
                "edges": [{"source": "trigger", "target": "out"}]
            }
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/agents/from-spec")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let agent_id = body_json(resp.into_body()).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/agents/{agent_id}/stream"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "trigger_data": {} })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );

        let frames = drenar_sse(resp.into_body(), 40, Duration::from_millis(500)).await;
        let crudo = frames.concat();

        // Los nombres de evento que un cliente de 0.7.0 ya conocía.
        for esperado in [
            "event: run.started",
            "event: graph.started",
            "event: node.started",
            "event: node.completed",
            "event: graph.completed",
        ] {
            assert!(
                crudo.contains(esperado),
                "el /stream perdió '{esperado}'; crudo:\n{crudo}"
            );
        }

        // El sobre sigue siendo el de `StreamEvent`: {"event": …, "data": …}.
        let payloads: Vec<Value> = crudo
            .split("\n\n")
            .filter_map(|b| b.lines().find_map(|l| l.strip_prefix("data: ")))
            .filter_map(|j| serde_json::from_str::<Value>(j).ok())
            .collect();
        assert!(!payloads.is_empty());
        for p in &payloads {
            assert!(
                p.get("event").is_some() && p.get("data").is_some(),
                "sobre inesperado en /stream: {p}"
            );
            // Y NO el envelope del otro bus: si esto aparece, alguien mezcló
            // los dos flujos y rompió el contrato existente.
            assert!(
                p.get("event_id").is_none() && p.get("event_type").is_none(),
                "un ExecutionEvent se coló en /stream: {p}"
            );
        }

        // graph.completed conserva sus campos (PRD-015).
        let completed = payloads
            .iter()
            .find(|p| p["event"] == "graph.completed")
            .expect("graph.completed");
        assert!(completed["data"]["status"].is_string());
        assert!(completed["data"]["nodes_executed"].is_number());
        assert!(completed["data"]["output"].is_object());
    }
}
