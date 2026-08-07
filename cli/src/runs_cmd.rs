//! `mirai runs …` — el ciclo de vida de un run desde la terminal (PRD-021-D).
//!
//! Ojo con el nombre: `mirai sessions` son las **sesiones de Claude en tmux**
//! (PRD-013, otro subsistema). `mirai runs` son las **ejecuciones de un agente**
//! sobre el grafo — lo que la API llama `/api/v1/sessions`.
//!
//! ```text
//! mirai runs list   [--status paused|running|…] [--agent <id>] [--limit N] [--json]
//! mirai runs show   <run_id> [--json]
//! mirai runs resume <run_id> [--response <texto|JSON>] [--by <quién>] [--json]
//! mirai runs cancel <run_id> [--json]
//! ```
//!
//! Habla HTTP contra un `mirai serve` (igual que `mirai sessions`): el server es
//! el único que tiene la DB, el runner y los checkpoints. Ubicación: `--host` /
//! `--port` o `MIRAI_HOST` / `MIRAI_PORT` (default `127.0.0.1:3000`). Auth:
//! `MIRAI_API_KEY` → `X-API-Key`.

use std::process;

use serde_json::{json, Value};

use crate::colors;

// ---------------------------------------------------------------------------
// Error de CLI
// ---------------------------------------------------------------------------

/// Un fallo ya **traducido a algo que se le puede decir a un humano**.
///
/// Las funciones de este módulo devuelven `Result` en vez de llamar a
/// `process::exit` para que los tests puedan ejercitar los caminos de error
/// (404, 409 de DB pre-v3, …) sin matar el proceso de test.
#[derive(Debug)]
pub struct CliError {
    pub mensaje: String,
}

impl CliError {
    fn nuevo(mensaje: impl Into<String>) -> Self {
        Self {
            mensaje: mensaje.into(),
        }
    }
}

type R<T> = Result<T, CliError>;

// ---------------------------------------------------------------------------
// Entrada
// ---------------------------------------------------------------------------

/// Punto de entrada del subcomando `runs`.
pub async fn cmd_runs(args: &[String]) {
    let resultado = match args.first().map(|s| s.as_str()) {
        None | Some("list" | "ls") => {
            let client = Client::from_args(args);
            listar(&client, &ListOpts::from_args(args)).await
        }
        Some("show") => {
            let rest = &args[1..];
            match id_posicional(rest) {
                Some(id) => {
                    let client = Client::from_args(rest);
                    mostrar(&client, &id, has_json(rest)).await
                }
                None => Err(CliError::nuevo("Uso: mirai runs show <run_id> [--json]")),
            }
        }
        Some("resume") => {
            let rest = &args[1..];
            match id_posicional(rest) {
                Some(id) => {
                    let client = Client::from_args(rest);
                    reanudar(
                        &client,
                        &id,
                        super::parse_flag(rest, "--response").as_deref(),
                        super::parse_flag(rest, "--by").as_deref(),
                        has_json(rest),
                    )
                    .await
                }
                None => Err(CliError::nuevo(
                    "Uso: mirai runs resume <run_id> [--response <texto|JSON>] [--by <quién>]",
                )),
            }
        }
        Some("cancel") => {
            let rest = &args[1..];
            match id_posicional(rest) {
                Some(id) => {
                    let client = Client::from_args(rest);
                    cancelar(&client, &id, has_json(rest)).await
                }
                None => Err(CliError::nuevo("Uso: mirai runs cancel <run_id>")),
            }
        }
        Some("help" | "--help" | "-h") => {
            print_runs_help();
            return;
        }
        Some(otro) => Err(CliError::nuevo(format!(
            "Subcomando desconocido: {otro}. Corré `mirai runs help`."
        ))),
    };

    match resultado {
        Ok(salida) => println!("{salida}"),
        Err(e) => {
            eprintln!("{}{}{}", colors::RED, e.mensaje, colors::RESET);
            process::exit(1);
        }
    }
}

