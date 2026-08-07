//! W3-real — reanudar un run **después de reiniciar el proceso** (PRD-021-F).
//!
//! Los tests de `tests_ciclo_vida.rs` reanudan dentro del MISMO proceso, donde
//! `AppState.agents` (un `HashMap` en RAM) sigue vivo. Por eso no cazaron el
//! fallo real: el estado del run sobrevivía al reinicio (checkpoint en SQLite)
//! pero la **definición del agente** no, así que `resume` respondía
//! `409 Conflict — "el agente '…' ya no está registrado: no hay grafo que
//! reanudar"`.
//!
//! Estos tests cruzan el reinicio **de verdad**:
//!   1. DB SQLite **en disco** (no `:memory:`, que muere con la conexión).
//!   2. Se construye un `AppState`+`Router`, se ejecuta hasta la parada.
//!   3. Se **destruye por completo** ese estado en memoria (`drop`) y se
//!      construye uno NUEVO sobre la MISMA base — el proceso nuevo.
//!   4. Se verifica que el registro de agentes del proceso nuevo está **vacío**
//!      (si no, el test no estaría probando nada) y se reanuda.
//!
//! El efecto observable es un **archivo en disco**: cada nodo `system/bash` le
//! hace `>>` una línea. Contar líneas es lo único que demuestra que un nodo no
//! se ejecutó dos veces — el estado final NO lo demuestra, porque un nodo
//! re-ejecutado deja exactamente el mismo estado.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::db::SqliteSessionRepo;
use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use super::create_router;
use super::state::{AppState, LLMFactory};

// ---------------------------------------------------------------------------
// Andamiaje
// ---------------------------------------------------------------------------

/// LLM factory de test. Los grafos de este archivo NO tienen nodos `ai/*`, así
/// que esto nunca se llama: existe solo porque `AppState::new` lo exige.
fn test_llm_factory() -> LLMFactory {
    use crate::adapters::MockLLMResource;
    Arc::new(|| Box::new(MockLLMResource::new()))
}

/// Levanta un "proceso": estado en memoria NUEVO sobre la DB de `path`.
///
/// Llamarlo dos veces con el mismo `path` es exactamente lo que hace un
/// reinicio: `AppState::new` arranca con `agents`/`sessions` vacíos y lo único
/// que sobrevive es lo que quedó escrito en SQLite.
fn proceso(path: &Path) -> (Router, AppState, Arc<SqliteSessionRepo>) {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let mut state = AppState::new(registry, test_llm_factory(), None);
    let repo = Arc::new(SqliteSessionRepo::open(path).expect("abrir la DB en disco"));
    state.session_repo = Some(repo.clone());
    let app = create_router(state.clone());
    (app, state, repo)
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn post(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp.into_body()).await)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp.into_body()).await)
}

async fn crear_agente(app: &Router, spec: Value) -> String {
    let (status, body) = post(app, "/api/v1/agents/from-spec", spec).await;
    assert_eq!(status, StatusCode::CREATED, "alta de agente: {body}");
    body["id"].as_str().unwrap().to_string()
}

