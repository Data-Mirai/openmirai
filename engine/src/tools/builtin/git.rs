use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Macro
// ---------------------------------------------------------------------------

macro_rules! git_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: "git".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl Default for $factory {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Helpers: run git commands
// ---------------------------------------------------------------------------

/// Run a git command and return (stdout, stderr, exit_code).
async fn run_git(args: &[&str], cwd: &str) -> Result<(String, String, i32), ToolError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool_type: "git".into(),
            message: format!("Failed to run git: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);
    Ok((stdout, stderr, code))
}

/// Run a git command and return stdout, or error if non-zero exit.
async fn run_git_ok(args: &[&str], cwd: &str, tool_type: &str) -> Result<String, ToolError> {
    let (stdout, stderr, code) = run_git(args, cwd).await?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed {
            tool_type: tool_type.into(),
            message: format!("git {} failed: {}", args[0], stderr.trim()),
        });
    }
    Ok(stdout)
}

/// Resolve the working directory from inputs, defaulting to ".".
fn resolve_cwd(inputs: &HashMap<String, Value>, config: &HashMap<String, Value>) -> String {
    // Check inputs first (git/status, git/diff, git/log use "path" in inputs)
    inputs
        .get("path")
        .and_then(|v| v.as_str())
        .or_else(|| config.get("path").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .unwrap_or(".")
        .to_string()
}

// ===========================================================================
// GitStatusTool
// ===========================================================================

git_tool! {
    struct GitStatusTool, factory GitStatusFactory;
    tool_type = "git/status",
    name = "Git Status",
    description = "Shows the working tree status: staged, modified, and untracked files.",
    inputs = [
        field("path", FieldType::String, false, "Repository path (defaults to cwd)"),
    ],
    outputs = [
        field("branch", FieldType::String, true, "Current branch name"),
        field("staged", FieldType::Array, true, "Staged files"),
        field("modified", FieldType::Array, true, "Modified but unstaged files"),
        field("untracked", FieldType::Array, true, "Untracked files"),
        field("clean", FieldType::Boolean, true, "True if working tree is clean"),
        field("raw", FieldType::String, true, "Raw git status output"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for GitStatusTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let cwd = resolve_cwd(&inputs, config);

        // Get branch
        let branch_out = run_git_ok(&["branch", "--show-current"], &cwd, "git/status").await?;
        let branch = branch_out.trim();
        let branch = if branch.is_empty() {
            "HEAD (detached)"
        } else {
            branch
        };

        // Get porcelain status
        let raw = run_git_ok(&["status", "--porcelain=v1"], &cwd, "git/status").await?;

        let mut staged: Vec<String> = Vec::new();
        let mut modified: Vec<String> = Vec::new();
        let mut untracked: Vec<String> = Vec::new();

        for line in raw.lines() {
            if line.len() < 4 {
                continue;
            }
            let bytes = line.as_bytes();
            let index_status = bytes[0] as char;
            let worktree_status = bytes[1] as char;
            let filepath = &line[3..];

            if "MADRC".contains(index_status) {
                staged.push(filepath.to_string());
            }
            if "MD".contains(worktree_status) {
                modified.push(filepath.to_string());
            }
            if index_status == '?' && worktree_status == '?' {
                untracked.push(filepath.to_string());
            }
        }

        let clean = staged.is_empty() && modified.is_empty() && untracked.is_empty();

        let mut out = HashMap::new();
        out.insert("branch".to_string(), json!(branch));
        out.insert("staged".to_string(), json!(staged));
        out.insert("modified".to_string(), json!(modified));
        out.insert("untracked".to_string(), json!(untracked));
        out.insert("clean".to_string(), json!(clean));
        out.insert("raw".to_string(), json!(raw));
        Ok(out)
    }
}

// ===========================================================================
// GitDiffTool
// ===========================================================================

git_tool! {
    struct GitDiffTool, factory GitDiffFactory;
    tool_type = "git/diff",
    name = "Git Diff",
    description = "Shows differences in the working tree, staged changes, or between commits.",
    inputs = [
        field("path", FieldType::String, false, "Repository path (defaults to cwd)"),
    ],
    outputs = [
        field("diff", FieldType::String, true, "Diff output"),
        field("files_changed", FieldType::Number, true, "Number of files changed"),
        field("insertions", FieldType::Number, true, "Lines added"),
        field("deletions", FieldType::Number, true, "Lines removed"),
    ],
    config_fields = [
        field("staged", FieldType::Boolean, false, "Show staged changes (--cached)"),
        field("ref", FieldType::String, false, "Compare against ref (e.g. 'HEAD~3', 'main')"),
        field("file", FieldType::String, false, "Limit diff to specific file"),
    ]
}

#[async_trait]
impl Tool for GitDiffTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let cwd = resolve_cwd(&inputs, config);
        let staged = config
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let git_ref = config.get("ref").and_then(|v| v.as_str()).unwrap_or("");
        let file_filter = config.get("file").and_then(|v| v.as_str()).unwrap_or("");

        // Build diff command args
        let mut args: Vec<&str> = vec!["diff"];
        if staged {
            args.push("--cached");
        }
        if !git_ref.is_empty() {
            args.push(git_ref);
        }
        if !file_filter.is_empty() {
            args.push("--");
            args.push(file_filter);
        }

        let diff_text = run_git_ok(&args, &cwd, "git/diff").await?;

        // Get stat summary
        let mut stat_args = args.clone();
        stat_args.push("--stat");
        let stat_text = run_git_ok(&stat_args, &cwd, "git/diff").await?;

        let mut files_changed: i64 = 0;
        let mut insertions: i64 = 0;
        let mut deletions: i64 = 0;

        for line in stat_text.lines() {
            if line.contains("file") && line.contains("changed") {
                for part in line.split(',') {
                    let part = part.trim();
                    if part.contains("file") {
                        if let Some(num) = part.split_whitespace().next() {
                            files_changed = num.parse().unwrap_or(0);
                        }
                    } else if part.contains("insertion") {
                        if let Some(num) = part.split_whitespace().next() {
                            insertions = num.parse().unwrap_or(0);
                        }
                    } else if part.contains("deletion") {
                        if let Some(num) = part.split_whitespace().next() {
                            deletions = num.parse().unwrap_or(0);
                        }
                    }
                }
            }
        }

        // Truncate large diffs
        let max_len = 50_000;
        let diff_output = if diff_text.len() > max_len {
            let total = diff_text.len();
            format!(
                "{}\n... (truncated, {} total chars)",
                &diff_text[..max_len],
                total
            )
        } else {
            diff_text
        };

        let mut out = HashMap::new();
        out.insert("diff".to_string(), json!(diff_output));
        out.insert("files_changed".to_string(), json!(files_changed));
        out.insert("insertions".to_string(), json!(insertions));
        out.insert("deletions".to_string(), json!(deletions));
        Ok(out)
    }
}

