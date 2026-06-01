//! Preflight checks before running an agent (PRD-014).
//!
//! Turns late, cryptic runtime failures ("HTTP 404: model not found") into
//! early, actionable guidance: is Ollama running? is the model pulled? is a
//! cloud API key present? Used by both `mirai run`/`mirai doctor` and the
//! editor's ▶ Run gate.

use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    /// "error" | "warn"
    pub level: String,
    /// Human-readable description of what's missing.
    pub message: String,
    /// The exact command / step that resolves it.
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preflight {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Check whether `provider`/`model` is ready to run.
/// - For `ollama`: the daemon must be reachable and the model installed.
/// - For cloud providers: an API key must be present (`has_key`).
pub async fn check(provider: &str, model: &str, ollama_host: &str, has_key: bool) -> Preflight {
    let mut diagnostics = Vec::new();

    if provider == "ollama" {
        match installed_models(ollama_host).await {
            Ok(models) => {
                if !model_present(&models, model) {
                    diagnostics.push(Diagnostic {
                        level: "error".to_string(),
                        message: format!("Ollama model '{model}' is not downloaded."),
                        action: format!("ollama pull {model}"),
                    });
                }
            }
            Err(_) => diagnostics.push(Diagnostic {
                level: "error".to_string(),
                message: format!("Ollama is not reachable at {ollama_host}."),
                action: "Start it with `ollama serve` (or install from https://ollama.com)"
                    .to_string(),
            }),
        }
    } else if !has_key {
        diagnostics.push(Diagnostic {
            level: "error".to_string(),
            message: format!("Provider '{provider}' requires an API key."),
            action: "Pass --api-key, or set the provider's env var (e.g. ANTHROPIC_API_KEY / OPENAI_API_KEY / MIRAI_API_KEY)"
                .to_string(),
        });
    }

    Preflight {
        ok: diagnostics.is_empty(),
        provider: provider.to_string(),
        model: model.to_string(),
        diagnostics,
    }
}

/// Build an Ollama API URL, tolerating a trailing slash on the host.
fn ollama_url(host: &str, path: &str) -> String {
    format!(
        "{}/{}",
        host.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// A reqwest client with the given timeout (build errors mapped to `String`).
fn ollama_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())
}

/// Map a non-2xx Ollama response to an error.
fn check_http_ok(resp: &reqwest::Response) -> Result<(), String> {
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// Models installed locally in Ollama via `GET /api/tags`.
pub async fn installed_models(ollama_host: &str) -> Result<Vec<String>, String> {
    let url = ollama_url(ollama_host, "api/tags");
    let client = ollama_client(5)?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    check_http_ok(&resp)?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let models = v
        .get("models")
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(models)
}

/// Pull an Ollama model, invoking `on_progress(status, completed, total)` for
/// each streamed update. Used by `mirai models pull` and the editor's download.
pub async fn pull_model<F>(ollama_host: &str, name: &str, mut on_progress: F) -> Result<(), String>
where
    F: FnMut(&str, Option<u64>, Option<u64>),
{
    use futures_util::StreamExt;

    let url = ollama_url(ollama_host, "api/pull");
    let client = ollama_client(3600)?;
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "name": name, "stream": true }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    check_http_ok(&resp)?;

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(nl) = buf.find('\n') {
            let line: String = buf.drain(..=nl).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let completed = v.get("completed").and_then(serde_json::Value::as_u64);
                let total = v.get("total").and_then(serde_json::Value::as_u64);
                on_progress(status, completed, total);
            }
        }
    }
    Ok(())
}

/// Whether `want` is among `installed`, tolerating the implicit `:latest` tag.
fn model_present(installed: &[String], want: &str) -> bool {
    let norm = |m: &str| -> String {
        if m.contains(':') {
            m.to_string()
        } else {
            format!("{m}:latest")
        }
    };
    let target = norm(want);
    installed.iter().any(|m| norm(m) == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_match_tolerates_latest() {
        let installed = vec!["qwen3:8b".to_string(), "llama3.2:latest".to_string()];
        assert!(model_present(&installed, "qwen3:8b"));
        assert!(model_present(&installed, "llama3.2")); // -> llama3.2:latest
        assert!(!model_present(&installed, "gemma3:1b"));
    }

    #[tokio::test]
    async fn cloud_provider_without_key_is_blocked() {
        let pf = check(
            "claude",
            "claude-3-5-sonnet",
            "http://localhost:11434",
            false,
        )
        .await;
        assert!(!pf.ok);
        assert_eq!(pf.diagnostics.len(), 1);
        assert!(pf.diagnostics[0].message.contains("API key"));
    }

    #[tokio::test]
    async fn cloud_provider_with_key_is_ok() {
        let pf = check("openai", "gpt-4", "http://localhost:11434", true).await;
        assert!(pf.ok);
    }
}
