use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType};
use crate::tools::registry::{Tool, ToolRegistry};


pub mod copy;
pub mod delete;
pub mod edit_file;
pub mod file_info;
pub mod glob_files;
pub mod grep_files;
pub mod list_dir;
pub mod mkdir;
pub mod move_file;
pub mod read_file;
pub mod tree;
pub mod write_file;

// Re-export tool structs for tests and backward compat.
pub use copy::*;
pub use delete::*;
pub use edit_file::*;
pub use file_info::*;
pub use glob_files::*;
pub use grep_files::*;
pub use list_dir::*;
pub use mkdir::*;
pub use move_file::*;
pub use read_file::*;
pub use tree::*;
pub use write_file::*;

pub fn register_filesystem_tools(registry: &mut ToolRegistry) {
    // Canonical names (match Python: filesystem/*)
    registry.register("filesystem/read_file", Box::new(ReadFileFactory::new()));
    registry.register("filesystem/write_file", Box::new(WriteFileFactory::new()));
    registry.register("filesystem/list_dir", Box::new(ListDirFactory::new()));
    registry.register("filesystem/glob_files", Box::new(GlobFilesFactory::new()));
    registry.register("filesystem/grep_files", Box::new(GrepFilesFactory::new()));
    registry.register("filesystem/edit_file", Box::new(EditFileFactory::new()));
    registry.register("filesystem/copy", Box::new(CopyFactory::new()));
    registry.register("filesystem/move", Box::new(MoveFactory::new()));
    registry.register("filesystem/delete", Box::new(DeleteFactory::new()));
    registry.register("filesystem/mkdir", Box::new(MkdirFactory::new()));
    registry.register("filesystem/tree", Box::new(TreeFactory::new()));
    registry.register("filesystem/file_info", Box::new(FileInfoFactory::new()));

    // Legacy aliases (fs/* → filesystem/*) — remove in v0.3.0
    registry.register_alias("fs/read_file", "filesystem/read_file");
    registry.register_alias("fs/write_file", "filesystem/write_file");
    registry.register_alias("fs/list_dir", "filesystem/list_dir");
    registry.register_alias("fs/glob_files", "filesystem/glob_files");
    registry.register_alias("fs/grep_files", "filesystem/grep_files");
    registry.register_alias("fs/edit_file", "filesystem/edit_file");
    registry.register_alias("fs/copy", "filesystem/copy");
    registry.register_alias("fs/move", "filesystem/move");
    registry.register_alias("fs/delete", "filesystem/delete");
    registry.register_alias("fs/mkdir", "filesystem/mkdir");
    registry.register_alias("fs/tree", "filesystem/tree");
    registry.register_alias("fs/file_info", "filesystem/file_info");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::InMemoryContext;
    use std::io::Write;
    use tempfile::TempDir;

    fn ctx() -> InMemoryContext {
        InMemoryContext::new("test-run")
    }

    // -- ReadFileTool -------------------------------------------------------

    #[tokio::test]
    async fn read_file_basic() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("hello.txt");
        {
            let mut f = fs::File::create(&file_path).unwrap();
            writeln!(f, "line one").unwrap();
            writeln!(f, "line two").unwrap();
            writeln!(f, "line three").unwrap();
        }

        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        let config = HashMap::new();
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["lines"], json!(3));
        let content = result["content"].as_str().unwrap();
        assert!(content.contains("line one"));
        assert!(content.contains("line three"));
    }

    #[tokio::test]
    async fn read_file_with_offset_limit() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("nums.txt");
        {
            let mut f = fs::File::create(&file_path).unwrap();
            for i in 1..=10 {
                writeln!(f, "line {}", i).unwrap();
            }
        }

        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        let mut config = HashMap::new();
        config.insert("offset".to_string(), json!(2));
        config.insert("limit".to_string(), json!(3));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        let content = result["content"].as_str().unwrap();
        assert!(content.contains("line 3"));
        assert!(content.contains("line 5"));
        assert!(!content.contains("line 1\t") || !content.starts_with("     1\t"));
    }

    #[tokio::test]
    async fn read_file_not_found() {
        let tool = ReadFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("/nonexistent/file.txt"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    // -- WriteFileTool ------------------------------------------------------

    #[tokio::test]
    async fn write_file_creates_new() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new.txt");

        let tool = WriteFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("content".to_string(), json!("hello world"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert_eq!(result["bytes_written"], json!(11));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn write_file_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("a/b/c/deep.txt");

        let tool = WriteFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("content".to_string(), json!("deep"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert!(file_path.exists());
    }

    // -- ListDirTool --------------------------------------------------------

    #[tokio::test]
    async fn list_dir_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        let tool = ListDirTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(dir.path().display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        // Hidden excluded by default
        assert_eq!(result["count"], json!(3));
        let entries = result["entries"].as_array().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"subdir"));
        assert!(!names.contains(&".hidden"));
    }

    #[tokio::test]
    async fn list_dir_show_hidden() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        let tool = ListDirTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(dir.path().display().to_string()));
        let mut config = HashMap::new();
        config.insert("show_hidden".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- GlobFilesTool ------------------------------------------------------

    #[tokio::test]
    async fn glob_files_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("b.rs"), "fn test() {}").unwrap();
        fs::write(dir.path().join("c.txt"), "hello").unwrap();

        let tool = GlobFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("*.rs"));
        let mut config = HashMap::new();
        config.insert("path".to_string(), json!(dir.path().display().to_string()));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- EditFileTool -------------------------------------------------------

    #[tokio::test]
    async fn edit_file_single_replacement() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("edit_me.txt");
        fs::write(&file_path, "hello world").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("old_string".to_string(), json!("hello"));
        inputs.insert("new_string".to_string(), json!("goodbye"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["replacements"], json!(1));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "goodbye world");
    }

    #[tokio::test]
    async fn edit_file_replace_all() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("multi.txt");
        fs::write(&file_path, "aaa bbb aaa").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("old_string".to_string(), json!("aaa"));
        inputs.insert("new_string".to_string(), json!("ccc"));
        let mut config = HashMap::new();
        config.insert("replace_all".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["replacements"], json!(2));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "ccc bbb ccc");
    }

    #[tokio::test]
    async fn edit_file_ambiguous_without_replace_all() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("dup.txt");
        fs::write(&file_path, "aaa bbb aaa").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("old_string".to_string(), json!("aaa"));
        inputs.insert("new_string".to_string(), json!("ccc"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn edit_file_not_found_string() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("nope.txt");
        fs::write(&file_path, "hello world").unwrap();

        let tool = EditFileTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file_path.display().to_string()));
        inputs.insert("old_string".to_string(), json!("xyz"));
        inputs.insert("new_string".to_string(), json!("abc"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await;
        assert!(result.is_err());
    }

    // -- GrepFilesTool ------------------------------------------------------

    #[tokio::test]
    async fn grep_files_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.txt"),
            "hello world\nfoo bar\nhello again",
        )
        .unwrap();
        fs::write(dir.path().join("b.txt"), "nothing here").unwrap();

        let tool = GrepFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("hello"));
        let mut config = HashMap::new();
        config.insert("path".to_string(), json!(dir.path().display().to_string()));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn grep_files_case_insensitive() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "Hello World\nhello world").unwrap();

        let tool = GrepFilesTool;
        let mut inputs = HashMap::new();
        inputs.insert("pattern".to_string(), json!("HELLO"));
        let mut config = HashMap::new();
        config.insert("path".to_string(), json!(dir.path().display().to_string()));
        config.insert("case_insensitive".to_string(), json!(true));
        let result = tool.execute(inputs, &config, &ctx()).await.unwrap();

        assert_eq!(result["count"], json!(2));
    }

    // -- CopyTool -----------------------------------------------------------

    #[tokio::test]
    async fn copy_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        fs::write(&src, "copy me").unwrap();
        let dest = dir.path().join("dest.txt");

        let tool = CopyTool;
        let mut inputs = HashMap::new();
        inputs.insert("source".to_string(), json!(src.display().to_string()));
        inputs.insert("destination".to_string(), json!(dest.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["bytes_copied"], json!(7));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "copy me");
    }

    // -- MoveTool -----------------------------------------------------------

    #[tokio::test]
    async fn move_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("old.txt");
        fs::write(&src, "move me").unwrap();
        let dest = dir.path().join("new.txt");

        let tool = MoveTool;
        let mut inputs = HashMap::new();
        inputs.insert("source".to_string(), json!(src.display().to_string()));
        inputs.insert("destination".to_string(), json!(dest.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "move me");
        assert!(result["destination"].as_str().is_some());
    }

    // -- DeleteTool ---------------------------------------------------------

    #[tokio::test]
    async fn delete_file_basic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("delete_me.txt");
        fs::write(&file, "bye").unwrap();

        let tool = DeleteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["deleted"], json!(true));
        assert!(!file.exists());
    }

    // -- MkdirTool ----------------------------------------------------------

    #[tokio::test]
    async fn mkdir_creates_nested() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a/b/c");

        let tool = MkdirTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(nested.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["created"], json!(true));
        assert!(nested.is_dir());
    }

    // -- TreeTool -----------------------------------------------------------

    #[tokio::test]
    async fn tree_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), "b").unwrap();

        let tool = TreeTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(dir.path().display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert!(result["tree"].as_str().unwrap().contains("a.txt"));
        assert!(result["tree"].as_str().unwrap().contains("sub/"));
        assert_eq!(result["files"], json!(2));
        assert_eq!(result["dirs"], json!(1));
    }

    // -- FileInfoTool -------------------------------------------------------

    #[tokio::test]
    async fn file_info_basic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("info.txt");
        fs::write(&file, "hello").unwrap();

        let tool = FileInfoTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!(file.display().to_string()));
        let result = tool.execute(inputs, &HashMap::new(), &ctx()).await.unwrap();

        assert_eq!(result["size"], json!(5));
        assert_eq!(result["is_file"], json!(true));
        assert_eq!(result["is_dir"], json!(false));
    }

    // -- Registration -------------------------------------------------------

    #[test]
    fn register_filesystem_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_filesystem_tools(&mut reg);
        // Canonical names
        assert!(reg.get("filesystem/read_file").is_some());
        assert!(reg.get("filesystem/write_file").is_some());
        assert!(reg.get("filesystem/list_dir").is_some());
        assert!(reg.get("filesystem/glob_files").is_some());
        assert!(reg.get("filesystem/grep_files").is_some());
        assert!(reg.get("filesystem/edit_file").is_some());
        assert!(reg.get("filesystem/copy").is_some());
        assert!(reg.get("filesystem/move").is_some());
        assert!(reg.get("filesystem/delete").is_some());
        assert!(reg.get("filesystem/mkdir").is_some());
        assert!(reg.get("filesystem/tree").is_some());
        assert!(reg.get("filesystem/file_info").is_some());
        assert_eq!(reg.list_tools().len(), 12);
    }

    #[test]
    fn legacy_fs_aliases_resolve() {
        let mut reg = ToolRegistry::new();
        register_filesystem_tools(&mut reg);
        // Legacy aliases (fs/* → filesystem/*)
        assert!(reg.get("fs/read_file").is_some());
        assert!(reg.get("fs/write_file").is_some());
        assert!(reg.get("fs/list_dir").is_some());
        assert!(reg.get("fs/copy").is_some());
        assert!(reg.get("fs/delete").is_some());
    }
}