// ===========================================================================
// GitLogTool
// ===========================================================================

git_tool! {
    struct GitLogTool, factory GitLogFactory;
    tool_type = "git/log",
    name = "Git Log",
    description = "Shows the commit history with hash, author, date, and message.",
    inputs = [
        field("path", FieldType::String, false, "Repository path (defaults to cwd)"),
    ],
    outputs = [
        field("commits", FieldType::Array, true, "List of {hash, author, date, message} entries"),
        field("count", FieldType::Number, true, "Number of commits returned"),
        field("raw", FieldType::String, true, "Raw log output"),
    ],
    config_fields = [
        field("limit", FieldType::Number, false, "Maximum commits to return"),
        field("author", FieldType::String, false, "Filter by author name"),
        field("file", FieldType::String, false, "Show history for specific file"),
    ]
}

const COMMIT_SEP: &str = "---COMMIT_SEP---";

#[async_trait]
impl Tool for GitLogTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let cwd = resolve_cwd(&inputs, config);
        let limit = config.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);
        let author = config.get("author").and_then(|v| v.as_str()).unwrap_or("");
        let file_filter = config.get("file").and_then(|v| v.as_str()).unwrap_or("");

        let fmt = format!("%H{COMMIT_SEP}%an{COMMIT_SEP}%ai{COMMIT_SEP}%s");
        let limit_arg = format!("-{}", limit);

        let mut args: Vec<String> = vec!["log".into(), limit_arg, format!("--format={}", fmt)];
        if !author.is_empty() {
            args.push(format!("--author={}", author));
        }
        if !file_filter.is_empty() {
            args.push("--".into());
            args.push(file_filter.into());
        }

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let raw = run_git_ok(&args_refs, &cwd, "git/log").await?;

        let mut commits: Vec<Value> = Vec::new();
        for line in raw.trim().lines() {
            let parts: Vec<&str> = line.splitn(4, COMMIT_SEP).collect();
            if parts.len() == 4 {
                commits.push(json!({
                    "hash": parts[0],
                    "author": parts[1],
                    "date": parts[2],
                    "message": parts[3],
                }));
            }
        }

        let count = commits.len();

        let mut out = HashMap::new();
        out.insert("commits".to_string(), json!(commits));
        out.insert("count".to_string(), json!(count));
        out.insert("raw".to_string(), json!(raw));
        Ok(out)
    }
}

// ===========================================================================
// GitCommitTool
// ===========================================================================

git_tool! {
    struct GitCommitTool, factory GitCommitFactory;
    tool_type = "git/commit",
    name = "Git Commit",
    description = "Stages specified files and creates a commit. Never amends or force-pushes.",
    inputs = [
        field("message", FieldType::String, true, "Commit message"),
        field("files", FieldType::Array, false, "Files to stage (if empty, commits already-staged files)"),
    ],
    outputs = [
        field("hash", FieldType::String, true, "Commit hash"),
        field("message", FieldType::String, true, "Commit message used"),
        field("files_committed", FieldType::Number, true, "Number of files in the commit"),
        field("success", FieldType::Boolean, true, "Whether commit succeeded"),
    ],
    config_fields = [
        field("path", FieldType::String, false, "Repository path (defaults to cwd)"),
    ]
}