/// Primer argumento que no sea una bandera: el `run_id`.
fn id_posicional(args: &[String]) -> Option<String> {
    args.first().filter(|a| !a.starts_with("--")).cloned()
}

fn has_json(args: &[String]) -> bool {
    super::has_flag(args, "--json")
}

// ---------------------------------------------------------------------------
// Cliente HTTP
// ---------------------------------------------------------------------------

pub struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl Client {
    /// Cliente contra una base explícita (`http://host:port/api/v1`).
    pub fn nuevo(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            http: reqwest::Client::new(),
        }
    }

    fn from_args(args: &[String]) -> Self {
        let host = super::parse_flag(args, "--host")
            .or_else(|| std::env::var("MIRAI_HOST").ok())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = super::parse_flag(args, "--port")
            .or_else(|| std::env::var("MIRAI_PORT").ok())
            .unwrap_or_else(|| "3000".to_string());
        Self::nuevo(
            format!("http://{host}:{port}/api/v1"),
            std::env::var("MIRAI_API_KEY").ok(),
        )
    }

    /// Manda la petición y devuelve `(status, cuerpo)`. Solo falla si no se
    /// pudo hablar con el server: los errores HTTP los interpreta cada
    /// subcomando, que es el que sabe qué significa un 409 en su contexto.
    async fn pedir(
        &self,
        method: reqwest::Method,
        path: &str,
        cuerpo: Option<Value>,
    ) -> R<(u16, Value)> {
        let _ = (method, path, cuerpo);
        Err(CliError::nuevo("sin implementar"))
    }
}

// ---------------------------------------------------------------------------
// Subcomandos
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ListOpts {
    pub status: Option<String>,
    pub agent_id: Option<String>,
    pub limit: Option<String>,
    pub json: bool,
}

impl ListOpts {
    fn from_args(args: &[String]) -> Self {
        Self {
            status: super::parse_flag(args, "--status"),
            agent_id: super::parse_flag(args, "--agent"),
            limit: super::parse_flag(args, "--limit"),
            json: has_json(args),
        }
    }
}

/// Ruta + query de `runs list`, con la validación del `--status` hecha ACÁ.
fn ruta_list(o: &ListOpts) -> R<String> {
    let _ = o;
    Err(CliError::nuevo("sin implementar"))
}

async fn listar(client: &Client, o: &ListOpts) -> R<String> {
    let _ = (client, o);
    Err(CliError::nuevo("sin implementar"))
}

async fn mostrar(client: &Client, id: &str, json_out: bool) -> R<String> {
    let _ = (client, id, json_out);
    Err(CliError::nuevo("sin implementar"))
}

async fn reanudar(
    client: &Client,
    id: &str,
    respuesta: Option<&str>,
    por: Option<&str>,
    json_out: bool,
) -> R<String> {
    let _ = (client, id, respuesta, por, json_out);
    Err(CliError::nuevo("sin implementar"))
}

async fn cancelar(client: &Client, id: &str, json_out: bool) -> R<String> {
    let _ = (client, id, json_out);
    Err(CliError::nuevo("sin implementar"))
}

// ---------------------------------------------------------------------------
// Traducción de errores del server
// ---------------------------------------------------------------------------

/// Convierte un error HTTP del engine en algo accionable.
fn explicar_error(accion: &str, id: &str, status: u16, cuerpo: &Value) -> String {
    let _ = (accion, id, status, cuerpo);
    String::new()
}

