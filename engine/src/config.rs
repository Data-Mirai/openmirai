//! User configuration persisted at `~/.openmirai/config.toml` (PRD-014).
//!
//! Lets a developer pick their default provider + model once; the CLI and the
//! editor both read it. Precedence at run time: CLI flag > env > this config >
//! built-in fallback. API keys are NEVER stored here (env/flag only).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_ollama_host")]
    pub ollama_host: String,
}

fn default_provider() -> String {
    "ollama".to_string()
}
fn default_model() -> String {
    "qwen3:8b".to_string()
}
fn default_ollama_host() -> String {
    "http://localhost:11434".to_string()
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
            default_model: default_model(),
            ollama_host: default_ollama_host(),
        }
    }
}

impl UserConfig {
    /// `~/.openmirai/config.toml` (None if no home dir).
    pub fn config_path() -> Option<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        Some(PathBuf::from(home).join(".openmirai").join("config.toml"))
    }

    /// Load from disk, falling back to defaults if absent or unparseable.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Whether a config file exists on disk.
    pub fn exists() -> bool {
        Self::config_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Persist to `~/.openmirai/config.toml` (creates the dir).
    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = Self::config_path().ok_or_else(|| std::io::Error::other("no home directory"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_ollama_qwen() {
        let c = UserConfig::default();
        assert_eq!(c.default_provider, "ollama");
        assert_eq!(c.default_model, "qwen3:8b");
        assert_eq!(c.ollama_host, "http://localhost:11434");
    }

    #[test]
    fn toml_roundtrip() {
        let c = UserConfig {
            default_provider: "claude".into(),
            default_model: "claude-3-5-sonnet".into(),
            ollama_host: "http://localhost:11434".into(),
        };
        let s = toml::to_string_pretty(&c).unwrap();
        let back: UserConfig = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
        // keys must never carry secrets — sanity that the struct has no key field
        assert!(!s.to_lowercase().contains("api_key"));
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let back: UserConfig = toml::from_str("default_model = \"gemma3:1b\"").unwrap();
        assert_eq!(back.default_model, "gemma3:1b");
        assert_eq!(back.default_provider, "ollama"); // defaulted
    }
}
