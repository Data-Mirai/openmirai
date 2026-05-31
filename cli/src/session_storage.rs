//! Session persistence — stores conversations as manifest + JSONL transcript.
//!
//! Each session lives in `~/.datamirai/sessions/<session_id>/` with:
//! - `manifest.json`    metadata (id, provider, model, cwd, timestamps)
//! - `transcript.jsonl`  append-only log of every event

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, Write as IoWrite};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub message_count: u32,
    pub checkpoint_count: u32,
    pub status: String, // "active" | "closed"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub ts: f64,
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub message_index: usize,
    pub ts: f64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn short_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rand: u32 = rand_u32();
    format!("ses_{ts}_{rand:08x}")
}

/// Cheap pseudo-random u32 using time nanos (no extra deps).
fn rand_u32() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // xorshift32 seeded from nanos
    let mut x = nanos.wrapping_add(0x9E37_79B9);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

// ---------------------------------------------------------------------------
// SessionStorage
// ---------------------------------------------------------------------------

pub struct SessionStorage {
    base_dir: PathBuf,
}

impl SessionStorage {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self {
            base_dir: PathBuf::from(home).join(".datamirai").join("sessions"),
        }
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(session_id)
    }

    fn manifest_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("manifest.json")
    }

    fn transcript_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("transcript.jsonl")
    }

    // --- Create ---

    pub fn create_session(&self, provider: &str, model: &str, cwd: &str) -> SessionManifest {
        let id = short_id();
        let dir = self.session_dir(&id);
        fs::create_dir_all(&dir).ok();

        let now = now_secs();
        let manifest = SessionManifest {
            id: id.clone(),
            provider: provider.to_string(),
            model: model.to_string(),
            cwd: cwd.to_string(),
            created_at: now,
            updated_at: now,
            message_count: 0,
            checkpoint_count: 0,
            status: "active".to_string(),
        };

        if let Ok(data) = serde_json::to_string_pretty(&manifest) {
            fs::write(self.manifest_path(&id), data).ok();
        }
        // Create empty transcript.
        fs::write(self.transcript_path(&id), "").ok();

        manifest
    }

    // --- Append ---

    pub fn append_entry(&self, session_id: &str, entry: &TranscriptEntry) {
        if let Ok(line) = serde_json::to_string(entry) {
            let path = self.transcript_path(session_id);
            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{line}");
            }
        }
        self.bump_manifest_ts(session_id);
    }

    pub fn append_user_message(&self, session_id: &str, content: &str) {
        self.append_entry(
            session_id,
            &TranscriptEntry {
                ts: now_secs(),
                role: "user".to_string(),
                content: content.to_string(),
                metadata: serde_json::Map::new(),
            },
        );
    }

    pub fn append_assistant_message(&self, session_id: &str, content: &str) {
        self.append_entry(
            session_id,
            &TranscriptEntry {
                ts: now_secs(),
                role: "assistant".to_string(),
                content: content.to_string(),
                metadata: serde_json::Map::new(),
            },
        );
    }

    pub fn append_tool_call(&self, session_id: &str, tool: &str, args: &Value, round_num: u32) {
        let mut meta = serde_json::Map::new();
        meta.insert("tool".to_string(), Value::String(tool.to_string()));
        meta.insert("args".to_string(), args.clone());
        meta.insert("round".to_string(), Value::Number(round_num.into()));
        self.append_entry(
            session_id,
            &TranscriptEntry {
                ts: now_secs(),
                role: "tool_call".to_string(),
                content: String::new(),
                metadata: meta,
            },
        );
    }

    pub fn append_tool_result(&self, session_id: &str, tool: &str, result: &Value, round_num: u32) {
        let result_str = serde_json::to_string(result).unwrap_or_default();
        let truncated = if result_str.len() > 5000 {
            format!("{}...(truncated)", &result_str[..5000])
        } else {
            result_str
        };
        let mut meta = serde_json::Map::new();
        meta.insert("tool".to_string(), Value::String(tool.to_string()));
        meta.insert("round".to_string(), Value::Number(round_num.into()));
        self.append_entry(
            session_id,
            &TranscriptEntry {
                ts: now_secs(),
                role: "tool_result".to_string(),
                content: truncated,
                metadata: meta,
            },
        );
    }

    // --- Checkpoints ---

    pub fn create_checkpoint(
        &self,
        session_id: &str,
        message_index: usize,
        label: &str,
    ) -> Checkpoint {
        let id = format!("chk_{:06x}", rand_u32() & 0xFFFFFF);
        let ts = now_secs();
        let mut meta = serde_json::Map::new();
        meta.insert("checkpoint_id".to_string(), Value::String(id.clone()));
        meta.insert("label".to_string(), Value::String(label.to_string()));
        meta.insert(
            "message_index".to_string(),
            Value::Number(message_index.into()),
        );
        self.append_entry(
            session_id,
            &TranscriptEntry {
                ts,
                role: "checkpoint".to_string(),
                content: String::new(),
                metadata: meta,
            },
        );
        self.bump_checkpoint_count(session_id);
        Checkpoint {
            id,
            label: label.to_string(),
            message_index,
            ts,
        }
    }

    pub fn list_checkpoints(&self, session_id: &str) -> Vec<Checkpoint> {
        let entries = self.read_transcript(session_id);
        entries
            .iter()
            .filter(|e| e.role == "checkpoint")
            .filter_map(|e| {
                Some(Checkpoint {
                    id: e.metadata.get("checkpoint_id")?.as_str()?.to_string(),
                    label: e
                        .metadata
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    message_index: e
                        .metadata
                        .get("message_index")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    ts: e.ts,
                })
            })
            .collect()
    }

    // --- Read ---

    pub fn read_manifest(&self, session_id: &str) -> Option<SessionManifest> {
        let path = self.manifest_path(session_id);
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn read_transcript(&self, session_id: &str) -> Vec<TranscriptEntry> {
        let path = self.transcript_path(session_id);
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = std::io::BufReader::new(file);
        reader
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                serde_json::from_str(trimmed).ok()
            })
            .collect()
    }

    pub fn rebuild_messages(&self, session_id: &str) -> Vec<serde_json::Map<String, Value>> {
        let entries = self.read_transcript(session_id);
        entries
            .iter()
            .filter(|e| matches!(e.role.as_str(), "user" | "assistant" | "system"))
            .map(|e| {
                let mut m = serde_json::Map::new();
                m.insert("role".to_string(), Value::String(e.role.clone()));
                m.insert("content".to_string(), Value::String(e.content.clone()));
                m
            })
            .collect()
    }

    // --- List ---

    pub fn list_sessions(&self, limit: usize) -> Vec<SessionManifest> {
        let dir = match fs::read_dir(&self.base_dir) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let mut sessions: Vec<SessionManifest> = dir
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().to_string_lossy().to_string();
                self.read_manifest(&name)
            })
            .collect();
        sessions.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sessions.truncate(limit);
        sessions
    }

    // --- Close ---

    pub fn close_session(&self, session_id: &str) {
        let path = self.manifest_path(session_id);
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(mut manifest) = serde_json::from_str::<SessionManifest>(&data) {
                manifest.status = "closed".to_string();
                manifest.updated_at = now_secs();
                if let Ok(json) = serde_json::to_string_pretty(&manifest) {
                    fs::write(path, json).ok();
                }
            }
        }
    }

    // --- Internal ---

    fn bump_manifest_ts(&self, session_id: &str) {
        let path = self.manifest_path(session_id);
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(mut manifest) = serde_json::from_str::<SessionManifest>(&data) {
                manifest.updated_at = now_secs();
                manifest.message_count += 1;
                if let Ok(json) = serde_json::to_string_pretty(&manifest) {
                    fs::write(path, json).ok();
                }
            }
        }
    }

    fn bump_checkpoint_count(&self, session_id: &str) {
        let path = self.manifest_path(session_id);
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(mut manifest) = serde_json::from_str::<SessionManifest>(&data) {
                manifest.checkpoint_count += 1;
                if let Ok(json) = serde_json::to_string_pretty(&manifest) {
                    fs::write(path, json).ok();
                }
            }
        }
    }
}

impl Default for SessionStorage {
    fn default() -> Self {
        Self::new()
    }
}
