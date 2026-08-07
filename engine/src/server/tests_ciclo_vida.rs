//! Ciclo de vida de un run: pausar, listar, reanudar, cancelar (PRD-021-B/C).
//!
//! Estas son **las validaciones del contrato** (W1, W2, W4, W5, W6, W7). Se
//! escribieron ANTES del código y corren contra el router HTTP real, la DB
//! SQLite real y las tools built-in reales — cero mocks en el camino de
//! ejecución (los grafos de estos tests no tocan un solo nodo `ai/*`, así que
//! la `LLMFactory` de test nunca se invoca).
//!
//! El efecto observable de W4/W5/W7 es un **archivo en disco**: cada nodo
//! `system/bash` le hace `>>` una línea. Contar líneas es contar efectos —
//! que es lo único que demuestra que un nodo no se ejecutó dos veces (leer el
//! estado final NO lo demuestra: un re-ejecutado deja el mismo estado).

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

/// Router + estado + repo SQLite REAL (in-memory, no fake).
fn app_con_db() -> (Router, AppState, Arc<SqliteSessionRepo>) {
    let mut registry = ToolRegistry::new();
    register_all_builtin_tools(&mut registry);
    let mut state = AppState::new(registry, test_llm_factory(), None);
    let repo = Arc::new(SqliteSessionRepo::open_in_memory().unwrap());
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

/// Da de alta el agente y devuelve su id.
async fn crear_agente(app: &Router, spec: Value) -> String {
    let (status, body) = post(app, "/api/v1/agents/from-spec", spec).await;
    assert_eq!(status, StatusCode::CREATED, "alta de agente: {body}");
    body["id"].as_str().unwrap().to_string()
}

/// Ejecuta el agente y devuelve el id del run.
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

/// Carpeta temporal única para los efectos observables de un test.
fn dir_efectos(nombre: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("mirai-021-{nombre}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Cuántas veces se ejecutó cada nodo, según el archivo de efectos.
fn efectos(log: &std::path::Path) -> Vec<String> {
    match std::fs::read_to_string(log) {
        Ok(txt) => txt.lines().map(str::to_string).collect(),
        Err(_) => vec![],
    }
}

/// Nodo `system/bash` que deja una marca (append) en el log de efectos.
fn nodo_efecto(id: &str, log: &std::path::Path) -> Value {
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
// W1 — un `logic/human_input` deja el run PAUSADO, con estado, y en el listado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w1_human_input_deja_el_run_pausado_con_estado_y_en_el_listado() {
    let (app, _state, repo) = app_con_db();

    let agent_id = crear_agente(
        &app,
        json!({
            "name": "w1-pausa",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "hola"}}},
                    {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "¿seguimos?", "options": ["si", "no"]}},
                    {"id": "fin", "tool_type": "output/response", "config": {"message": "listo"}}
                ],
                "edges": [
                    {"source": "trigger", "target": "ask"},
                    {"source": "ask", "target": "fin"}
                ]
            }
        }),
    )
    .await;

    let run_id = ejecutar(&app, &agent_id).await;

    // 1) El run quedó PAUSADO (no `interrupted`: eso era la 0.7.0 y ahí moría).
    let (status, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        run["run_status"], "paused",
        "un human_input pausa el run, no lo mata: {run}"
    );
    assert_eq!(run["current_node_id"], "ask");

    // 2) Con su estado guardado en disco, listo para retomar.
    let cp = repo
        .get_checkpoint(&run_id)
        .await
        .unwrap()
        .expect("un run pausado guarda su estado");
    assert_eq!(cp.cursor_node_id.as_deref(), Some("ask"));
    assert_eq!(cp.executed_nodes, vec!["trigger"]);
    assert!(!cp.already_executed("fin"), "el nodo final no corrió");

    // 3) Y aparece en el listado de pausados.
    let (status, pausados) = get(&app, "/api/v1/sessions?status=paused").await;
    assert_eq!(status, StatusCode::OK);
    let arr = pausados.as_array().expect("lista");
    assert_eq!(arr.len(), 1, "el run pausado tiene que salir listado");
    assert_eq!(arr[0]["id"], run_id);
    assert_eq!(arr[0]["current_node_id"], "ask");

    // Y NO aparece filtrando por otro estado.
    let (_, completados) = get(&app, "/api/v1/sessions?status=completed").await;
    assert!(completados.as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// W2 — con la respuesta humana por la API, el run continúa y termina
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w2_resume_con_respuesta_humana_continua_y_completa() {
    let (app, _state, repo) = app_con_db();

    let agent_id = crear_agente(
        &app,
        json!({
            "name": "w2-resume",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "hola"}}},
                    {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "¿seguimos?"}},
                    {"id": "fin", "tool_type": "output/response", "config": {"message": "listo"}}
                ],
                "edges": [
                    {"source": "trigger", "target": "ask"},
                    {"source": "ask", "target": "fin"}
                ]
            }
        }),
    )
    .await;

    let run_id = ejecutar(&app, &agent_id).await;

    let (status, body) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/resume"),
        json!({"response": "si", "responded_by": "gabriel"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    assert_eq!(body["status"], "Completed", "el run tiene que terminar");

    // Continuó DESDE EL NODO SIGUIENTE: `fin` corrió, y la respuesta humana
    // quedó en el estado del nodo que preguntaba.
    let estado = &body["state"];
    assert_eq!(estado["ask"]["response"], "si");
    assert_eq!(estado["ask"]["responded_by"], "gabriel");
    assert!(estado["fin"].is_object(), "el nodo siguiente sí corrió");
    assert!(
        estado["trigger"].is_object(),
        "el estado de antes de la pausa se conservó"
    );

    // El run ya no está pausado y soltó su checkpoint.
    let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
    assert_eq!(run["run_status"], "completed");
    assert!(repo.get_checkpoint(&run_id).await.unwrap().is_none());
    let (_, pausados) = get(&app, "/api/v1/sessions?status=paused").await;
    assert!(pausados.as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// W4 — cancelar un run EN VUELO lo detiene y lo registra con su nodo
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w4_cancelar_un_run_en_vuelo_lo_detiene_y_registra_el_nodo() {
    let dir = dir_efectos("w4");
    let log = dir.join("efectos.log");

    let (app, _state, _repo) = app_con_db();

    let agent_id = crear_agente(
        &app,
        json!({
            "name": "w4-cancel",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    nodo_efecto("marca_a", &log),
                    {"id": "espera", "tool_type": "logic/wait", "config": {"delay_seconds": 3.0}},
                    nodo_efecto("marca_b", &log)
                ],
                "edges": [
                    {"source": "trigger", "target": "marca_a"},
                    {"source": "marca_a", "target": "espera"},
                    {"source": "espera", "target": "marca_b"}
                ]
            }
        }),
    )
    .await;

    // El run arranca en background: el `execute` no vuelve hasta terminar.
    let app_bg = app.clone();
    let agente = agent_id.clone();
    let corriendo = tokio::spawn(async move { ejecutar(&app_bg, &agente).await });

    // Se espera a verlo EN VUELO por la propia API (?status=running).
    let mut run_id = String::new();
    for _ in 0..100 {
        let (_, vivos) = get(&app, "/api/v1/sessions?status=running").await;
        if let Some(primero) = vivos.as_array().and_then(|a| a.first()) {
            run_id = primero["id"].as_str().unwrap().to_string();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!run_id.is_empty(), "el run en vuelo tiene que ser visible");

    // Cancelar.
    let (status, body) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "cancel: {body}");

    // El run se detiene solo (cooperativo: en frontera de nodo, no un kill).
    let run_id_terminado = tokio::time::timeout(std::time::Duration::from_secs(20), corriendo)
        .await
        .expect("el run cancelado no puede quedarse colgado")
        .unwrap();
    assert_eq!(run_id_terminado, run_id);

    let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
    assert_eq!(run["run_status"], "cancelled");
    assert_eq!(
        run["current_node_id"], "marca_b",
        "queda registrado el nodo en el que iba: {run}"
    );

    // Y el nodo posterior a la cancelación NUNCA corrió (efecto observable).
    let marcas = efectos(&log);
    assert_eq!(
        marcas,
        vec!["marca_a"],
        "cancelar tiene que impedir el efecto del nodo siguiente"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// W5 — un run que falló en el nodo 7 de 10 reanuda EN EL 7, con su estado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w5_run_fallido_en_el_nodo_7_reanuda_en_el_7() {
    let dir = dir_efectos("w5");
    let log = dir.join("efectos.log");
    // El nodo 7 lee un archivo que todavía no existe → falla ahí.
    let puerta = dir.join("puerta.txt");

    let (app, _state, repo) = app_con_db();

    let mut nodos = vec![json!({
        "id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}
    })];
    let mut aristas = vec![];
    let mut previo = "trigger".to_string();
    for i in 1..=10 {
        let id = format!("n{i}");
        if i == 7 {
            nodos.push(json!({
                "id": id,
                "tool_type": "filesystem/read_file",
                "config": {"path": puerta.display().to_string(), "retry_policy": {"max_retries": 0}}
            }));
        } else {
            nodos.push(nodo_efecto(&id, &log));
        }
        aristas.push(json!({"source": previo, "target": id}));
        previo = id;
    }

    let agent_id = crear_agente(
        &app,
        json!({
            "name": "w5-fallo",
            "version": "v1",
            "graph": {"nodes": nodos, "edges": aristas}
        }),
    )
    .await;

    let run_id = ejecutar(&app, &agent_id).await;

    // Falló en el 7 y solo corrieron los 6 primeros.
    let (_, run) = get(&app, &format!("/api/v1/sessions/{run_id}")).await;
    assert_eq!(run["run_status"], "failed", "{run}");
    assert_eq!(
        run["current_node_id"], "n7",
        "el registro dice dónde se cayó"
    );
    assert_eq!(
        efectos(&log),
        vec!["n1", "n2", "n3", "n4", "n5", "n6"],
        "los nodos 8-10 no llegaron a correr"
    );

    let cp = repo.get_checkpoint(&run_id).await.unwrap().expect("estado");
    assert_eq!(cp.cursor_node_id.as_deref(), Some("n7"), "ahí se retoma");

    // Se destraba la causa del fallo y se reanuda.
    std::fs::write(&puerta, "abierta").unwrap();
    let (status, body) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    assert_eq!(body["status"], "Completed");
    assert_eq!(
        body["resumed_from"], "n7",
        "arranca EN EL 7, no desde cero: {body}"
    );

    // Con el estado que tenía: lo de los nodos 1-6 sigue ahí.
    let estado = &body["state"];
    for i in 1..=6 {
        assert!(
            estado[format!("n{i}")].is_object(),
            "el estado del nodo n{i} se conservó"
        );
    }
    // Y los nodos 1-6 NO se volvieron a ejecutar; 8-10 sí corrieron, una vez.
    assert_eq!(
        efectos(&log),
        vec!["n1", "n2", "n3", "n4", "n5", "n6", "n8", "n9", "n10"]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// W6 — consultar un run muestra estado, nodo actual y desde cuándo
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w6_consultar_un_run_muestra_estado_nodo_actual_y_desde_cuando() {
    let (app, _state, _repo) = app_con_db();

    // (a) Run PAUSADO.
    let pausable = crear_agente(
        &app,
        json!({
            "name": "w6-pausa",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "?"}}
                ],
                "edges": [{"source": "trigger", "target": "ask"}]
            }
        }),
    )
    .await;
    let pausado = ejecutar(&app, &pausable).await;

    let (status, run) = get(&app, &format!("/api/v1/sessions/{pausado}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["run_status"], "paused", "estado");
    assert_eq!(run["current_node_id"], "ask", "nodo actual");
    let desde = run["since"].as_f64().expect("desde cuándo (epoch)");
    let arranque = run["started_at"].as_f64().expect("cuándo arrancó");
    assert!(arranque > 0.0);
    assert!(
        desde >= arranque,
        "since = desde cuándo está en este estado"
    );

    // (b) Run COMPLETADO.
    let simple = crear_agente(
        &app,
        json!({
            "name": "w6-ok",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    {"id": "fin", "tool_type": "output/response", "config": {"message": "ok"}}
                ],
                "edges": [{"source": "trigger", "target": "fin"}]
            }
        }),
    )
    .await;
    let completado = ejecutar(&app, &simple).await;
    let (_, run) = get(&app, &format!("/api/v1/sessions/{completado}")).await;
    assert_eq!(run["run_status"], "completed");
    assert!(run["since"].as_f64().unwrap() > 0.0);
    assert!(run["finished_at"].as_f64().unwrap() > 0.0);

    // Y el listado también responde "¿en qué va cada agente?".
    let (_, todos) = get(&app, "/api/v1/sessions").await;
    let arr = todos.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    for r in arr {
        assert!(r["run_status"].is_string(), "estado por run: {r}");
        assert!(r["started_at"].as_f64().unwrap() > 0.0);
    }
}