async fn ejecutar(app: &Router, agent_id: &str) -> String {
    let (status, body) = post(
        app,
        &format!("/api/v1/agents/{agent_id}/execute"),
        json!({ "trigger_data": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "execute: {body}");
    body["session_id"].as_str().unwrap().to_string()
}

/// Carpeta temporal única (aloja la DB y el log de efectos).
fn dir_temporal(nombre: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("mirai-021f-{nombre}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Qué nodos dejaron efecto, en orden. Una línea = una ejecución.
fn efectos(log: &Path) -> Vec<String> {
    match std::fs::read_to_string(log) {
        Ok(txt) => txt.lines().map(str::to_string).collect(),
        Err(_) => vec![],
    }
}

/// Nodo `system/bash` que deja una marca (append) en el log de efectos.
fn nodo_efecto(id: &str, log: &Path) -> Value {
    json!({
        "id": id,
        "tool_type": "system/bash",
        "config": {
            "command": format!("printf '{id}\\n' >> {}", log.display()),
            "retry_policy": {"max_retries": 0}
        }
    })
}

// ---------------------------------------------------------------------------
// W3-real (pausado) — un run PAUSADO reanuda tras reiniciar el proceso
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w3_real_run_pausado_reanuda_despues_de_reiniciar_el_proceso() {
    let dir = dir_temporal("pausado");
    let db = dir.join("mirai.db");
    let log = dir.join("efectos.log");

    // --- Proceso 1: crear, ejecutar hasta la pausa -------------------------
    let run_id = {
        let (app, _state, repo) = proceso(&db);

        let agent_id = crear_agente(
            &app,
            json!({
                "name": "w3f-pausa",
                "version": "v1",
                "graph": {
                    "nodes": [
                        {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "hola"}}},
                        nodo_efecto("antes", &log),
                        {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "¿seguimos?"}},
                        nodo_efecto("despues", &log),
                        {"id": "fin", "tool_type": "output/response", "config": {"message": "listo"}}
                    ],
                    "edges": [
                        {"source": "trigger", "target": "antes"},
                        {"source": "antes", "target": "ask"},
                        {"source": "ask", "target": "despues"},
                        {"source": "despues", "target": "fin"}
                    ]
                }
            }),
        )
        .await;

        let run_id = ejecutar(&app, &agent_id).await;

        let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
        assert_eq!(run["run_status"], "paused", "tiene que quedar pausado: {run}");
        assert_eq!(run["current_node_id"], "ask");
        assert_eq!(
            efectos(&log),
            vec!["antes"],
            "solo corrió el nodo previo a la pausa"
        );
        assert!(repo.get_checkpoint(&run_id).await.unwrap().is_some());

        run_id
    };
    // Fin del scope: `app`, `state` y `repo` del proceso 1 quedan destruidos.
    // Lo único que sigue existiendo del run es lo que se escribió en `db`.

    // --- Proceso 2: estado en memoria NUEVO sobre la MISMA base ------------
    let (app, state, _repo) = proceso(&db);

    // El proceso nuevo NO conoce ningún agente: si esto no fuera cierto, el
    // test no estaría probando el reinicio.
    assert!(
        state.agents.read().await.is_empty(),
        "el proceso nuevo arranca con el registro de agentes vacío"
    );

    // El run sí sobrevivió, pausado y con su nodo.
    let (status, pausados) = get(&app, "/api/v1/sessions?status=paused").await;
    assert_eq!(status, StatusCode::OK);
    let arr = pausados.as_array().expect("lista");
    assert_eq!(arr.len(), 1, "el run pausado sobrevive al reinicio");
    assert_eq!(arr[0]["id"], run_id);
    assert_eq!(arr[0]["current_node_id"], "ask");

    // AQUÍ estaba el fallo: 409 "el agente '…' ya no está registrado".
    let (status, body) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/resume"),
        json!({"response": "si", "responded_by": "gabriel"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "un run pausado tiene que reanudar tras reiniciar el proceso: {body}"
    );
    assert_eq!(body["status"], "Completed", "el run tiene que terminar: {body}");
    assert_eq!(body["resumed_from"], "despues", "sigue por el nodo siguiente");

    // El estado de antes de la pausa se conservó y la respuesta humana entró.
    let estado = &body["state"];
    assert_eq!(estado["ask"]["response"], "si");
    assert_eq!(estado["ask"]["responded_by"], "gabriel");
    assert!(
        estado["trigger"].is_object(),
        "el estado previo a la pausa sobrevivió al reinicio"
    );

    // Lo que importa: los nodos YA EJECUTADOS no se repitieron.
    assert_eq!(
        efectos(&log),
        vec!["antes", "despues"],
        "'antes' corrió UNA sola vez: reanudar no repite lo ya hecho"
    );

    let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
    assert_eq!(run["run_status"], "completed");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// W3-real (fallido) — el caso simétrico: un run FALLIDO también reanuda
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w3_real_run_fallido_reanuda_despues_de_reiniciar_el_proceso() {
    let dir = dir_temporal("fallido");
    let db = dir.join("mirai.db");
    let log = dir.join("efectos.log");
    // `n4` lee un archivo que todavía no existe → el run se cae ahí.
    let puerta = dir.join("puerta.txt");

    // --- Proceso 1: ejecutar hasta el fallo --------------------------------
    let run_id = {
        let (app, _state, repo) = proceso(&db);

        let agent_id = crear_agente(
            &app,
            json!({
                "name": "w3f-fallo",
                "version": "v1",
                "graph": {
                    "nodes": [
                        {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                        nodo_efecto("n1", &log),
                        nodo_efecto("n2", &log),
                        nodo_efecto("n3", &log),
                        {
                            "id": "n4",
                            "tool_type": "filesystem/read_file",
                            "config": {
                                "path": puerta.display().to_string(),
                                "retry_policy": {"max_retries": 0}
                            }
                        },
                        nodo_efecto("n5", &log),
                        nodo_efecto("n6", &log)
                    ],
                    "edges": [
                        {"source": "trigger", "target": "n1"},
                        {"source": "n1", "target": "n2"},
                        {"source": "n2", "target": "n3"},
                        {"source": "n3", "target": "n4"},
                        {"source": "n4", "target": "n5"},
                        {"source": "n5", "target": "n6"}
                    ]
                }
            }),
        )
        .await;

        let run_id = ejecutar(&app, &agent_id).await;

        let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
        assert_eq!(run["run_status"], "failed", "tiene que quedar fallido: {run}");
        assert_eq!(run["current_node_id"], "n4", "el registro dice dónde se cayó");
        assert_eq!(
            efectos(&log),
            vec!["n1", "n2", "n3"],
            "n5/n6 no llegaron a correr"
        );
        assert!(repo.get_checkpoint(&run_id).await.unwrap().is_some());

        run_id
    };
    // Proceso 1 destruido.

    // Se destraba la causa del fallo (como haría un operador entre reinicios).
    std::fs::write(&puerta, "abierta").unwrap();

    // --- Proceso 2: estado en memoria NUEVO sobre la MISMA base ------------
    let (app, state, _repo) = proceso(&db);
    assert!(
        state.agents.read().await.is_empty(),
        "el proceso nuevo arranca con el registro de agentes vacío"
    );

    let (status, fallidos) = get(&app, "/api/v1/sessions?status=failed").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fallidos.as_array().unwrap().len(),
        1,
        "el run fallido sobrevive al reinicio"
    );

    // Mismo 409 que el caso pausado: sin la spec no hay grafo que reanudar.
    let (status, body) = post(&app, &format!("/api/v1/sessions/{run_id}/resume"), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "un run fallido tiene que reanudar tras reiniciar el proceso: {body}"
    );
    assert_eq!(body["status"], "Completed", "{body}");
    assert_eq!(body["resumed_from"], "n4", "reintenta EN EL nodo que falló");

    // n1-n3 no se repitieron; n5/n6 corrieron una sola vez.
    assert_eq!(
        efectos(&log),
        vec!["n1", "n2", "n3", "n5", "n6"],
        "reanudar tras reinicio no repite los nodos ya ejecutados"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
