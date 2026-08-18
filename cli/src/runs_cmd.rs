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
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.header("X-API-Key", key);
        }
        if let Some(body) = cuerpo {
            req = req.json(&body);
        }
        let resp = req.send().await.map_err(|e| {
            CliError::nuevo(format!(
                "No hay engine en {} — ¿arrancaste `mirai serve`? ({e})",
                self.base_url.trim_end_matches("/api/v1")
            ))
        })?;
        let status = resp.status().as_u16();
        // Un cuerpo que no es JSON no es motivo para tumbar el comando: el
        // status manda y `explicar_error` sabe qué decir sin el cuerpo.
        let body = resp.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, body))
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

/// Los estados de ciclo de vida que entiende el engine
/// (`SessionStatus::parse_strict`, engine/src/db/repositories.rs).
const ESTADOS: [&str; 7] = [
    "running",
    "paused",
    "completed",
    "failed",
    "timeout",
    "cancelled",
    "interrupted",
];

/// Ruta + query de `runs list`, con la validación del `--status` hecha ACÁ.
///
/// El server también valida (400 con la lista de válidos), pero un typo no
/// merece un viaje de red: se rechaza en local diciendo cuáles valen. Si el
/// valor es bueno, el filtro viaja en la query — filtrar en el cliente sería
/// mentir con `--limit` (el server corta ANTES de que el filtro se aplique).
fn ruta_list(o: &ListOpts) -> R<String> {
    let mut query: Vec<String> = Vec::new();

    if let Some(estado) = &o.status {
        let normalizado = estado.trim().to_lowercase();
        if !ESTADOS.contains(&normalizado.as_str()) {
            return Err(CliError::nuevo(format!(
                "Estado '{estado}' desconocido. Los que valen: {}.",
                ESTADOS.join(" | ")
            )));
        }
        query.push(format!("status={normalizado}"));
    }
    if let Some(agente) = &o.agent_id {
        query.push(format!("agent_id={agente}"));
    }
    if let Some(limite) = &o.limit {
        limite
            .parse::<usize>()
            .map_err(|_| CliError::nuevo(format!("--limit '{limite}' no es un número.")))?;
        query.push(format!("limit={limite}"));
    }

    Ok(if query.is_empty() {
        "/sessions".to_string()
    } else {
        format!("/sessions?{}", query.join("&"))
    })
}

async fn listar(client: &Client, o: &ListOpts) -> R<String> {
    let ruta = ruta_list(o)?;
    let (status, body) = client.pedir(reqwest::Method::GET, &ruta, None).await?;
    if status != 200 {
        return Err(CliError::nuevo(explicar_error(
            "listar los runs",
            "",
            status,
            &body,
        )));
    }
    let runs = body.as_array().cloned().unwrap_or_default();

    if o.json {
        return Ok(pretty(&json!(runs)));
    }
    if runs.is_empty() {
        let filtro = o
            .status
            .as_deref()
            .map(|s| format!(" con estado '{s}'"))
            .unwrap_or_default();
        return Ok(format!(
            "{}No hay runs{filtro}. Ejecutá un agente y volvé a mirar.{}",
            colors::DIM,
            colors::RESET
        ));
    }

    Ok(format!(
        "{}{}mirar uno: mirai runs show <run_id>{}",
        render_tabla(&runs),
        colors::DIM,
        colors::RESET
    ))
}

async fn mostrar(client: &Client, id: &str, json_out: bool) -> R<String> {
    let (status, run) = client
        .pedir(reqwest::Method::GET, &format!("/sessions/{id}"), None)
        .await?;
    if status != 200 {
        return Err(CliError::nuevo(explicar_error(
            "ver el run",
            id,
            status,
            &run,
        )));
    }
    if json_out {
        // Crudo a propósito: lo que consume un script es el contrato de la API,
        // no nuestro formato bonito (que sí puede cambiar).
        return Ok(pretty(&run));
    }

    // Motivo de la pausa: la pregunta que dejó colgado al run. La API de runs
    // no la expone (el `interrupt_info` no sobrevive a la DB), así que se saca
    // de la spec del agente. Best-effort: si el agente ya no está registrado se
    // muestra el detalle igual, sin la pregunta.
    let pregunta = if texto(&run, "run_status") == "paused" {
        pregunta_pendiente(client, &run).await
    } else {
        None
    };

    Ok(render_detalle(id, &run, pregunta.as_ref()))
}

