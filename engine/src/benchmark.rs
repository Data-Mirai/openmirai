//! Benchmark logger — append-only JSONL file for performance metrics.
//!
//! Tracks cold_start, execution, llm_latency, tool_latency, and memory_usage.
//! Each entry is a single JSON line appended to the benchmark file.
//!
//! Enabled via `--benchmark` CLI flag or `MIRAI_BENCHMARK=1` env var.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static ENABLED: AtomicBool = AtomicBool::new(false);
static FILE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static COLD_START: Mutex<Option<Instant>> = Mutex::new(None);

/// Enable benchmark logging to the given file path.
pub fn enable(path: impl Into<PathBuf>) {
    let p = path.into();
    *FILE_PATH.lock().unwrap() = Some(p);
    ENABLED.store(true, Ordering::SeqCst);
}

/// Check if benchmarks are enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Mark the start of the process (for cold_start measurement).
pub fn mark_process_start() {
    *COLD_START.lock().unwrap() = Some(Instant::now());
}

/// Record cold_start metric (time from process start to first ready).
pub fn record_cold_start() {
    if !is_enabled() {
        return;
    }
    if let Some(start) = COLD_START.lock().unwrap().take() {
        let ms = start.elapsed().as_millis() as u64;
        log(BenchmarkEntry {
            metric_type: MetricType::ColdStart,
            value_ms: Some(ms),
            value_bytes: None,
            context: serde_json::json!({"binary": "mirai"}),
        });
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    ColdStart,
    Execution,
    LlmLatency,
    ToolLatency,
    MemoryUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkEntry {
    pub metric_type: MetricType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_bytes: Option<u64>,
    #[serde(default)]
    pub context: Value,
}

#[derive(Debug, Serialize)]
struct BenchmarkLine {
    ts: String,
    #[serde(rename = "type")]
    metric_type: MetricType,
    #[serde(skip_serializing_if = "Option::is_none")]
    ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    ctx: Value,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Log a benchmark entry. No-op if benchmarking is not enabled.
pub fn log(entry: BenchmarkEntry) {
    if !is_enabled() {
        return;
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = ts.as_secs();
    // Simple ISO-like timestamp
    let ts_str = format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + secs / 31_536_000,
        (secs % 31_536_000) / 2_592_000 + 1,
        (secs % 2_592_000) / 86400 + 1,
        (secs % 86400) / 3600,
        (secs % 3600) / 60,
        secs % 60,
    );

    // Use chrono if available for proper formatting, fallback to epoch
    let line = BenchmarkLine {
        ts: ts_str,
        metric_type: entry.metric_type,
        ms: entry.value_ms,
        bytes: entry.value_bytes,
        ctx: entry.context,
    };

    if let Ok(json) = serde_json::to_string(&line) {
        append_line(&json);
    }
}

/// Log execution duration.
pub fn log_execution(duration_ms: u64, agent_name: &str, node_count: usize, provider: &str) {
    log(BenchmarkEntry {
        metric_type: MetricType::Execution,
        value_ms: Some(duration_ms),
        value_bytes: None,
        context: serde_json::json!({
            "agent": agent_name,
            "nodes": node_count,
            "provider": provider,
        }),
    });
}

/// Log LLM call latency.
pub fn log_llm_latency(duration_ms: u64, model: &str, provider: &str, tokens_in: u32, tokens_out: u32) {
    log(BenchmarkEntry {
        metric_type: MetricType::LlmLatency,
        value_ms: Some(duration_ms),
        value_bytes: None,
        context: serde_json::json!({
            "model": model,
            "provider": provider,
            "tokens_in": tokens_in,
            "tokens_out": tokens_out,
        }),
    });
}

/// Log tool execution latency.
pub fn log_tool_latency(duration_ms: u64, tool_type: &str, node_id: &str) {
    log(BenchmarkEntry {
        metric_type: MetricType::ToolLatency,
        value_ms: Some(duration_ms),
        value_bytes: None,
        context: serde_json::json!({
            "tool_type": tool_type,
            "node_id": node_id,
        }),
    });
}

/// Log current memory usage (RSS).
pub fn log_memory_usage() {
    // On macOS/Linux, read RSS from /proc or use sysinfo.
    // For portability, we use a simple approach.
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            log(BenchmarkEntry {
                                metric_type: MetricType::MemoryUsage,
                                value_ms: None,
                                value_bytes: Some(kb * 1024),
                                context: serde_json::json!({"source": "proc"}),
                            });
                            return;
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
        {
            if let Ok(rss_str) = String::from_utf8(output.stdout) {
                if let Ok(kb) = rss_str.trim().parse::<u64>() {
                    log(BenchmarkEntry {
                        metric_type: MetricType::MemoryUsage,
                        value_ms: None,
                        value_bytes: Some(kb * 1024),
                        context: serde_json::json!({"source": "ps"}),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn append_line(line: &str) {
    let path = match FILE_PATH.lock().unwrap().clone() {
        Some(p) => p,
        None => return,
    };

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{}", line);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Tests use `log_to_file` directly to avoid global state conflicts
    // between parallel tests.

    fn log_to_file(path: &std::path::Path, entry: BenchmarkEntry) {
        let ts = "2026-05-26T00:00:00Z";
        let line = BenchmarkLine {
            ts: ts.to_string(),
            metric_type: entry.metric_type,
            ms: entry.value_ms,
            bytes: entry.value_bytes,
            ctx: entry.context,
        };
        if let Ok(json) = serde_json::to_string(&line) {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{}", json);
            }
        }
    }

    #[test]
    fn benchmark_entry_serializes() {
        let entry = BenchmarkEntry {
            metric_type: MetricType::Execution,
            value_ms: Some(1234),
            value_bytes: None,
            context: serde_json::json!({"agent": "test"}),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("execution"));
        assert!(json.contains("1234"));
    }

    #[test]
    fn log_writes_jsonl_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bench_test.jsonl");

        log_to_file(&path, BenchmarkEntry {
            metric_type: MetricType::Execution,
            value_ms: Some(1234),
            value_bytes: None,
            context: serde_json::json!({"agent": "test"}),
        });
        log_to_file(&path, BenchmarkEntry {
            metric_type: MetricType::MemoryUsage,
            value_ms: None,
            value_bytes: Some(8192),
            context: Value::Null,
        });

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["type"], "execution");
        assert_eq!(first["ms"], 1234);
        assert_eq!(first["ctx"]["agent"], "test");

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["type"], "memory_usage");
        assert_eq!(second["bytes"], 8192);
    }

    #[test]
    fn log_execution_convenience_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bench_exec.jsonl");

        // Simulate log_execution directly to file
        log_to_file(&path, BenchmarkEntry {
            metric_type: MetricType::Execution,
            value_ms: Some(500),
            value_bytes: None,
            context: serde_json::json!({
                "agent": "my-agent",
                "nodes": 3,
                "provider": "ollama",
            }),
        });

        let content = std::fs::read_to_string(&path).unwrap();
        let entry: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(entry["type"], "execution");
        assert_eq!(entry["ms"], 500);
        assert_eq!(entry["ctx"]["agent"], "my-agent");
        assert_eq!(entry["ctx"]["nodes"], 3);
    }

    #[test]
    fn metric_type_serde_roundtrip() {
        let types = vec![
            MetricType::ColdStart,
            MetricType::Execution,
            MetricType::LlmLatency,
            MetricType::ToolLatency,
            MetricType::MemoryUsage,
        ];
        for mt in types {
            let json = serde_json::to_string(&mt).unwrap();
            let back: MetricType = serde_json::from_str(&json).unwrap();
            assert_eq!(mt, back);
        }
    }
}
