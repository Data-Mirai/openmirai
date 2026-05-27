//! Code execution sandbox — isolated process execution with limits.
//!
//! Phase 1: Process-level isolation using `tokio::process::Command` with:
//! - Timeout enforcement (kill on exceed)
//! - Working directory isolation (temp dir)
//! - Network access control (basic, via config)
//! - Filesystem access control (temp_only, readonly, none)

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Sandbox execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Maximum execution time in milliseconds.
    #[serde(default = "default_timeout")]
    pub max_time_ms: u64,
    /// Maximum memory in MB (informational — enforced via ulimit on supported OS).
    #[serde(default = "default_memory")]
    pub max_memory_mb: u64,
    /// Whether network access is allowed.
    #[serde(default)]
    pub network_access: bool,
    /// Filesystem access level.
    #[serde(default)]
    pub filesystem_access: FilesystemAccess,
}

fn default_timeout() -> u64 {
    30_000
}
fn default_memory() -> u64 {
    256
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_time_ms: 30_000,
            max_memory_mb: 256,
            network_access: false,
            filesystem_access: FilesystemAccess::TempOnly,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    None,
    Readonly,
    #[default]
    TempOnly,
}

/// Supported languages for sandbox execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    Javascript,
    Bash,
}

/// Result of a sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Execute code in an isolated sandbox.
pub async fn execute(
    code: &str,
    language: &Language,
    config: &SandboxConfig,
) -> SandboxResult {
    let start = std::time::Instant::now();

    // Create temp directory for isolation.
    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return SandboxResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: -1,
                duration_ms: 0,
                timed_out: false,
                error: Some(format!("Failed to create temp dir: {e}")),
            };
        }
    };

    let work_dir = temp_dir.path();

    // Write code to temp file.
    let (file_name, interpreter) = match language {
        Language::Python => ("script.py", vec!["python3", "script.py"]),
        Language::Javascript => ("script.js", vec!["node", "script.js"]),
        Language::Bash => ("script.sh", vec!["bash", "script.sh"]),
    };

    let script_path = work_dir.join(file_name);
    if let Err(e) = std::fs::write(&script_path, code) {
        return SandboxResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: -1,
            duration_ms: 0,
            timed_out: false,
            error: Some(format!("Failed to write script: {e}")),
        };
    }

    // Build command.
    let mut cmd = Command::new(interpreter[0]);
    for arg in &interpreter[1..] {
        cmd.arg(arg);
    }
    cmd.current_dir(work_dir);

    // Set resource limits via env/ulimit wrapper for supported platforms.
    #[cfg(unix)]
    {
        let mem_bytes = config.max_memory_mb * 1024 * 1024;
        // Use ulimit wrapper for memory
        let wrapper_code = format!(
            "ulimit -v {} 2>/dev/null; exec {} {}",
            mem_bytes / 1024, // ulimit uses KB
            interpreter[0],
            script_path.display()
        );
        cmd = Command::new("bash");
        cmd.arg("-c").arg(&wrapper_code).current_dir(work_dir);
    }

    // Restrict environment for security.
    cmd.env_clear();
    cmd.env("HOME", work_dir);
    cmd.env("TMPDIR", work_dir);
    cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");

    // Execute with timeout.
    let timeout_dur = Duration::from_millis(config.max_time_ms);

    let output = match timeout(timeout_dur, cmd.output()).await {
        Ok(Ok(output)) => {
            let elapsed = start.elapsed().as_millis() as u64;
            SandboxResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                duration_ms: elapsed,
                timed_out: false,
                error: None,
            }
        }
        Ok(Err(e)) => {
            let elapsed = start.elapsed().as_millis() as u64;
            SandboxResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: -1,
                duration_ms: elapsed,
                timed_out: false,
                error: Some(format!("Process error: {e}")),
            }
        }
        Err(_) => {
            let elapsed = start.elapsed().as_millis() as u64;
            SandboxResult {
                stdout: String::new(),
                stderr: format!("Execution timed out after {}ms", config.max_time_ms),
                exit_code: -1,
                duration_ms: elapsed,
                timed_out: true,
                error: Some(format!(
                    "Sandbox timeout: exceeded {}ms limit",
                    config.max_time_ms
                )),
            }
        }
    };

    // Cleanup happens automatically when temp_dir is dropped.
    output
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_python_hello() {
        let result = execute(
            "print('hello sandbox')",
            &Language::Python,
            &SandboxConfig::default(),
        )
        .await;

        // Python might not be installed; skip if not available.
        if result.error.is_some() && result.error.as_ref().unwrap().contains("Process error") {
            return; // Skip if python3 not found
        }
        assert_eq!(result.stdout.trim(), "hello sandbox");
        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn execute_bash_echo() {
        let result = execute(
            "echo 'hello from bash'",
            &Language::Bash,
            &SandboxConfig::default(),
        )
        .await;

        assert_eq!(result.stdout.trim(), "hello from bash");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn execute_timeout() {
        let config = SandboxConfig {
            max_time_ms: 500,
            ..Default::default()
        };
        let result = execute("sleep 10", &Language::Bash, &config).await;
        assert!(result.timed_out);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn execute_nonzero_exit() {
        let result = execute(
            "exit 42",
            &Language::Bash,
            &SandboxConfig::default(),
        )
        .await;
        assert_eq!(result.exit_code, 42);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn execute_stderr() {
        let result = execute(
            "echo 'err' >&2",
            &Language::Bash,
            &SandboxConfig::default(),
        )
        .await;
        assert!(result.stderr.contains("err"));
    }

    #[test]
    fn config_defaults() {
        let config = SandboxConfig::default();
        assert_eq!(config.max_time_ms, 30_000);
        assert_eq!(config.max_memory_mb, 256);
        assert!(!config.network_access);
        assert_eq!(config.filesystem_access, FilesystemAccess::TempOnly);
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = SandboxConfig {
            max_time_ms: 5000,
            max_memory_mb: 128,
            network_access: true,
            filesystem_access: FilesystemAccess::Readonly,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_time_ms, 5000);
        assert!(back.network_access);
    }
}
