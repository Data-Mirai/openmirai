//! Interactive setup wizard — detect providers, pick model, configure session.
//!
//! Ported from Python's `setup_wizard.py`. Uses crossterm for raw-mode
//! arrow-key selection.

#![allow(dead_code)]

use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    style::Print,
    terminal::{self, ClearType},
};

use crate::colors::*;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// A detected model available for selection.
#[derive(Debug, Clone)]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_window: Option<u32>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub local: bool,
}

impl ModelOption {
    pub fn ctx_label(&self) -> String {
        match self.context_window {
            Some(ctx) if ctx >= 1_000_000 => format!("{}M", ctx / 1_000_000),
            Some(ctx) => format!("{}K", ctx / 1000),
            None => "?".to_string(),
        }
    }

    pub fn modality_label(&self) -> &str {
        if self.supports_vision {
            "multimodal"
        } else {
            "text"
        }
    }
}

/// Final session configuration produced by the wizard.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub provider: String,
    pub model: String,
    pub autonomy_level: String,
    pub max_tool_rounds: u32,
    pub confirm_writes: bool,
    pub context_window: Option<u32>,
    pub supports_vision: bool,
    pub temperature: f32,
    pub max_tokens: u32,
}

/// Autonomy level definition.
struct AutonomyLevel {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    max_tool_rounds: u32,
    confirm_writes: bool,
}

const AUTONOMY_LEVELS: &[AutonomyLevel] = &[
    AutonomyLevel {
        key: "assisted",
        name: "Assisted",
        description: "Pregunta-respuesta. El humano lidera cada paso.",
        max_tool_rounds: 1,
        confirm_writes: true,
    },
    AutonomyLevel {
        key: "copilot",
        name: "Copilot",
        description: "Humano pide, agente ejecuta N pasos, humano revisa.",
        max_tool_rounds: 25,
        confirm_writes: false,
    },
    AutonomyLevel {
        key: "autopilot",
        name: "Autopilot",
        description: "Agente reacciona a eventos, humano supervisa.",
        max_tool_rounds: 50,
        confirm_writes: false,
    },
    AutonomyLevel {
        key: "self_driving",
        name: "Self-Driving",
        description: "Agente persigue objetivos, humano observa.",
        max_tool_rounds: 100,
        confirm_writes: false,
    },
];

fn get_autonomy(key: &str) -> &'static AutonomyLevel {
    AUTONOMY_LEVELS
        .iter()
        .find(|l| l.key == key)
        .unwrap_or(&AUTONOMY_LEVELS[1]) // copilot default
}

// ---------------------------------------------------------------------------
// Arrow-key selector (crossterm raw mode)
// ---------------------------------------------------------------------------

/// Interactive selector with arrow keys. Each option is (display_name, value).
///
/// Returns the `value` string of the selected option.
pub fn select_option(prompt: &str, options: &[(String, String)]) -> Option<String> {
    if options.is_empty() {
        return None;
    }

    let mut selected: usize = 0;
    let mut stdout = io::stdout();

    // Print prompt
    print!("\n  {BOLD}{prompt}{RESET}\n");
    print!("  {DIM}(flechas para navegar, Enter para seleccionar){RESET}\n\n");
    let _ = stdout.flush();

    // Enter raw mode for key capture
    terminal::enable_raw_mode().ok()?;

    // Render options initially
    render_options(&mut stdout, options, selected);

    loop {
        // Poll for key events
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
                match code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected < options.len() - 1 {
                            selected += 1;
                        }
                    }
                    KeyCode::Enter => {
                        terminal::disable_raw_mode().ok();
                        // Clear the options and show the selected value
                        clear_options(&mut stdout, options.len());
                        println!("  {GREEN}{BOLD}{}{RESET}", options[selected].0);
                        let _ = stdout.flush();
                        return Some(options[selected].1.clone());
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        terminal::disable_raw_mode().ok();
                        clear_options(&mut stdout, options.len());
                        return None;
                    }
                    _ => {}
                }
                // Redraw
                clear_options(&mut stdout, options.len());
                render_options(&mut stdout, options, selected);
            }
        }
    }
}