async fn reanudar(
    client: &Client,
    id: &str,
    respuesta: Option<&str>,
    por: Option<&str>,
    json_out: bool,
) -> R<String> {
    let mut cuerpo = json!({});
    if let Some(r) = respuesta {
        cuerpo["response"] = valor_respuesta(r);
    }
    if let Some(p) = por {
        cuerpo["responded_by"] = json!(p);
    }

    let (status, body) = client
        .pedir(
            reqwest::Method::POST,
            &format!("/sessions/{id}/resume"),
            Some(cuerpo),
        )
        .await?;
    if !(200..300).contains(&status) {
        return Err(CliError::nuevo(explicar_error(
            "reanudar", id, status, &body,
        )));
    }
    if json_out {
        return Ok(pretty(&body));
    }

    let final_ = texto(&body, "run_status");
    let desde = texto(&body, "resumed_from");
    let saltados: Vec<String> = body["skipped_nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let corridos = body["trace"].as_array().map(|a| a.len()).unwrap_or(0);

    let mut out = format!(
        "{}Run {id} reanudado desde '{desde}' → {}{}\n",
        colors::GREEN,
        color_estado(&final_),
        colors::RESET
    );
    out.push_str(&campo(
        "Saltados",
        &if saltados.is_empty() {
            "-".to_string()
        } else {
            format!("{} (no se re-ejecutaron)", saltados.join(", "))
        },
    ));
    out.push_str(&campo(
        "Ejecutados",
        &format!("{corridos} nodo(s) en esta reanudación"),
    ));

    // Un run que vuelve a pausarse pidió OTRA cosa: hay que decirlo, si no
    // parece que terminó.
    match final_.as_str() {
        "paused" => {
            out.push_str(&campo(
                "Ojo",
                "volvió a pausarse: hay otra pregunta pendiente",
            ));
            out.push_str(&campo("Seguir", &format!("mirai runs show {id}")));
        }
        "failed" | "timeout" => {
            // Reanudar "salió bien" (HTTP 200) pero el run terminó mal: exit
            // code 1, si no un `resume && deploy` mentiría.
            let motivo = texto(&body, "error");
            return Err(CliError::nuevo(format!(
                "El run {id} se reanudó desde '{desde}' y terminó en {final_}: {}\n  Detalle: mirai runs show {id}",
                if motivo.is_empty() { "sin detalle".into() } else { motivo }
            )));
        }
        _ => out.push_str(&campo("Detalle", &format!("mirai runs show {id}"))),
    }
    Ok(out.trim_end().to_string())
}

async fn cancelar(client: &Client, id: &str, json_out: bool) -> R<String> {
    let (status, body) = client
        .pedir(
            reqwest::Method::POST,
            &format!("/sessions/{id}/cancel"),
            None,
        )
        .await?;
    if !(200..300).contains(&status) {
        return Err(CliError::nuevo(explicar_error(
            "cancelar", id, status, &body,
        )));
    }
    if json_out {
        return Ok(pretty(&body));
    }
    Ok(format!(
        "{}Cancelación pedida para el run {id}.{}\n{}{}",
        colors::GREEN,
        colors::RESET,
        campo(
            "Cómo para",
            "es cooperativo: el run se detiene en la próxima frontera de nodo",
        ),
        campo("Comprobar", &format!("mirai runs show {id}")),
    )
    .trim_end()
    .to_string())
}

/// La pregunta que dejó pausado al run, sacada de la spec del agente.
async fn pregunta_pendiente(client: &Client, run: &Value) -> Option<(String, Vec<String>)> {
    let agente = run.get("agent_id").and_then(|v| v.as_str())?;
    let nodo = run.get("current_node_id").and_then(|v| v.as_str())?;
    let (status, spec) = client
        .pedir(
            reqwest::Method::GET,
            &format!("/agents/{agente}/spec"),
            None,
        )
        .await
        .ok()?;
    if status != 200 {
        return None;
    }
    let def = spec["graph"]["nodes"]
        .as_array()?
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(nodo))?;
    let prompt = def["config"]["prompt"].as_str()?.to_string();
    let opciones = def["config"]["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some((prompt, opciones))
}

// ---------------------------------------------------------------------------
// Traducción de errores del server
// ---------------------------------------------------------------------------

/// Convierte un error HTTP del engine en algo accionable.
///
/// Regla: **nunca** volcar el JSON crudo. El operador no depura el engine, está
/// tratando de mover un run — necesita causa y siguiente paso, no un `{...}`.
fn explicar_error(accion: &str, id: &str, status: u16, cuerpo: &Value) -> String {
    let servidor = cuerpo
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    match status {
        401 | 403 => format!(
            "El engine pide API key para {accion}. Exportá MIRAI_API_KEY con la misma clave con la que arrancó `mirai serve`."
        ),
        404 => format!(
            "El run '{id}' no existe. Mirá los que hay con `mirai runs list`."
        ),
        400 => format!("El engine rechazó la petición: {}", detalle(&servidor)),
        503 => format!(
            "Este `mirai serve` arrancó SIN base de datos de runs, así que no puede {accion}.\n  \
             Qué hacer: reinicialo con `mirai serve --db-path ~/.openmirai/engine.db` (es el default) y volvé a intentar."
        ),
        409 if servidor.contains("no hay grafo que reanudar") => {
            let agente = entre_comillas(&servidor).unwrap_or_else(|| "?".to_string());
            format!(
                "No se puede reanudar el run '{id}': el engine no tiene la definición del agente '{agente}', así que no hay grafo que retomar.\n  \
                 Por qué: ese run viene de una base anterior al esquema v3, donde la spec del agente no quedó guardada (columna agents.spec vacía).\n  \
                 Qué hacer: ese run no se recupera — volvé a lanzarlo (`mirai run <spec.yaml>` o POST /api/v1/agents/<agent_id>/execute).\n  \
                 Los runs creados con esta versión sí guardan su spec: esos se reanudan aunque reinicies el proceso."
            )
        }
        409 if servidor.contains("no dejó estado guardado") => format!(
            "El run '{id}' no dejó checkpoint: no hay desde dónde retomarlo.\n  \
             Por qué: murió antes de terminar su primer nodo, o viene de una base anterior al esquema v2 (sin checkpoints).\n  \
             Qué hacer: relanzalo desde cero; no hay estado que rescatar."
        ),
        409 if servidor.contains("no se puede reanudar") => {
            let estado = entre_parentesis(&servidor).unwrap_or_else(|| "terminal".to_string());
            format!(
                "El run '{id}' no se puede reanudar: está en {estado}. Solo se reanuda lo que sigue vivo (paused) o lo que falló (failed).\n  \
                 Cómo quedó: mirai runs show {id}"
            )
        }
        409 if servidor.contains("no está en vuelo") => {
            let estado = entre_parentesis(&servidor).unwrap_or_else(|| "terminado".to_string());
            format!(
                "El run '{id}' no está corriendo en este engine (está en {estado}): no hay nada que cancelar.\n  \
                 Ojo: cancelar solo aplica a un run EN VUELO. Uno pausado se reanuda (`mirai runs resume {id}`) o se deja quieto."
            )
        }
        409 => format!(
            "No se pudo {accion} el run '{id}': {}",
            detalle(&servidor)
        ),
        _ => format!(
            "El engine respondió {status} al {accion} el run '{id}': {}",
            detalle(&servidor)
        ),
    }
}

fn detalle(servidor: &str) -> String {
    if servidor.is_empty() {
        "sin detalle".to_string()
    } else {
        servidor.to_string()
    }
}

/// Lo que va entre las primeras comillas simples de un mensaje del engine.
fn entre_comillas(msg: &str) -> Option<String> {
    msg.split('\'').nth(1).map(str::to_string)
}

/// El estado que el engine reporta como `(estado: X)`.
fn entre_parentesis(msg: &str) -> Option<String> {
    let (_, resto) = msg.split_once("(estado:")?;
    let (valor, _) = resto.split_once(')')?;
    Some(valor.trim().to_string())
}

/// `--response`: si el texto es JSON válido se manda tal cual; si no, como string.
///
/// Así `--response si` manda `"si"` y `--response '{"aprobado":true}'` manda el
/// objeto — sin obligar a escribir JSON para contestar "si".
fn valor_respuesta(texto: &str) -> Value {
    serde_json::from_str(texto).unwrap_or_else(|_| Value::String(texto.to_string()))
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Tabla de runs: RUN · AGENTE · ESTADO · NODO · INICIO.
///
/// Sin color en las celdas a propósito: la tabla se pipea a `grep`/`awk`, y los
/// códigos ANSI dentro de una columna rompen tanto el filtro como el ancho.
fn render_tabla(runs: &[Value]) -> String {
    let cabeceras = ["RUN", "AGENTE", "ESTADO", "NODO", "INICIO"];
    let filas: Vec<[String; 5]> = runs
        .iter()
        .map(|r| {
            [
                celda(r, "id"),
                celda(r, "agent_name"),
                celda(r, "run_status"),
                celda(r, "current_node_id"),
                r.get("started_at")
                    .and_then(|v| v.as_f64())
                    .map(fecha_corta)
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();

    // Ancho en CARACTERES: "reportería" mide 10, no 11 (que es lo que mide en
    // bytes) — medir en bytes desalinea cualquier nombre con tilde.
    let mut anchos: Vec<usize> = cabeceras.iter().map(|h| h.chars().count()).collect();
    for fila in &filas {
        for (i, valor) in fila.iter().enumerate() {
            anchos[i] = anchos[i].max(valor.chars().count());
        }
    }

    let mut out = String::new();
    for (i, h) in cabeceras.iter().enumerate() {
        out.push_str(&rellenar(h, anchos[i]));
        out.push_str("  ");
    }
    out.push('\n');
    for fila in &filas {
        for (i, valor) in fila.iter().enumerate() {
            out.push_str(&rellenar(valor, anchos[i]));
            out.push_str("  ");
        }
        out.push('\n');
    }
    out
}

/// Detalle de un run: estado, dónde quedó, por qué, y cómo seguir.
fn render_detalle(id: &str, run: &Value, pregunta: Option<&(String, Vec<String>)>) -> String {
    let estado = texto(run, "run_status");
    let nodo = texto(run, "current_node_id");
    let reanudable = run.get("resumable").and_then(|v| v.as_bool()) == Some(true);

    let mut out = format!("{}RUN {id}{}\n", colors::BOLD, colors::RESET);
    out.push_str(&campo(
        "Estado",
        &format!(
            "{}{}",
            color_estado(&estado),
            if reanudable { " (reanudable)" } else { "" }
        ),
    ));

    let agente = texto(run, "agent_name");
    let agente_id = texto(run, "agent_id");
    if !agente.is_empty() || !agente_id.is_empty() {
        out.push_str(&campo("Agente", &format!("{agente} ({agente_id})")));
    }
    if !nodo.is_empty() {
        out.push_str(&campo(
            "Nodo",
            &format!(
                "{nodo}{}",
                match estado.as_str() {
                    "paused" => "  (acá quedó esperando)",
                    "failed" | "timeout" => "  (acá falló)",
                    // El runner cancela en FRONTERA de nodo: el que quedó
                    // apuntado es el que ya no llegó a ejecutarse.
                    "cancelled" => "  (se detuvo antes de entrar acá)",
                    "running" => "  (acá va)",
                    _ => "",
                }
            ),
        ));
    }

    // Por qué está donde está.
    match estado.as_str() {
        "paused" => {
            let motivo = match pregunta {
                Some((prompt, opciones)) if !opciones.is_empty() => {
                    format!(
                        "esperando a un humano — \"{prompt}\"  [{}]",
                        opciones.join(" | ")
                    )
                }
                Some((prompt, _)) => format!("esperando a un humano — \"{prompt}\""),
                None => "esperando a un humano (la pregunta ya no está en el engine)".to_string(),
            };
            out.push_str(&campo("Motivo", &motivo));
        }
        "failed" | "timeout" => {
            let e = texto(run, "error");
            out.push_str(&campo(
                "Motivo",
                if e.is_empty() { "sin detalle" } else { &e },
            ));
        }
        "cancelled" => out.push_str(&campo("Motivo", "cancelado a pedido")),
        _ => {}
    }

    if let Some(inicio) = run.get("started_at").and_then(|v| v.as_f64()) {
        out.push_str(&campo(
            "Inicio",
            &format!("{} ({})", fecha(inicio), hace(inicio)),
        ));
    }
    if let Some(desde) = run.get("since").and_then(|v| v.as_f64()) {
        // La etiqueta sale del ESTADO, no de `finished_at`: un run pausado
        // también trae `finished_at` (el motor registra el resultado de la
        // pausa), y llamarle "Fin" a un run que sigue vivo es mentir.
        let etiqueta = match estado.as_str() {
            "running" | "paused" => "Ahí desde",
            _ => "Fin",
        };
        out.push_str(&campo(
            etiqueta,
            &format!("{} ({})", fecha(desde), hace(desde)),
        ));
    }
    if let Some(ms) = run.get("duration_ms").and_then(|v| v.as_u64()) {
        out.push_str(&campo("Duró", &format!("{:.1}s", ms as f64 / 1000.0)));
    }

    // Historial: la traza es lo que la API expone como "qué pasó" nodo a nodo.
    let traza = run["trace"].as_array().cloned().unwrap_or_default();
    if !traza.is_empty() {
        out.push_str(&campo("Traza", &format!("{} nodo(s)", traza.len())));
        for (i, t) in traza.iter().enumerate() {
            let err = t
                .get("error")
                .and_then(|v| v.as_str())
                .map(|e| format!("  ← {e}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "    {:>2}  {:<22} {:<20} {:<8} {:>7} ms{err}\n",
                i + 1,
                texto(t, "node_id"),
                texto(t, "tool_type"),
                texto(t, "status"),
                t.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            ));
        }
    }

    // Cómo seguir.
    let siguiente = match estado.as_str() {
        "paused" => {
            let ejemplo = pregunta
                .and_then(|(_, o)| o.first().cloned())
                .unwrap_or_else(|| "tu respuesta".to_string());
            Some(format!("mirai runs resume {id} --response \"{ejemplo}\""))
        }
        "failed" | "timeout" if reanudable => Some(format!("mirai runs resume {id}")),
        "running" => Some(format!("mirai runs cancel {id}")),
        _ => None,
    };
    if let Some(cmd) = siguiente {
        out.push_str(&campo("Seguir", &cmd));
    }
    out.trim_end().to_string()
}

/// Una línea "  Etiqueta   valor" con la etiqueta atenuada y alineada.
fn campo(etiqueta: &str, valor: &str) -> String {
    format!(
        "  {}{}{}  {valor}\n",
        colors::DIM,
        rellenar(etiqueta, 11),
        colors::RESET
    )
}

/// Rellena a la derecha contando CARACTERES (no bytes).
fn rellenar(texto: &str, ancho: usize) -> String {
    let largo = texto.chars().count();
    format!("{texto}{}", " ".repeat(ancho.saturating_sub(largo)))
}

fn color_estado(estado: &str) -> String {
    let color = match estado {
        "running" => colors::CYAN,
        "paused" => colors::YELLOW,
        "completed" => colors::GREEN,
        "failed" | "timeout" => colors::RED,
        _ => colors::DIM,
    };
    format!("{color}{estado}{}", colors::RESET)
}

fn celda(v: &Value, clave: &str) -> String {
    v.get(clave)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn texto(v: &Value, clave: &str) -> String {
    v.get(clave)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// Epoch (segundos, como lo devuelve la API) → hora local legible.
fn fecha(epoch: f64) -> String {
    match chrono::DateTime::from_timestamp(epoch as i64, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        None => "-".to_string(),
    }
}

fn fecha_corta(epoch: f64) -> String {
    match chrono::DateTime::from_timestamp(epoch as i64, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "-".to_string(),
    }
}

/// "hace 3m" — lo que uno realmente quiere saber de un run en vuelo.
fn hace(epoch: f64) -> String {
    let delta = (chrono::Utc::now().timestamp() - epoch as i64).max(0);
    match delta {
        0..=59 => format!("hace {delta}s"),
        60..=3599 => format!("hace {}m", delta / 60),
        3600..=86_399 => format!("hace {}h", delta / 3600),
        _ => format!("hace {}d", delta / 86_400),
    }
}

fn print_runs_help() {
    println!(
        "\
{bold}mirai runs{reset} — ejecuciones de agentes: ver en qué van, reanudarlas y cancelarlas

{bold}USO:{reset}
    mirai runs list [--status S] [--agent <id>] [--limit N] [--json]
                                      Lista los runs (id, agente, estado, nodo, inicio)
    mirai runs show <run_id> [--json] Detalle: estado, dónde quedó, por qué y la traza
    mirai runs resume <run_id> [--response <texto|JSON>] [--by <quién>] [--json]
                                      Reanuda un run pausado o fallido, desde donde quedó
    mirai runs cancel <run_id> [--json]
                                      Cancela un run EN VUELO (para en frontera de nodo)

{bold}ESTADOS (--status):{reset}
    running | paused | completed | failed | timeout | cancelled | interrupted

{bold}SERVER:{reset}
    --host <h> / --port <p>   Dónde corre mirai serve (default 127.0.0.1:3000)
    MIRAI_HOST / MIRAI_PORT   Equivalentes por variable de entorno
    MIRAI_API_KEY             Se manda como X-API-Key si está seteada

{bold}EJEMPLOS:{reset}
    mirai runs list --status paused              Los que esperan a un humano
    mirai runs show r7d3f1a2                     Qué está preguntando y dónde quedó
    mirai runs resume r7d3f1a2 --response \"si\"    Contestar y seguir desde el nodo siguiente
    mirai runs resume r7d3f1a2                   Retomar uno que falló (sin respuesta humana)
    mirai runs cancel r7d3f1a2                   Frenar uno en vuelo
    mirai runs list --json | jq '.[].id'         Para scripts

{bold}OJO:{reset}
    `mirai runs` son las ejecuciones del MOTOR sobre un grafo.
    `mirai sessions` son las sesiones de Claude en tmux — otro subsistema.
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
                {"node_id": "trigger", "tool_type": "trigger/manual", "status": "ok",
                 "duration_ms": 2, "retries": 0, "started_at": 1_754_500_000.0, "finished_at": 1_754_500_000.002}
            ],
            "error": null,
            "agent_id": "ag1",
            "agent_name": "revisor",
            "transcript": [],
            "state": {},
            "started_at": 1_754_500_000.0,
            // Un run PAUSADO también trae `finished_at`: el motor registra el
            // resultado de la pausa. Verificado contra un engine real.
            "finished_at": 1_754_500_003.0,
            "duration_ms": 3000,
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
        // Un run pausado sigue VIVO: nada de rotularlo "Fin" porque la API le
        // haya puesto `finished_at` al registrar la pausa.
        assert!(salida.contains("Ahí desde"), "{salida}");
        assert!(
            !salida.contains("Fin  "),
            "un run vivo no terminó: {salida}"
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

    #[tokio::test]
    async fn resume_que_termina_fallando_no_puede_salir_con_exito() {
        // El HTTP salió 200 (reanudó bien) pero el run terminó en `failed`:
        // devolver Ok haría que `mirai runs resume X && desplegar` mintiera.
        let fake = fake_engine(vec![(
            "POST /api/v1/sessions/run-malo/resume",
            200,
            json!({
                "session_id": "run-malo",
                "resumed_from": "paso7",
                "skipped_nodes": ["a", "b"],
                "status": "Failed",
                "run_status": "failed",
                "trace": [],
                "error": "bash exited with code 1",
            }),
        )])
        .await;

        let e = reanudar(&fake.client(), "run-malo", None, None, false)
            .await
            .unwrap_err();
        assert!(e.mensaje.contains("failed"), "{}", e.mensaje);
        assert!(
            e.mensaje.contains("bash exited with code 1"),
            "{}",
            e.mensaje
        );
        assert!(e.mensaje.contains("paso7"), "{}", e.mensaje);
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