// ---------------------------------------------------------------------------
// W7 — al reanudar, los nodos ya ejecutados NO se vuelven a ejecutar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w7_reanudar_no_repite_los_efectos_de_los_nodos_ya_ejecutados() {
    let dir = dir_efectos("w7");
    let log = dir.join("efectos.log");

    let (app, _state, _repo) = app_con_db();

    let agent_id = crear_agente(
        &app,
        json!({
            "name": "w7-efectos",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    nodo_efecto("correo_1", &log),
                    nodo_efecto("correo_2", &log),
                    {"id": "ask", "tool_type": "logic/human_input", "config": {"prompt": "¿mando el tercero?"}},
                    nodo_efecto("correo_3", &log)
                ],
                "edges": [
                    {"source": "trigger", "target": "correo_1"},
                    {"source": "correo_1", "target": "correo_2"},
                    {"source": "correo_2", "target": "ask"},
                    {"source": "ask", "target": "correo_3"}
                ]
            }
        }),
    )
    .await;

    let run_id = ejecutar(&app, &agent_id).await;

    // Antes de reanudar: dos efectos, uno por nodo ejecutado.
    assert_eq!(efectos(&log), vec!["correo_1", "correo_2"]);

    let (status, body) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/resume"),
        json!({"response": "dale"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    assert_eq!(body["status"], "Completed");

    // LA validación: cada correo se mandó UNA sola vez.
    assert_eq!(
        efectos(&log),
        vec!["correo_1", "correo_2", "correo_3"],
        "reanudar NO puede repetir el efecto de un nodo ya ejecutado"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Bordes del contrato (no son W, pero el TL los va a buscar)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reanudar_un_run_completado_es_conflicto_no_una_segunda_ejecucion() {
    let (app, _state, _repo) = app_con_db();
    let agent_id = crear_agente(
        &app,
        json!({
            "name": "borde-completado",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    {"id": "fin", "tool_type": "output/response", "config": {"message": "ok"}}
                ],
                "edges": [{"source": "trigger", "target": "fin"}]
            }
        }),
    )
    .await;
    let run_id = ejecutar(&app, &agent_id).await;

    let (status, _) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn reanudar_o_cancelar_un_run_inexistente_es_404() {
    let (app, _state, _repo) = app_con_db();
    let (status, _) = post(&app, "/api/v1/sessions/fantasma/resume", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post(&app, "/api/v1/sessions/fantasma/cancel", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancelar_un_run_ya_terminado_es_conflicto() {
    let (app, _state, _repo) = app_con_db();
    let agent_id = crear_agente(
        &app,
        json!({
            "name": "borde-cancel",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"m": "x"}}},
                    {"id": "fin", "tool_type": "output/response", "config": {"message": "ok"}}
                ],
                "edges": [{"source": "trigger", "target": "fin"}]
            }
        }),
    )
    .await;
    let run_id = ejecutar(&app, &agent_id).await;
    let (status, _) = post(
        &app,
        &format!("/api/v1/sessions/{run_id}/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}