fn render_options(stdout: &mut io::Stdout, options: &[(String, String)], selected: usize) {
    for (i, (label, _)) in options.iter().enumerate() {
        if i == selected {
            let _ = execute!(
                stdout,
                Print(format!("  {CYAN}{BOLD}  -> {label}{RESET}\r\n"))
            );
        } else {
            let _ = execute!(stdout, Print(format!("  {DIM}     {label}{RESET}\r\n")));
        }
    }
    let _ = stdout.flush();
}

fn clear_options(stdout: &mut io::Stdout, count: usize) {
    for _ in 0..count {
        let _ = execute!(
            stdout,
            cursor::MoveUp(1),
            terminal::Clear(ClearType::CurrentLine)
        );
    }
    let _ = stdout.flush();
}

// ---------------------------------------------------------------------------
// Vision model detection patterns
// ---------------------------------------------------------------------------

const VISION_PATTERNS: &[&str] = &[
    "llava",
    "vision",
    "bakllava",
    "moondream",
    "minicpm-v",
    "cogvlm",
    "gpt-4o",
    "gpt-4-turbo",
    "gemini",
    "claude-3",
    "pixtral",
];

fn is_vision_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    VISION_PATTERNS.iter().any(|p| lower.contains(p))
}

// ---------------------------------------------------------------------------
// Ollama detection
// ---------------------------------------------------------------------------

fn is_ollama_installed() -> bool {
    which("ollama")
}

fn which(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check Ollama health via HTTP. Returns "ok", "not_running", "no_models", or "error:...".
async fn ollama_health(base_url: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let url = format!("{base_url}/api/tags");
    match client.get(&url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return format!("error:Ollama respondio status {}", resp.status());
            }
            match resp.json::<serde_json::Value>().await {
                Ok(data) => {
                    let models = data
                        .get("models")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    if models > 0 {
                        "ok".to_string()
                    } else {
                        "no_models".to_string()
                    }
                }
                Err(e) => format!("error:{e}"),
            }
        }
        Err(e) => {
            if e.is_timeout() {
                "error:timeout conectando a Ollama".to_string()
            } else if e.is_connect() {
                "not_running".to_string()
            } else {
                format!("error:{e}")
            }
        }
    }
}

/// Detect models from a running Ollama instance.
async fn detect_ollama_models(base_url: &str) -> Vec<ModelOption> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let url = format!("{base_url}/api/tags");
    let resp = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };

    let data: serde_json::Value = match resp.json().await {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let models_arr = match data.get("models").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut models = Vec::new();
    for m in models_arr {
        let name = m
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let details = m.get("details").cloned().unwrap_or(serde_json::json!({}));
        let param_size = details
            .get("parameter_size")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Try to get context window from /api/show
        let ctx = get_ollama_context_length(&client, base_url, &name).await;

        let display = if param_size.is_empty() {
            name.clone()
        } else {
            format!("{name} ({param_size})")
        };

        models.push(ModelOption {
            id: name.clone(),
            name: display,
            provider: "ollama".to_string(),
            context_window: ctx,
            supports_tools: true,
            supports_vision: is_vision_model(&name),
            local: true,
        });
    }

    models
}