#[async_trait]
impl Tool for GitCommitTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let cwd = config
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(".");

        let message = inputs
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "git/commit".into(),
                message: "missing required input: message".into(),
            })?;

        if message.trim().is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "git/commit".into(),
                message: "Commit message cannot be empty".into(),
            });
        }

        // Stage files if provided
        let files = inputs
            .get("files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if !files.is_empty() {
            let file_strs: Vec<String> = files
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();

            let mut add_args: Vec<&str> = vec!["add"];
            for f in &file_strs {
                add_args.push(f.as_str());
            }
            run_git_ok(&add_args, cwd, "git/commit").await?;
        }

        // Check there's something staged
        let staged_out =
            run_git_ok(&["diff", "--cached", "--name-only"], cwd, "git/commit").await?;
        let staged_files: Vec<&str> = staged_out
            .trim()
            .lines()
            .filter(|l| !l.is_empty())
            .collect();

        if staged_files.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "git/commit".into(),
                message: "Nothing to commit -- no staged changes".into(),
            });
        }

        // Create commit (NEVER amend, NEVER skip hooks)
        run_git_ok(&["commit", "-m", message], cwd, "git/commit").await?;

        // Get the commit hash
        let hash_out = run_git_ok(&["rev-parse", "HEAD"], cwd, "git/commit").await?;
        let commit_hash = hash_out.trim().to_string();

        let files_committed = staged_files.len();

        let mut out = HashMap::new();
        out.insert("hash".to_string(), json!(commit_hash));
        out.insert("message".to_string(), json!(message));
        out.insert("files_committed".to_string(), json!(files_committed));
        out.insert("success".to_string(), json!(true));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all git tools into the given registry.
pub fn register_git_tools(registry: &mut ToolRegistry) {
    registry.register("git/status", Box::new(GitStatusFactory::new()));
    registry.register("git/diff", Box::new(GitDiffFactory::new()));
    registry.register("git/log", Box::new(GitLogFactory::new()));
    registry.register("git/commit", Box::new(GitCommitFactory::new()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Registration -------------------------------------------------------

    #[test]
    fn register_git_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_git_tools(&mut reg);
        assert!(reg.get("git/status").is_some());
        assert!(reg.get("git/diff").is_some());
        assert!(reg.get("git/log").is_some());
        assert!(reg.get("git/commit").is_some());
        assert_eq!(reg.list_tools().len(), 4);
    }

    // -- Porcelain parsing ---------------------------------------------------

    #[test]
    fn parse_porcelain_staged() {
        let line = "M  src/main.rs";
        let bytes = line.as_bytes();
        let index = bytes[0] as char;
        assert!("MADRC".contains(index), "Expected staged file with index=M");
    }

    #[test]
    fn parse_porcelain_untracked() {
        let line = "?? new_file.txt";
        let bytes = line.as_bytes();
        let index = bytes[0] as char;
        let worktree = bytes[1] as char;
        assert_eq!(index, '?');
        assert_eq!(worktree, '?');
    }

    #[test]
    fn parse_porcelain_modified_worktree() {
        let line = " M src/lib.rs";
        let bytes = line.as_bytes();
        let worktree = bytes[1] as char;
        assert!("MD".contains(worktree));
    }

    // -- Commit separator parsing -------------------------------------------

    #[test]
    fn parse_log_line() {
        let line =
            format!("abc123{COMMIT_SEP}Author{COMMIT_SEP}2025-01-01{COMMIT_SEP}Initial commit");
        let parts: Vec<&str> = line.splitn(4, COMMIT_SEP).collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "abc123");
        assert_eq!(parts[1], "Author");
        assert_eq!(parts[2], "2025-01-01");
        assert_eq!(parts[3], "Initial commit");
    }

    // -- Stat parsing -------------------------------------------------------

    #[test]
    fn parse_diff_stat_line() {
        let line = " 3 files changed, 10 insertions(+), 5 deletions(-)";
        let mut files_changed: i64 = 0;
        let mut insertions: i64 = 0;
        let mut deletions: i64 = 0;

        if line.contains("file") && line.contains("changed") {
            for part in line.split(',') {
                let part = part.trim();
                if part.contains("file") {
                    if let Some(num) = part.split_whitespace().next() {
                        files_changed = num.parse().unwrap_or(0);
                    }
                } else if part.contains("insertion") {
                    if let Some(num) = part.split_whitespace().next() {
                        insertions = num.parse().unwrap_or(0);
                    }
                } else if part.contains("deletion") {
                    if let Some(num) = part.split_whitespace().next() {
                        deletions = num.parse().unwrap_or(0);
                    }
                }
            }
        }

        assert_eq!(files_changed, 3);
        assert_eq!(insertions, 10);
        assert_eq!(deletions, 5);
    }
}
