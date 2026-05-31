//! TreeTool

use super::*;

// ===========================================================================
// TreeTool
// ===========================================================================

fs_tool! {
    struct TreeTool, factory TreeFactory;
    tool_type = "filesystem/tree",
    name = "Directory Tree",
    description = "Displays a recursive directory listing with indentation",
    category = "filesystem",
    inputs = [
        field("path", FieldType::String, true, "Root directory path"),
    ],
    outputs = [
        field("tree", FieldType::String, true, "Formatted directory tree"),
        field("files", FieldType::Number, true, "Total file count"),
        field("dirs", FieldType::Number, true, "Total directory count"),
    ],
    config_fields = [
        field("max_depth", FieldType::Number, false, "Maximum depth to traverse (default 5)"),
        field("show_hidden", FieldType::Boolean, false, "Include hidden files"),
    ]
}

#[derive(Clone, Copy)]
struct TreeOpts {
    max_depth: usize,
    show_hidden: bool,
}

fn build_tree(
    dir: &Path,
    prefix: &str,
    depth: usize,
    opts: TreeOpts,
    lines: &mut Vec<String>,
    file_count: &mut usize,
    dir_count: &mut usize,
) {
    if depth > opts.max_depth {
        return;
    }

    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by(|a, b| {
        a.file_name()
            .to_string_lossy()
            .cmp(&b.file_name().to_string_lossy())
    });

    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !opts.show_hidden && name.starts_with('.') {
            continue;
        }
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };

        lines.push(format!("{}{}{}{}", prefix, connector, name, suffix));

        if is_dir {
            *dir_count += 1;
            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            build_tree(
                &entry.path(),
                &child_prefix,
                depth + 1,
                opts,
                lines,
                file_count,
                dir_count,
            );
        } else {
            *file_count += 1;
        }
    }
}

#[async_trait]
impl Tool for TreeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let raw_path = inputs.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "filesystem/tree".into(),
                message: "missing required input: path".into(),
            }
        })?;
        let max_depth = config
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        let show_hidden = config
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = fs::canonicalize(raw_path).map_err(|e| ToolError::ExecutionFailed {
            tool_type: "filesystem/tree".into(),
            message: format!("Path not found: {raw_path} ({e})"),
        })?;

        if !path.is_dir() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "filesystem/tree".into(),
                message: format!("Not a directory: {}", path.display()),
            });
        }

        let mut lines = vec![format!("{}/", path.display())];
        let mut file_count: usize = 0;
        let mut dir_count: usize = 0;
        build_tree(
            &path,
            "",
            0,
            TreeOpts {
                max_depth,
                show_hidden,
            },
            &mut lines,
            &mut file_count,
            &mut dir_count,
        );

        let mut out = HashMap::new();
        out.insert("tree".to_string(), json!(lines.join("\n")));
        out.insert("files".to_string(), json!(file_count));
        out.insert("dirs".to_string(), json!(dir_count));
        Ok(out)
    }
}