async fn get_ollama_context_length(
    client: &reqwest::Client,
    base_url: &str,
    model_name: &str,
) -> Option<u32> {
    let url = format!("{base_url}/api/show");
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "name": model_name }))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let info = data.get("model_info")?.as_object()?;
    for (key, value) in info {
        if key.ends_with(".context_length") {
            return value.as_u64().map(|v| v as u32);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Ollama system helpers (install, start, pull)
// ---------------------------------------------------------------------------

fn try_start_ollama() -> String {
    // Spawn ollama serve in background
    let _ = Command::new("ollama")
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    let spinner = ["*", "o", "O", "@", "*"];

    // Poll health check -- Ollama needs a moment to bind the port
    for i in 0..12 {
        std::thread::sleep(Duration::from_millis(500));
        let frame = spinner[i % spinner.len()];
        let secs = ((i + 1) as f64) * 0.5;
        print!("\r  {MAGENTA}{frame}{RESET} {DIM}Iniciando Ollama... ({secs:.0}s){RESET}  ");
        let _ = io::stdout().flush();

        let status =
            tokio::runtime::Handle::current().block_on(ollama_health("http://localhost:11434"));
        if status == "ok" || status == "no_models" {
            print!("\r{}\r", " ".repeat(50));
            let _ = io::stdout().flush();
            return status;
        }
    }
    print!("\r{}\r", " ".repeat(50));
    let _ = io::stdout().flush();
    "not_running".to_string()
}

fn install_ollama() -> bool {
    let system = std::env::consts::OS;
    match system {
        "macos" => {
            if which("brew") {
                println!("\n  {DIM}Ejecutando: brew install ollama{RESET}\n");
                Command::new("brew")
                    .args(["install", "ollama"])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            } else {
                println!("\n  {YELLOW}Homebrew no encontrado.{RESET}");
                println!("  {DIM}Descarga Ollama desde: https://ollama.com/download{RESET}");
                false
            }
        }
        "linux" => {
            println!("\n  {DIM}Ejecutando instalador de Ollama...{RESET}\n");
            Command::new("sh")
                .args(["-c", "curl -fsSL https://ollama.com/install.sh | sh"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        _ => {
            println!("\n  {YELLOW}Descarga Ollama desde: https://ollama.com/download{RESET}");
            false
        }
    }
}

fn pull_ollama_model(model_name: &str) -> bool {
    println!("\n  {DIM}Descargando {model_name}...{RESET}\n");
    Command::new("ollama")
        .args(["pull", model_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Suggested models for first-time Ollama setup
// ---------------------------------------------------------------------------

fn suggested_models() -> Vec<(String, String)> {
    vec![
        (
            "qwen3:8b -- 8B, rapido, recomendado (~5 GB)".to_string(),
            "qwen3:8b".to_string(),
        ),
        (
            "llama3.2 -- 3B, ligero (~2 GB)".to_string(),
            "llama3.2".to_string(),
        ),
        (
            "gemma4 -- 8B, Google (~5 GB)".to_string(),
            "gemma4".to_string(),
        ),
        (
            "deepseek-r1:8b -- 8B, razonamiento (~5 GB)".to_string(),
            "deepseek-r1:8b".to_string(),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Provider setup: Local (Ollama)
// ---------------------------------------------------------------------------

async fn setup_local_provider() -> Option<ModelOption> {
    let base_url = "http://localhost:11434";

    loop {
        // --- Check 1: Is Ollama installed? ---
        if !is_ollama_installed() {
            println!("\n  {RED}x Ollama no esta instalado{RESET}");

            let mut choices: Vec<(String, String)> = Vec::new();

            let system = std::env::consts::OS;
            if system == "macos" && which("brew") {
                println!("  {DIM}Se puede instalar con Homebrew{RESET}");
                choices.push((
                    "Instalar Ollama ahora (brew install ollama)".to_string(),
                    "install".to_string(),
                ));
            } else if system == "linux" {
                println!("  {DIM}Se puede instalar automaticamente{RESET}");
                choices.push(("Instalar Ollama ahora".to_string(), "install".to_string()));
            } else {
                println!("  {DIM}Descarga: https://ollama.com/download{RESET}");
            }

            choices.push((
                "Reintentar (ya lo instale)".to_string(),
                "retry".to_string(),
            ));
            choices.push(("Cambiar a Cloud".to_string(), "cloud".to_string()));
            choices.push(("Salir".to_string(), "exit".to_string()));

            match select_option("Siguiente paso", &choices)?.as_str() {
                "install" => {
                    if install_ollama() {
                        println!("\n  {GREEN}+ Ollama instalado{RESET}");
                    } else {
                        println!("\n  {RED}x Error instalando Ollama{RESET}");
                    }
                    continue;
                }
                "retry" => continue,
                "cloud" => return setup_cloud_provider(),
                _ => return None,
            }
        }

        // --- Check 2: Is Ollama running? ---
        println!("\n  {DIM}Conectando con Ollama...{RESET}");
        let mut status = ollama_health(base_url).await;

        if status == "not_running" {
            println!("  {YELLOW}Ollama instalado pero no esta corriendo{RESET}");
            status = try_start_ollama();
            if status == "ok" || status == "no_models" {
                println!("  {GREEN}+ Ollama iniciado{RESET}");
            } else {
                println!("  {RED}x No se pudo iniciar Ollama{RESET}");
                println!("  {DIM}Inicialo manualmente: ollama serve{RESET}");
                let retry_choices = vec![
                    ("Reintentar".to_string(), "retry".to_string()),
                    ("Cambiar a Cloud".to_string(), "cloud".to_string()),
                    ("Salir".to_string(), "exit".to_string()),
                ];
                match select_option("Siguiente paso", &retry_choices)?.as_str() {
                    "retry" => continue,
                    "cloud" => return setup_cloud_provider(),
                    _ => return None,
                }
            }
        }

        // --- Check 3: Does Ollama have models? ---
        if status == "ok" {
            let models = detect_ollama_models(base_url).await;
            if !models.is_empty() {
                let n = models.len();
                let plural = if n != 1 { "s" } else { "" };
                println!("  {GREEN}+{RESET} Ollama conectado -- {n} modelo{plural}");
                return pick_model(&models);
            }
            // Fall through to no_models
        }

        if status == "no_models" || status == "ok" {
            println!("  {GREEN}+{RESET} Ollama conectado");
            println!("  {YELLOW}No hay modelos descargados{RESET}");

            let mut choices = suggested_models();
            choices.push(("Cambiar a Cloud".to_string(), "__cloud__".to_string()));
            choices.push(("Salir".to_string(), "__exit__".to_string()));

            let chosen = select_option("Descargar un modelo", &choices)?;
            if chosen == "__cloud__" {
                return setup_cloud_provider();
            }
            if chosen == "__exit__" {
                return None;
            }
            if pull_ollama_model(&chosen) {
                println!("\n  {GREEN}+ {chosen} listo{RESET}");
            } else {
                println!("\n  {RED}x Error descargando {chosen}{RESET}");
            }
            continue; // Re-detect models
        }

        // --- Other error ---
        if status.starts_with("error:") {
            let detail = status.strip_prefix("error:").unwrap_or(&status);
            println!("\n  {RED}x {detail}{RESET}");
            let retry_choices = vec![
                ("Reintentar".to_string(), "retry".to_string()),
                ("Cambiar a Cloud".to_string(), "cloud".to_string()),
                ("Salir".to_string(), "exit".to_string()),
            ];
            match select_option("Siguiente paso", &retry_choices)?.as_str() {
                "retry" => continue,
                "cloud" => return setup_cloud_provider(),
                _ => return None,
            }
        }
    }
}

fn pick_model(models: &[ModelOption]) -> Option<ModelOption> {
    if models.len() == 1 {
        println!("  {DIM}Modelo: {}{RESET}", models[0].name);
        return Some(models[0].clone());
    }

    let choices: Vec<(String, String)> = models
        .iter()
        .map(|m| {
            let loc = if m.local { "local" } else { "cloud" };
            let label = format!(
                "{}  -- {} | {} | {} ctx",
                m.name,
                loc,
                m.modality_label(),
                m.ctx_label()
            );
            (label, m.id.clone())
        })
        .collect();

    let selected_id = select_option("Modelo", &choices)?;
    models.iter().find(|m| m.id == selected_id).cloned()
}

// ---------------------------------------------------------------------------
// Provider setup: Cloud
// ---------------------------------------------------------------------------

struct RemoteProvider {
    provider: &'static str,
    label: &'static str,
    default_model: &'static str,
    env_key: &'static str,
}

const REMOTE_PROVIDERS: &[RemoteProvider] = &[
    RemoteProvider {
        provider: "groq",
        label: "Groq (cloud, free tier)",
        default_model: "qwen-qwq-32b",
        env_key: "GROQ_API_KEY",
    },
    RemoteProvider {
        provider: "nvidia",
        label: "NVIDIA NIM (cloud, free tier)",
        default_model: "meta/llama-3.3-70b-instruct",
        env_key: "NVIDIA_API_KEY",
    },
    RemoteProvider {
        provider: "openai",
        label: "OpenAI (cloud, paid)",
        default_model: "gpt-4o",
        env_key: "OPENAI_API_KEY",
    },
    RemoteProvider {
        provider: "openrouter",
        label: "OpenRouter (cloud, multi-model)",
        default_model: "meta-llama/llama-3.3-70b-instruct",
        env_key: "OPENROUTER_API_KEY",
    },
];

fn detect_remote_providers() -> Vec<&'static RemoteProvider> {
    REMOTE_PROVIDERS
        .iter()
        .filter(|p| std::env::var(p.env_key).is_ok())
        .collect()
}

fn setup_cloud_provider() -> Option<ModelOption> {
    loop {
        println!("\n  {DIM}Detectando API keys...{RESET}");
        let providers = detect_remote_providers();

        if providers.is_empty() {
            println!("\n  {RED}x No hay API keys configuradas{RESET}");
            println!("  {DIM}Configura al menos una:{RESET}");
            println!("    {DIM}export GROQ_API_KEY=gsk_...{RESET}          {GREEN}(gratis){RESET}");
            println!("    {DIM}export NVIDIA_API_KEY=nvapi-...{RESET}      {GREEN}(gratis){RESET}");
            println!("    {DIM}export OPENAI_API_KEY=sk-...{RESET}         {YELLOW}(pago){RESET}");
            println!("    {DIM}export OPENROUTER_API_KEY=sk-or-...{RESET}  {YELLOW}(multi-modelo){RESET}");

            let choices = vec![
                (
                    "Reintentar (despues de exportar la key)".to_string(),
                    "retry".to_string(),
                ),
                ("Cambiar a Local (Ollama)".to_string(), "local".to_string()),
                ("Salir".to_string(), "exit".to_string()),
            ];
            match select_option("Siguiente paso", &choices)?.as_str() {
                "retry" => continue,
                "local" => {
                    // Cannot call async from sync context, return a marker
                    return Some(ModelOption {
                        id: "__switch_local__".to_string(),
                        name: String::new(),
                        provider: "ollama".to_string(),
                        context_window: None,
                        supports_tools: false,
                        supports_vision: false,
                        local: true,
                    });
                }
                _ => return None,
            }
        }

        let chosen = if providers.len() == 1 {
            let p = providers[0];
            println!("  {GREEN}+{RESET} {}", p.label);
            p
        } else {
            println!(
                "  {GREEN}+{RESET} {} proveedores disponibles",
                providers.len()
            );
            let choices: Vec<(String, String)> = providers
                .iter()
                .map(|p| (p.label.to_string(), p.provider.to_string()))
                .collect();
            let key = select_option("Proveedor", &choices)?;
            providers.iter().find(|p| p.provider == key).copied()?
        };

        return Some(ModelOption {
            id: chosen.default_model.to_string(),
            name: chosen.label.to_string(),
            provider: chosen.provider.to_string(),
            context_window: Some(128_000),
            supports_tools: true,
            supports_vision: is_vision_model(chosen.default_model),
            local: false,
        });
    }
}

// ---------------------------------------------------------------------------
// Context window choices
// ---------------------------------------------------------------------------

fn context_window_choices(default_ctx: u32) -> Vec<(String, String)> {
    let label = |n: u32| -> String {
        if n >= 1_000_000 {
            format!("{}M", n / 1_000_000)
        } else {
            format!("{}K", n / 1000)
        }
    };

    let mut choices = vec![
        (
            format!("Default ({} -- capacidad del modelo)", label(default_ctx)),
            default_ctx.to_string(),
        ),
        (
            "Small (4K -- ahorra memoria)".to_string(),
            "4096".to_string(),
        ),
        ("Medium (8K)".to_string(), "8192".to_string()),
        ("Standard (16K)".to_string(), "16384".to_string()),
        ("Large (32K)".to_string(), "32768".to_string()),
        ("Extra Large (64K)".to_string(), "65536".to_string()),
    ];

    // Deduplicate by value
    let mut seen = std::collections::HashSet::new();
    choices.retain(|(_, v)| seen.insert(v.clone()));
    choices
}

// ---------------------------------------------------------------------------
// The wizard
// ---------------------------------------------------------------------------

/// Run the interactive setup wizard. Returns a `SessionConfig`.
///
/// If `skip` is true or all params are provided, skips interactive prompts.
pub async fn run_setup_wizard(
    skip: bool,
    provider: &str,
    model: &str,
    autonomy: &str,
) -> Option<SessionConfig> {
    // Quick path
    if skip || (!provider.is_empty() && !model.is_empty() && !autonomy.is_empty()) {
        let level = get_autonomy(if autonomy.is_empty() {
            "copilot"
        } else {
            autonomy
        });
        return Some(SessionConfig {
            provider: if provider.is_empty() {
                "ollama"
            } else {
                provider
            }
            .to_string(),
            model: if model.is_empty() { "qwen3:8b" } else { model }.to_string(),
            autonomy_level: level.key.to_string(),
            max_tool_rounds: level.max_tool_rounds,
            confirm_writes: level.confirm_writes,
            context_window: None,
            supports_vision: false,
            temperature: 0.3,
            max_tokens: 4096,
        });
    }

    println!("\n  {BOLD}{MAGENTA}OpenMirai -- Setup{RESET}\n");

    // --- Step 1: Local or Cloud? ---
    let mode_choices = vec![
        (
            "Local (Ollama -- corre en tu maquina)".to_string(),
            "local".to_string(),
        ),
        ("Cloud (necesita API key)".to_string(), "cloud".to_string()),
    ];
    let mode = select_option("Donde correra el modelo?", &mode_choices)?;

    // --- Step 2: Provider + Model ---
    let mut selected = if mode == "local" {
        setup_local_provider().await?
    } else {
        setup_cloud_provider()?
    };

    // Handle the "switch to local" marker from cloud setup
    if selected.id == "__switch_local__" {
        selected = setup_local_provider().await?;
    }

    let final_provider = if provider.is_empty() {
        selected.provider.clone()
    } else {
        provider.to_string()
    };
    let final_model = if model.is_empty() {
        selected.id.clone()
    } else {
        model.to_string()
    };

    // --- Step 3: Context window ---
    let default_ctx = selected.context_window.unwrap_or(4096);
    let ctx_choices = context_window_choices(default_ctx);
    let selected_ctx_str = select_option("Ventana de contexto", &ctx_choices)?;
    let selected_ctx: u32 = selected_ctx_str.parse().unwrap_or(default_ctx);

    // --- Step 4: Autonomy level ---
    let level_choices: Vec<(String, String)> = AUTONOMY_LEVELS
        .iter()
        .map(|l| {
            (
                format!("{}  -- {}", l.name, l.description),
                l.key.to_string(),
            )
        })
        .collect();
    let selected_level_key = select_option("Autonomia", &level_choices)?;
    let level = get_autonomy(&selected_level_key);

    Some(SessionConfig {
        provider: final_provider,
        model: final_model,
        autonomy_level: level.key.to_string(),
        max_tool_rounds: level.max_tool_rounds,
        confirm_writes: level.confirm_writes,
        context_window: Some(selected_ctx),
        supports_vision: selected.supports_vision,
        temperature: 0.3,
        max_tokens: 4096,
    })
}