/// `--response`: si el texto es JSON válido se manda tal cual; si no, como string.
fn valor_respuesta(texto: &str) -> Value {
    let _ = texto;
    Value::Null
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Tabla de runs: RUN · AGENTE · ESTADO · NODO · INICIO.
fn render_tabla(runs: &[Value]) -> String {
    let _ = runs;
    String::new()
}

fn print_runs_help() {
    println!(
        "\
{bold}mirai runs{reset} — ejecuciones de agentes: ver, reanudar y cancelar

{bold}USO:{reset}
    mirai runs list [--status S] [--agent <id>] [--limit N] [--json]
    mirai runs show <run_id> [--json]
    mirai runs resume <run_id> [--response <texto|JSON>] [--by <quién>] [--json]
    mirai runs cancel <run_id> [--json]
",
        bold = colors::BOLD,
        reset = colors::RESET,
    );
}

// ---------------------------------------------------------------------------
// Tests — EL CONTRATO (se escribieron antes que la implementación)
// ---------------------------------------------------------------------------
//
// Corren contra un **engine de mentira de verdad**: un servidor HTTP real sobre
// TCP que responde lo que responde el engine (mismos códigos y mismos cuerpos,
// copiados de `engine/src/server/lifecycle.rs` y `handlers.rs`). Así se prueba
// la URL que se arma, el método, el cuerpo que se manda y la traducción del
// error — sin mockear el cliente HTTP y sin depender de un server real.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // -- Engine de mentira ---------------------------------------------------

    #[derive(Debug, Clone)]
    struct Peticion {
        linea: String,
        cuerpo: Value,
    }

    struct FakeEngine {
        base_url: String,
        peticiones: Arc<Mutex<Vec<Peticion>>>,
    }

    impl FakeEngine {
        fn client(&self) -> Client {
            Client::nuevo(self.base_url.clone(), None)
        }
        fn peticiones(&self) -> Vec<Peticion> {
            self.peticiones.lock().unwrap().clone()
        }
    }

    /// Rutas: `(patrón que debe contener la línea de petición, status, cuerpo)`.
    /// Gana la primera que matchea; lo que no matchea es 404.
    async fn fake_engine(rutas: Vec<(&'static str, u16, Value)>) -> FakeEngine {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let peticiones = Arc::new(Mutex::new(Vec::new()));
        let log = peticiones.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let rutas = rutas.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf: Vec<u8> = Vec::new();
                    while let Some((linea, cuerpo)) = leer_peticion(&mut sock, &mut buf).await {
                        log.lock().unwrap().push(Peticion {
                            linea: linea.clone(),
                            cuerpo,
                        });
                        let (status, body) = rutas
                            .iter()
                            .find(|(patron, _, _)| linea.contains(patron))
                            .map(|(_, s, b)| (*s, b.clone()))
                            .unwrap_or((404, json!({"error": "ruta no cableada en el fake"})));
                        let bytes = serde_json::to_vec(&body).unwrap();
                        let head = format!(
                            "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                            razon(status),
                            bytes.len()
                        );
                        if sock.write_all(head.as_bytes()).await.is_err()
                            || sock.write_all(&bytes).await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });

        FakeEngine {
            base_url: format!("http://{addr}/api/v1"),
            peticiones,
        }
    }

    fn razon(status: u16) -> &'static str {
        match status {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            400 => "Bad Request",
            404 => "Not Found",
            409 => "Conflict",
            503 => "Service Unavailable",
            _ => "Internal Server Error",
        }
    }

    /// Lee una petición HTTP (línea + cuerpo JSON). `None` al cerrar la conexión.
    async fn leer_peticion(
        sock: &mut tokio::net::TcpStream,
        buf: &mut Vec<u8>,
    ) -> Option<(String, Value)> {
        loop {
            if let Some(fin) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let cabecera = String::from_utf8_lossy(&buf[..fin]).to_string();
                let largo = cabecera
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        (k.trim().eq_ignore_ascii_case("content-length"))
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if buf.len() >= fin + 4 + largo {
                    let cuerpo: Value = serde_json::from_slice(&buf[fin + 4..fin + 4 + largo])
                        .unwrap_or(Value::Null);
                    buf.drain(..fin + 4 + largo);
                    let linea = cabecera.lines().next().unwrap_or("").to_string();
                    return Some((linea, cuerpo));
                }
            }
            let mut tmp = [0u8; 4096];
            match sock.read(&mut tmp).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
        }
    }

    // -- Datos de ejemplo (con la forma REAL de la API) ----------------------

    fn run_pausado() -> Value {
        json!({
            "id": "run-pausado",
            "agent_id": "ag1",
            "agent_name": "revisor",
            "status": "Paused",
            "run_status": "paused",
            "current_node_id": "ask",
            "trace_len": 1,
            "error": null,
            "started_at": 1_754_500_000.0,
            "finished_at": null,
            "duration_ms": null,
        })
    }

    fn run_completado() -> Value {
        json!({
            "id": "run-ok",
            "agent_id": "ag2",
            "agent_name": "reportería",
            "status": "Completed",
            "run_status": "completed",
            "current_node_id": null,
            "trace_len": 3,
            "error": null,
            "started_at": 1_754_400_000.0,
            "finished_at": 1_754_400_012.0,
            "duration_ms": 12000,
        })
    }

    fn detalle_pausado() -> Value {
        json!({
            "id": "run-pausado",
            "status": "Paused",
            "run_status": "paused",
            "current_node_id": "ask",
            "since": 1_754_500_003.0,
            "resumable": true,
            "trace": [
                {"node_id": "trigger", "tool_type": "trigger/manual", "status": "Ok",
                 "duration_ms": 2, "retries": 0, "started_at": 1_754_500_000.0, "finished_at": 1_754_500_000.002}
            ],
            "error": null,
            "agent_id": "ag1",
            "agent_name": "revisor",
            "transcript": [],
            "state": {},
            "started_at": 1_754_500_000.0,
            "finished_at": null,
            "duration_ms": null,
        })
    }

    // -- list ----------------------------------------------------------------

    #[tokio::test]
    async fn list_pinta_una_tabla_alineada_con_id_agente_estado_nodo_e_inicio() {
        let fake = fake_engine(vec![(
            "GET /api/v1/sessions",
            200,
            json!([run_pausado(), run_completado()]),
        )])
        .await;

        let salida = listar(&fake.client(), &ListOpts::default()).await.unwrap();
        let lineas: Vec<&str> = salida.lines().collect();

        assert!(lineas[0].contains("RUN"), "cabecera: {salida}");
        assert!(lineas[0].contains("AGENTE"));
        assert!(lineas[0].contains("ESTADO"));
        assert!(lineas[0].contains("NODO"));
        assert!(lineas[0].contains("INICIO"));
        assert!(salida.contains("run-pausado"));
        assert!(salida.contains("paused"));
        assert!(salida.contains("ask"));

        // Alineación: la columna ESTADO arranca en el mismo carácter en todas
        // las filas (el nombre con tilde "reportería" no puede correrla).
        let col = columna(lineas[0], "ESTADO");
        assert_eq!(
            columna(lineas[1], "paused"),
            col,
            "fila 1 desalineada: {salida}"
        );
        assert_eq!(
            columna(lineas[2], "completed"),
            col,
            "fila 2 desalineada: {salida}"
        );
    }

    /// Índice en CARACTERES (no bytes) donde arranca `aguja` en `linea`.
    fn columna(linea: &str, aguja: &str) -> usize {
        let byte = linea
            .find(aguja)
            .unwrap_or_else(|| panic!("'{aguja}' no está en '{linea}'"));
        linea[..byte].chars().count()
    }

    #[tokio::test]
    async fn list_filtra_por_estado_pasandolo_en_la_query() {
        let fake = fake_engine(vec![("GET /api/v1/sessions", 200, json!([run_pausado()]))]).await;

        let opts = ListOpts {
            status: Some("paused".into()),
            ..Default::default()
        };
        let salida = listar(&fake.client(), &opts).await.unwrap();
        assert!(salida.contains("run-pausado"));

        let pedido = &fake.peticiones()[0].linea;
        assert!(
            pedido.contains("/api/v1/sessions?status=paused"),
            "el filtro tiene que viajar en la URL: {pedido}"
        );
    }

    #[tokio::test]
    async fn list_rechaza_un_estado_inventado_sin_ir_al_server() {
        let opts = ListOpts {
            status: Some("pausado".into()),
            ..Default::default()
        };
        let e = ruta_list(&opts).unwrap_err();
        assert!(e.mensaje.contains("pausado"), "{}", e.mensaje);
        assert!(
            e.mensaje.contains("paused") && e.mensaje.contains("running"),
            "tiene que decir cuáles valen: {}",
            e.mensaje
        );
    }

    #[tokio::test]
    async fn list_json_es_json_valido_y_parseable() {
        let fake = fake_engine(vec![("GET /api/v1/sessions", 200, json!([run_pausado()]))]).await;
        let opts = ListOpts {
            json: true,
            ..Default::default()
        };
        let salida = listar(&fake.client(), &opts).await.unwrap();
        let parseado: Value = serde_json::from_str(&salida).expect("--json tiene que ser JSON");
        assert_eq!(parseado[0]["id"], "run-pausado");
    }

    #[tokio::test]
    async fn list_sin_runs_lo_dice_en_vez_de_una_tabla_vacia() {
        let fake = fake_engine(vec![("GET /api/v1/sessions", 200, json!([]))]).await;
        let salida = listar(&fake.client(), &ListOpts::default()).await.unwrap();
        assert!(salida.to_lowercase().contains("no hay runs"), "{salida}");
    }

    // -- show ----------------------------------------------------------------

    #[tokio::test]
    async fn show_trae_estado_nodo_traza_y_el_motivo_de_la_pausa() {
        let fake = fake_engine(vec![
            ("GET /api/v1/sessions/run-pausado", 200, detalle_pausado()),
            (
                "GET /api/v1/agents/ag1/spec",
                200,
                json!({
                    "name": "revisor",
                    "graph": {"nodes": [
                        {"id": "ask", "tool_type": "logic/human_input",
                         "config": {"prompt": "¿publico el reporte?", "options": ["si", "no"]}}
                    ], "edges": []}
                }),
            ),
        ])
        .await;

        let salida = mostrar(&fake.client(), "run-pausado", false).await.unwrap();
        assert!(salida.contains("paused"), "estado: {salida}");
        assert!(salida.contains("ask"), "nodo actual: {salida}");
        assert!(salida.contains("revisor"), "agente: {salida}");
        assert!(salida.contains("trigger"), "traza: {salida}");
        // El motivo de la pausa: qué está preguntando (si la API lo expone).
        assert!(
            salida.contains("¿publico el reporte?"),
            "motivo de la pausa: {salida}"
        );
        // Y cómo seguir.
        assert!(
            salida.contains("mirai runs resume run-pausado"),
            "siguiente paso: {salida}"
        );
    }

    #[tokio::test]
    async fn show_de_un_run_fallido_explica_por_que_fallo() {
        let mut detalle = detalle_pausado();
        detalle["run_status"] = json!("failed");
        detalle["status"] = json!("Failed");
        detalle["current_node_id"] = json!("paso7");
        detalle["error"] = json!("bash exited with code 1");
        let fake = fake_engine(vec![("GET /api/v1/sessions/run-malo", 200, detalle)]).await;

        let salida = mostrar(&fake.client(), "run-malo", false).await.unwrap();
        assert!(salida.contains("failed"));
        assert!(salida.contains("bash exited with code 1"), "{salida}");
        assert!(salida.contains("paso7"), "{salida}");
    }

    #[tokio::test]
    async fn show_de_un_run_que_no_existe_no_vomita_json() {
        let fake = fake_engine(vec![(
            "GET /api/v1/sessions/fantasma",
            404,
            json!({"error": "Session not found"}),
        )])
        .await;

        let e = mostrar(&fake.client(), "fantasma", false)
            .await
            .unwrap_err();
        assert!(e.mensaje.contains("fantasma"), "{}", e.mensaje);
        assert!(e.mensaje.contains("no existe"), "{}", e.mensaje);
        assert!(
            !e.mensaje.contains('{'),
            "nada de JSON crudo: {}",
            e.mensaje
        );
        assert!(
            e.mensaje.contains("mirai runs list"),
            "qué hacer: {}",
            e.mensaje
        );
    }

    #[tokio::test]
    async fn show_json_devuelve_el_cuerpo_de_la_api_tal_cual() {
        let fake = fake_engine(vec![(
            "GET /api/v1/sessions/run-pausado",
            200,
            detalle_pausado(),
        )])
        .await;
        let salida = mostrar(&fake.client(), "run-pausado", true).await.unwrap();
        let parseado: Value = serde_json::from_str(&salida).expect("--json tiene que ser JSON");
        assert_eq!(parseado["run_status"], "paused");
        assert_eq!(parseado["current_node_id"], "ask");
    }

    // -- resume --------------------------------------------------------------

    #[tokio::test]
    async fn resume_manda_la_respuesta_humana_y_cuenta_desde_donde_siguio() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-pausado/resume",
            200,
            json!({
                "session_id": "run-pausado",
                "agent_id": "ag1",
                "agent_name": "revisor",
                "resumed_from": "fin",
                "skipped_nodes": ["trigger", "ask"],
                "status": "Completed",
                "run_status": "completed",
                "trace": [],
                "transcript": [],
                "state": {},
                "error": null,
            }),
        )])
        .await;

        let salida = reanudar(&fake.client(), "run-pausado", Some("si"), None, false)
            .await
            .unwrap();
        assert!(salida.contains("run-pausado"));
        assert!(salida.contains("completed"), "estado final: {salida}");
        assert!(salida.contains("fin"), "desde dónde siguió: {salida}");

        let pedido = &fake.peticiones()[0];
        assert!(pedido.linea.starts_with("POST"), "{}", pedido.linea);
        assert_eq!(
            pedido.cuerpo["response"], "si",
            "la respuesta humana viaja en el cuerpo"
        );
    }

    #[test]
    fn la_respuesta_se_manda_como_json_si_lo_es_y_como_texto_si_no() {
        assert_eq!(valor_respuesta("si"), json!("si"));
        assert_eq!(valor_respuesta("{\"ok\":true}"), json!({"ok": true}));
        assert_eq!(valor_respuesta("42"), json!(42));
    }

    #[tokio::test]
    async fn resume_con_db_pre_v3_traduce_el_409_a_algo_accionable() {
        // El cuerpo EXACTO que devuelve `lifecycle.rs` cuando `agents.spec` es
        // NULL (DB creada antes del esquema v3): no hay grafo que reanudar.
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-viejo/resume",
            409,
            json!({"error": "el agente 'ag-viejo' del run 'run-viejo' ya no está registrado: no hay grafo que reanudar"}),
        )])
        .await;

        let e = reanudar(&fake.client(), "run-viejo", None, None, false)
            .await
            .unwrap_err();

        assert!(
            !e.mensaje.contains('{'),
            "nada de JSON crudo: {}",
            e.mensaje
        );
        assert!(e.mensaje.contains("run-viejo"), "{}", e.mensaje);
        // Tiene que explicar la CAUSA (la spec no quedó guardada / DB vieja)…
        assert!(
            e.mensaje.contains("v3") || e.mensaje.to_lowercase().contains("no quedó guardada"),
            "tiene que decir POR QUÉ: {}",
            e.mensaje
        );
        // …y QUÉ HACER (relanzar: ese run no se recupera).
        assert!(
            e.mensaje.to_lowercase().contains("relanz")
                || e.mensaje.to_lowercase().contains("volvé"),
            "tiene que decir QUÉ HACER: {}",
            e.mensaje
        );
    }

    #[tokio::test]
    async fn resume_de_un_run_ya_terminado_lo_dice_en_castellano() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-ok/resume",
            409,
            json!({"error": "el run 'run-ok' no se puede reanudar (estado: completed)"}),
        )])
        .await;

        let e = reanudar(&fake.client(), "run-ok", None, None, false)
            .await
            .unwrap_err();
        assert!(e.mensaje.contains("completed"), "{}", e.mensaje);
        assert!(!e.mensaje.contains('{'), "{}", e.mensaje);
    }

    #[tokio::test]
    async fn resume_sin_db_dice_como_arrancar_el_server() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/x/resume",
            503,
            json!({"error": "reanudar necesita persistencia de runs y este server arrancó sin DB"}),
        )])
        .await;

        let e = reanudar(&fake.client(), "x", None, None, false)
            .await
            .unwrap_err();
        assert!(e.mensaje.contains("--db-path"), "qué hacer: {}", e.mensaje);
    }

    #[tokio::test]
    async fn resume_de_un_run_inexistente_da_mensaje_claro() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/fantasma/resume",
            404,
            json!({"error": "run 'fantasma' no existe"}),
        )])
        .await;

        let e = reanudar(&fake.client(), "fantasma", None, None, false)
            .await
            .unwrap_err();
        assert!(
            e.mensaje.contains("fantasma") && e.mensaje.contains("no existe"),
            "{}",
            e.mensaje
        );
        assert!(!e.mensaje.contains('{'), "{}", e.mensaje);
    }

    // -- cancel --------------------------------------------------------------

    #[tokio::test]
    async fn cancel_acepta_el_202_y_explica_que_es_cooperativo() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-vivo/cancel",
            202,
            json!({
                "session_id": "run-vivo",
                "status": "cancelling",
                "detail": "cancelación cooperativa: el run se detiene en la próxima frontera de nodo",
            }),
        )])
        .await;

        let salida = cancelar(&fake.client(), "run-vivo", false).await.unwrap();
        assert!(salida.contains("run-vivo"));
        assert!(
            salida.to_lowercase().contains("frontera de nodo"),
            "{salida}"
        );
        assert!(
            salida.contains("mirai runs show run-vivo"),
            "cómo comprobar: {salida}"
        );
        assert!(fake.peticiones()[0].linea.starts_with("POST"));
    }

    #[tokio::test]
    async fn cancel_de_un_run_que_no_esta_en_vuelo_explica_el_409() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-pausado/cancel",
            409,
            json!({"error": "el run 'run-pausado' no está en vuelo en este proceso (estado: paused)"}),
        )])
        .await;

        let e = cancelar(&fake.client(), "run-pausado", false)
            .await
            .unwrap_err();
        assert!(e.mensaje.contains("paused"), "{}", e.mensaje);
        assert!(
            !e.mensaje.contains('{'),
            "nada de JSON crudo: {}",
            e.mensaje
        );
    }

    #[tokio::test]
    async fn cancel_de_un_run_inexistente_da_mensaje_claro() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/fantasma/cancel",
            404,
            json!({"error": "run 'fantasma' no existe"}),
        )])
        .await;

        let e = cancelar(&fake.client(), "fantasma", false)
            .await
            .unwrap_err();
        assert!(
            e.mensaje.contains("fantasma") && e.mensaje.contains("no existe"),
            "{}",
            e.mensaje
        );
    }

    #[tokio::test]
    async fn cancel_json_devuelve_el_cuerpo_de_la_api() {
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-vivo/cancel",
            202,
            json!({"session_id": "run-vivo", "status": "cancelling"}),
        )])
        .await;
        let salida = cancelar(&fake.client(), "run-vivo", true).await.unwrap();
        let parseado: Value = serde_json::from_str(&salida).unwrap();
        assert_eq!(parseado["status"], "cancelling");
    }

    // -- server caído --------------------------------------------------------

    #[tokio::test]
    async fn sin_server_dice_que_arranque_mirai_serve() {
        // Puerto cerrado a propósito (se bindea y se suelta).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = Client::nuevo(format!("http://{addr}/api/v1"), None);
        let e = listar(&client, &ListOpts::default()).await.unwrap_err();
        assert!(e.mensaje.contains("mirai serve"), "{}", e.mensaje);
    }
}
